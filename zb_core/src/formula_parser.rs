//! Ruby formula parser for Homebrew tap formulas.
//!
//! Parses a subset of the Ruby DSL used in Homebrew formulas to extract
//! the metadata needed for bottle installation. This is intentionally limited
//! to the parts we need and ignores `install`, `test`, `caveats`, and `service` blocks.
//!
//! # Supported DSL Elements
//!
//! ```ruby
//! class Foo < Formula
//!   desc "Description"
//!   homepage "https://..."
//!   url "https://..."
//!   sha256 "..."
//!   license "MIT"
//!   version "1.2.3"
//!   revision 1
//!
//!   depends_on "dep1"
//!   depends_on "dep2" => :build
//!   uses_from_macos "zlib"
//!   uses_from_macos "flex" => :build
//!
//!   bottle do
//!     rebuild 1
//!     sha256 cellar: :any, arm64_sonoma: "..."
//!     sha256 cellar: :any_skip_relocation, x86_64_linux: "..."
//!   end
//! end
//! ```

use tree_sitter::{Node, Parser};

use crate::formula::{BottleFile, Formula};

/// Error type for formula parsing failures.
#[derive(Debug)]
pub enum ParseError {
    /// Failed to initialize tree-sitter parser.
    ParserInit,
    /// Failed to parse Ruby source code.
    ParseFailed,
    /// Formula class not found in source.
    NoFormulaClass,
    /// Required field is missing.
    MissingField(&'static str),
    /// Invalid field value.
    InvalidValue { field: &'static str, message: String },
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::ParserInit => write!(f, "failed to initialize Ruby parser"),
            ParseError::ParseFailed => write!(f, "failed to parse Ruby source"),
            ParseError::NoFormulaClass => write!(f, "no Formula class found in source"),
            ParseError::MissingField(field) => write!(f, "missing required field: {}", field),
            ParseError::InvalidValue { field, message } => {
                write!(f, "invalid value for {}: {}", field, message)
            }
        }
    }
}

impl std::error::Error for ParseError {}

/// Parses a Ruby formula file and extracts metadata into a `Formula` struct.
///
/// # Arguments
/// * `source` - The Ruby source code of the formula file.
/// * `name` - The formula name (typically derived from the filename).
///
/// # Returns
/// A `Formula` struct populated with the parsed metadata, or a `ParseError`.
///
/// # Example
/// ```ignore
/// let source = r#"
/// class Jq < Formula
///   desc "Lightweight JSON processor"
///   homepage "https://jqlang.github.io/jq/"
///   url "https://github.com/jqlang/jq/archive/refs/tags/jq-1.7.tar.gz"
///   sha256 "abc123..."
///   license "MIT"
///
///   bottle do
///     sha256 cellar: :any, arm64_sonoma: "abc..."
///   end
/// end
/// "#;
/// let formula = parse_ruby_formula(source, "jq")?;
/// ```
pub fn parse_ruby_formula(source: &str, name: &str) -> Result<Formula, ParseError> {
    let mut parser = Parser::new();
    let language = tree_sitter_ruby::LANGUAGE;
    parser
        .set_language(&language.into())
        .map_err(|_| ParseError::ParserInit)?;

    let tree = parser.parse(source, None).ok_or(ParseError::ParseFailed)?;
    let root = tree.root_node();

    // Find the Formula class definition
    let class_node = find_formula_class(&root, source)?;

    // Extract fields from the class body
    let mut formula = Formula {
        name: name.to_string(),
        ..Default::default()
    };

    parse_class_body(&class_node, source, &mut formula)?;

    // Validate required fields
    if formula.versions.stable.is_empty() {
        return Err(ParseError::MissingField("version"));
    }

    Ok(formula)
}

/// Finds the Formula class definition in the AST.
fn find_formula_class<'a>(root: &'a Node, source: &str) -> Result<Node<'a>, ParseError> {
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        if child.kind() == "class" {
            // Check if it inherits from Formula
            // The superclass node contains "< Formula", we need to find the constant inside
            if let Some(superclass) = child.child_by_field_name("superclass") {
                // Find the constant child within superclass
                let mut sc_cursor = superclass.walk();
                for sc_child in superclass.children(&mut sc_cursor) {
                    if sc_child.kind() == "constant" {
                        let name = get_node_text(&sc_child, source);
                        if name == "Formula" {
                            return Ok(child);
                        }
                    }
                }
            }
        }
    }

    Err(ParseError::NoFormulaClass)
}

/// Parses the body of a Formula class and extracts metadata.
fn parse_class_body(class_node: &Node, source: &str, formula: &mut Formula) -> Result<(), ParseError> {
    let Some(body) = class_node.child_by_field_name("body") else {
        return Ok(());
    };

    let mut cursor = body.walk();
    let mut revision: u32 = 0;

    for child in body.children(&mut cursor) {
        match child.kind() {
            "call" | "method_call" => {
                parse_method_call(&child, source, formula, &mut revision)?;
            }
            "do_block" | "block" => {
                // Handle blocks that might be at class level
            }
            _ => {}
        }
    }

    // Apply revision to version if present
    if revision > 0 && !formula.versions.stable.is_empty() {
        // Revision is stored separately from version in our data model
        // but we track it for the effective_version calculation
    }

    Ok(())
}

/// Parses a method call and extracts relevant metadata.
fn parse_method_call(
    node: &Node,
    source: &str,
    formula: &mut Formula,
    revision: &mut u32,
) -> Result<(), ParseError> {
    // Get the method name
    let method_name = if let Some(method_node) = node.child_by_field_name("method") {
        get_node_text(&method_node, source)
    } else if let Some(first_child) = node.child(0) {
        get_node_text(&first_child, source)
    } else {
        return Ok(());
    };

    match method_name.as_str() {
        "desc" => {
            formula.desc = extract_string_arg(node, source);
        }
        "homepage" => {
            formula.homepage = extract_string_arg(node, source);
        }
        "license" => {
            formula.license = extract_string_arg(node, source);
        }
        "version" => {
            if let Some(v) = extract_string_arg(node, source) {
                formula.versions.stable = v;
            }
        }
        "url" => {
            // Extract version from URL if not explicitly set
            if formula.versions.stable.is_empty() {
                if let Some(url) = extract_string_arg(node, source) {
                    if let Some(v) = extract_version_from_url(&url) {
                        formula.versions.stable = v;
                    }
                }
            }
        }
        "revision" => {
            if let Some(rev) = extract_integer_arg(node, source) {
                *revision = rev as u32;
            }
        }
        "depends_on" => {
            parse_depends_on(node, source, formula);
        }
        "uses_from_macos" => {
            parse_uses_from_macos(node, source, formula);
        }
        "bottle" => {
            parse_bottle_block(node, source, formula)?;
        }
        _ => {}
    }

    Ok(())
}

/// Extracts a string argument from a method call.
fn extract_string_arg(node: &Node, source: &str) -> Option<String> {
    // Find the arguments node
    let args = node.child_by_field_name("arguments")?;

    let mut cursor = args.walk();
    for child in args.children(&mut cursor) {
        if let Some(s) = extract_string_value(&child, source) {
            return Some(s);
        }
    }

    None
}

/// Extracts a string value from various string node types.
fn extract_string_value(node: &Node, source: &str) -> Option<String> {
    match node.kind() {
        "string" | "string_content" => {
            // Find the string_content child
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "string_content" {
                    return Some(get_node_text(&child, source));
                }
            }
            // If no string_content, try the node itself (minus quotes)
            let text = get_node_text(node, source);
            Some(text.trim_matches('"').trim_matches('\'').to_string())
        }
        "bare_string" => Some(get_node_text(node, source)),
        _ => None,
    }
}

/// Extracts an integer argument from a method call.
fn extract_integer_arg(node: &Node, source: &str) -> Option<i64> {
    let args = node.child_by_field_name("arguments")?;

    let mut cursor = args.walk();
    for child in args.children(&mut cursor) {
        if child.kind() == "integer" {
            let text = get_node_text(&child, source);
            return text.parse().ok();
        }
    }

    None
}

/// Parses a depends_on declaration.
fn parse_depends_on(node: &Node, source: &str, formula: &mut Formula) {
    // depends_on "name" or depends_on "name" => :build
    let Some(args) = node.child_by_field_name("arguments") else {
        return;
    };

    let mut cursor = args.walk();
    let mut dep_name: Option<String> = None;
    let mut is_build_only = false;

    for child in args.children(&mut cursor) {
        match child.kind() {
            "string" | "bare_string" => {
                dep_name = extract_string_value(&child, source);
            }
            "pair" | "hash" => {
                // Check if this is a build dependency: "name" => :build
                if let Some((name, dep_type)) = parse_dependency_pair(&child, source) {
                    dep_name = Some(name);
                    is_build_only = matches!(dep_type.as_str(), "build" | "test");
                }
            }
            "argument_list" => {
                // Recurse into argument list
                let mut inner_cursor = child.walk();
                for inner_child in child.children(&mut inner_cursor) {
                    if let Some(s) = extract_string_value(&inner_child, source) {
                        dep_name = Some(s);
                    }
                    if inner_child.kind() == "pair" {
                        if let Some((name, dep_type)) = parse_dependency_pair(&inner_child, source) {
                            dep_name = Some(name);
                            is_build_only = matches!(dep_type.as_str(), "build" | "test");
                        }
                    }
                }
            }
            _ => {}
        }
    }

    if let Some(name) = dep_name {
        if is_build_only {
            if !formula.build_dependencies.contains(&name) {
                formula.build_dependencies.push(name);
            }
        } else if !formula.dependencies.contains(&name) {
            formula.dependencies.push(name);
        }
    }
}

/// Parses a dependency pair like "name" => :build.
fn parse_dependency_pair(node: &Node, source: &str) -> Option<(String, String)> {
    let key = node.child_by_field_name("key")?;
    let value = node.child_by_field_name("value")?;

    let name = extract_string_value(&key, source)?;
    let dep_type = get_node_text(&value, source).trim_start_matches(':').to_string();

    Some((name, dep_type))
}

/// Parses a uses_from_macos declaration.
fn parse_uses_from_macos(node: &Node, source: &str, formula: &mut Formula) {
    // Similar to depends_on, but only runtime deps go into uses_from_macos
    let Some(args) = node.child_by_field_name("arguments") else {
        return;
    };

    let mut cursor = args.walk();
    let mut dep_name: Option<String> = None;
    let mut is_runtime = true;

    for child in args.children(&mut cursor) {
        match child.kind() {
            "string" | "bare_string" => {
                dep_name = extract_string_value(&child, source);
            }
            "pair" | "hash" => {
                // Check for build/test only markers
                if let Some((name, dep_type)) = parse_dependency_pair(&child, source) {
                    dep_name = Some(name);
                    is_runtime = !matches!(dep_type.as_str(), "build" | "test");
                }
            }
            "argument_list" => {
                let mut inner_cursor = child.walk();
                for inner_child in child.children(&mut inner_cursor) {
                    if let Some(s) = extract_string_value(&inner_child, source) {
                        dep_name = Some(s);
                    }
                    if inner_child.kind() == "pair" {
                        if let Some((name, dep_type)) = parse_dependency_pair(&inner_child, source) {
                            dep_name = Some(name);
                            is_runtime = !matches!(dep_type.as_str(), "build" | "test");
                        }
                    }
                }
            }
            _ => {}
        }
    }

    if let Some(name) = dep_name {
        if is_runtime && !formula.uses_from_macos.contains(&name) {
            formula.uses_from_macos.push(name);
        }
    }
}

/// Parses a bottle block.
fn parse_bottle_block(node: &Node, source: &str, formula: &mut Formula) -> Result<(), ParseError> {
    // Find the do_block
    let block = find_child_by_kind(node, "do_block")
        .or_else(|| find_child_by_kind(node, "block"));

    let Some(block) = block else {
        return Ok(());
    };

    // Find the body of the block
    let body = block.child_by_field_name("body")
        .or_else(|| find_child_by_kind(&block, "body_statement"));

    let Some(body) = body else {
        return Ok(());
    };

    let mut cursor = body.walk();

    for child in body.children(&mut cursor) {
        if child.kind() == "call" || child.kind() == "method_call" {
            parse_bottle_statement(&child, source, formula)?;
        }
    }

    Ok(())
}

/// Parses a statement inside a bottle block.
fn parse_bottle_statement(node: &Node, source: &str, formula: &mut Formula) -> Result<(), ParseError> {
    let method_name = if let Some(method_node) = node.child_by_field_name("method") {
        get_node_text(&method_node, source)
    } else if let Some(first_child) = node.child(0) {
        get_node_text(&first_child, source)
    } else {
        return Ok(());
    };

    match method_name.as_str() {
        "rebuild" => {
            if let Some(r) = extract_integer_arg(node, source) {
                formula.bottle.stable.rebuild = r as u32;
            }
        }
        "sha256" => {
            parse_bottle_sha256(node, source, formula)?;
        }
        _ => {}
    }

    Ok(())
}

/// Parses a sha256 line in a bottle block.
/// Format: sha256 cellar: :any, arm64_sonoma: "hash..."
/// Or: sha256 arm64_sonoma: "hash..."
fn parse_bottle_sha256(node: &Node, source: &str, formula: &mut Formula) -> Result<(), ParseError> {
    let Some(args) = node.child_by_field_name("arguments") else {
        return Ok(());
    };

    // Parse all key-value pairs
    let mut platform: Option<String> = None;
    let mut sha256: Option<String> = None;

    let mut cursor = args.walk();
    for child in args.children(&mut cursor) {
        match child.kind() {
            "pair" => {
                let key = child.child_by_field_name("key");
                let value = child.child_by_field_name("value");

                if let (Some(k), Some(v)) = (key, value) {
                    let key_text = get_node_text(&k, source)
                        .trim_start_matches(':')
                        .to_string();

                    // Skip cellar: and root_url:, we care about platform: sha256
                    if key_text != "cellar" && key_text != "root_url" {
                        platform = Some(key_text);
                        sha256 = extract_string_value(&v, source);
                    }
                }
            }
            "hash" | "argument_list" => {
                // Parse hash entries
                let mut inner_cursor = child.walk();
                for pair in child.children(&mut inner_cursor) {
                    if pair.kind() == "pair" {
                        let key = pair.child_by_field_name("key");
                        let value = pair.child_by_field_name("value");

                        if let (Some(k), Some(v)) = (key, value) {
                            let key_text = get_node_text(&k, source)
                                .trim_start_matches(':')
                                .to_string();

                            if key_text != "cellar" && key_text != "root_url" {
                                platform = Some(key_text);
                                sha256 = extract_string_value(&v, source);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    if let (Some(plat), Some(hash)) = (platform, sha256) {
        // Convert platform name to match Homebrew API format
        let platform_key = normalize_platform_name(&plat);

        // Generate bottle URL (Homebrew's bottle URL format)
        let url = format!(
            "https://ghcr.io/v2/homebrew/core/{}/blobs/sha256:{}",
            formula.name, hash
        );

        formula.bottle.stable.files.insert(
            platform_key,
            BottleFile { url, sha256: hash },
        );
    }

    Ok(())
}

/// Normalizes platform names to match Homebrew API format.
fn normalize_platform_name(name: &str) -> String {
    // Platform names in Ruby formulas are symbols like :arm64_sonoma
    // They need to match the keys in the API JSON
    name.to_string()
}

/// Extracts version from a URL using common patterns.
fn extract_version_from_url(url: &str) -> Option<String> {
    // Common patterns:
    // - archive/refs/tags/v1.2.3.tar.gz
    // - releases/download/v1.2.3/...
    // - package-1.2.3.tar.gz
    // - Package-1.2.3.tgz
    // - jq-1.7.1.tar.gz

    // Pattern with lookahead to avoid capturing file extension
    let version_regex = regex::Regex::new(
        r"[-_/]v?(\d+\.\d+(?:\.\d+)?)(?:[-_.](?:tar|zip|gz|tgz|xz|bz2)|/|$)"
    ).ok()?;

    version_regex
        .captures(url)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

/// Gets the text content of a node.
fn get_node_text(node: &Node, source: &str) -> String {
    let start = node.start_byte();
    let end = node.end_byte();
    source[start..end].to_string()
}

/// Finds a child node by kind.
fn find_child_by_kind<'a>(node: &'a Node, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return Some(child);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_formula() {
        let source = r#"
class Jq < Formula
  desc "Lightweight and flexible command-line JSON processor"
  homepage "https://jqlang.github.io/jq/"
  url "https://github.com/jqlang/jq/releases/download/jq-1.7.1/jq-1.7.1.tar.gz"
  sha256 "2be64e7129cecb11d5906290eba10af694fb9e3e7f9fc208a311dc33ca837eb0"
  license "MIT"

  bottle do
    sha256 cellar: :any, arm64_sonoma: "abc123def456"
    sha256 cellar: :any, x86_64_linux: "789xyz000111"
  end

  depends_on "oniguruma"

  def install
    system "./configure", *std_configure_args
    system "make", "install"
  end
end
"#;

        let formula = parse_ruby_formula(source, "jq").unwrap();

        assert_eq!(formula.name, "jq");
        assert_eq!(formula.desc.as_deref(), Some("Lightweight and flexible command-line JSON processor"));
        assert_eq!(formula.homepage.as_deref(), Some("https://jqlang.github.io/jq/"));
        assert_eq!(formula.license.as_deref(), Some("MIT"));
        assert_eq!(formula.versions.stable, "1.7.1");
        assert_eq!(formula.dependencies, vec!["oniguruma"]);
        assert!(formula.bottle.stable.files.contains_key("arm64_sonoma"));
        assert!(formula.bottle.stable.files.contains_key("x86_64_linux"));
    }

    #[test]
    fn parse_formula_with_build_deps() {
        let source = r#"
class Ripgrep < Formula
  desc "Search tool like grep"
  homepage "https://github.com/BurntSushi/ripgrep"
  url "https://github.com/BurntSushi/ripgrep/archive/refs/tags/15.1.0.tar.gz"
  sha256 "046fa01a216793b8bd2750f9d68d4ad43986eb9c0d6122600f993906012972e8"
  license "Unlicense"

  bottle do
    sha256 cellar: :any, arm64_sonoma: "f4dc761b07edb8e6438b618d22f7e57252903e2f2b973e2c7aa0da518fc374b9"
  end

  depends_on "rust" => :build
  depends_on "pcre2"

  def install
  end
end
"#;

        let formula = parse_ruby_formula(source, "ripgrep").unwrap();

        assert_eq!(formula.name, "ripgrep");
        assert_eq!(formula.versions.stable, "15.1.0");
        assert!(formula.build_dependencies.contains(&"rust".to_string()));
        assert!(formula.dependencies.contains(&"pcre2".to_string()));
        assert!(!formula.dependencies.contains(&"rust".to_string()));
    }

    #[test]
    fn parse_formula_with_uses_from_macos() {
        let source = r#"
class Git < Formula
  desc "Distributed revision control system"
  homepage "https://git-scm.com"
  url "https://github.com/git/git/archive/refs/tags/v2.43.0.tar.gz"
  sha256 "abc123"
  license "GPL-2.0-only"

  bottle do
    sha256 arm64_sonoma: "def456"
  end

  depends_on "gettext"
  depends_on "pcre2"

  uses_from_macos "curl"
  uses_from_macos "expat"
  uses_from_macos "zlib"

  def install
  end
end
"#;

        let formula = parse_ruby_formula(source, "git").unwrap();

        assert_eq!(formula.name, "git");
        assert_eq!(formula.dependencies, vec!["gettext", "pcre2"]);
        assert_eq!(formula.uses_from_macos, vec!["curl", "expat", "zlib"]);
    }

    #[test]
    fn parse_formula_with_rebuild() {
        let source = r#"
class Foo < Formula
  desc "A test formula"
  homepage "https://example.com"
  url "https://example.com/foo-1.0.0.tar.gz"
  sha256 "abc123"
  license "MIT"

  bottle do
    rebuild 2
    sha256 arm64_sonoma: "def456"
  end

  def install
  end
end
"#;

        let formula = parse_ruby_formula(source, "foo").unwrap();

        assert_eq!(formula.bottle.stable.rebuild, 2);
        assert_eq!(formula.effective_version(), "1.0.0_2");
    }

    #[test]
    fn parse_formula_with_explicit_version() {
        let source = r#"
class Foo < Formula
  desc "A test formula"
  homepage "https://example.com"
  url "https://example.com/foo-source.tar.gz"
  sha256 "abc123"
  license "MIT"
  version "2.5.0"

  bottle do
    sha256 arm64_sonoma: "def456"
  end

  def install
  end
end
"#;

        let formula = parse_ruby_formula(source, "foo").unwrap();

        // Explicit version should override URL-derived version
        assert_eq!(formula.versions.stable, "2.5.0");
    }

    #[test]
    fn parse_formula_missing_class_fails() {
        let source = r#"
# Just some Ruby code
def foo
  puts "hello"
end
"#;

        let result = parse_ruby_formula(source, "foo");
        assert!(matches!(result, Err(ParseError::NoFormulaClass)));
    }

    #[test]
    fn extract_version_from_url_works() {
        assert_eq!(
            extract_version_from_url("https://example.com/foo-1.2.3.tar.gz"),
            Some("1.2.3".to_string())
        );
        assert_eq!(
            extract_version_from_url("https://github.com/foo/bar/archive/refs/tags/v2.0.0.tar.gz"),
            Some("2.0.0".to_string())
        );
        assert_eq!(
            extract_version_from_url("https://github.com/foo/bar/releases/download/v1.0/bar.tar.gz"),
            Some("1.0".to_string())
        );
    }

    #[test]
    fn parse_uses_from_macos_build_only_excluded() {
        let source = r#"
class Foo < Formula
  desc "Test"
  homepage "https://example.com"
  url "https://example.com/foo-1.0.0.tar.gz"
  sha256 "abc123"
  license "MIT"

  bottle do
    sha256 arm64_sonoma: "def456"
  end

  uses_from_macos "flex" => :build
  uses_from_macos "zlib"

  def install
  end
end
"#;

        let formula = parse_ruby_formula(source, "foo").unwrap();

        // Only runtime uses_from_macos should be included
        assert_eq!(formula.uses_from_macos, vec!["zlib"]);
        assert!(!formula.uses_from_macos.contains(&"flex".to_string()));
    }

    #[test]
    fn parse_multiple_bottle_platforms() {
        let source = r#"
class Foo < Formula
  desc "Test formula"
  homepage "https://example.com"
  url "https://example.com/foo-1.0.0.tar.gz"
  sha256 "abc123"
  license "MIT"

  bottle do
    sha256 cellar: :any, arm64_tahoe: "aaa111"
    sha256 cellar: :any, arm64_sequoia: "bbb222"
    sha256 cellar: :any, arm64_sonoma: "ccc333"
    sha256 cellar: :any, sonoma: "ddd444"
    sha256 cellar: :any_skip_relocation, arm64_linux: "eee555"
    sha256 cellar: :any_skip_relocation, x86_64_linux: "fff666"
  end

  def install
  end
end
"#;

        let formula = parse_ruby_formula(source, "foo").unwrap();

        assert_eq!(formula.bottle.stable.files.len(), 6);
        assert!(formula.bottle.stable.files.contains_key("arm64_tahoe"));
        assert!(formula.bottle.stable.files.contains_key("arm64_sequoia"));
        assert!(formula.bottle.stable.files.contains_key("arm64_sonoma"));
        assert!(formula.bottle.stable.files.contains_key("sonoma"));
        assert!(formula.bottle.stable.files.contains_key("arm64_linux"));
        assert!(formula.bottle.stable.files.contains_key("x86_64_linux"));

        // Verify sha256 values are correct
        assert_eq!(
            formula.bottle.stable.files.get("arm64_sonoma").unwrap().sha256,
            "ccc333"
        );
        assert_eq!(
            formula.bottle.stable.files.get("x86_64_linux").unwrap().sha256,
            "fff666"
        );
    }

    #[test]
    fn parse_formula_with_multiple_deps() {
        let source = r#"
class Python < Formula
  desc "Python interpreter"
  homepage "https://www.python.org/"
  url "https://www.python.org/ftp/python/3.12.0/Python-3.12.0.tgz"
  sha256 "abc123"
  license "Python-2.0"

  bottle do
    sha256 arm64_sonoma: "def456"
  end

  depends_on "pkgconf" => :build
  depends_on "mpdecimal"
  depends_on "openssl@3"
  depends_on "sqlite"
  depends_on "xz"

  uses_from_macos "bzip2"
  uses_from_macos "expat"
  uses_from_macos "libffi"
  uses_from_macos "ncurses"
  uses_from_macos "zlib"

  def install
  end
end
"#;

        let formula = parse_ruby_formula(source, "python").unwrap();

        assert_eq!(formula.versions.stable, "3.12.0");
        assert_eq!(formula.build_dependencies, vec!["pkgconf"]);
        assert_eq!(
            formula.dependencies,
            vec!["mpdecimal", "openssl@3", "sqlite", "xz"]
        );
        assert_eq!(
            formula.uses_from_macos,
            vec!["bzip2", "expat", "libffi", "ncurses", "zlib"]
        );
    }

    #[test]
    fn parse_formula_no_bottle_is_ok() {
        let source = r#"
class Foo < Formula
  desc "Test"
  homepage "https://example.com"
  url "https://example.com/foo-1.0.0.tar.gz"
  sha256 "abc123"
  license "MIT"
  version "1.0.0"

  def install
  end
end
"#;

        let formula = parse_ruby_formula(source, "foo").unwrap();

        assert_eq!(formula.name, "foo");
        assert_eq!(formula.versions.stable, "1.0.0");
        assert!(formula.bottle.stable.files.is_empty());
    }

    #[test]
    fn bottle_url_contains_formula_name() {
        let source = r#"
class MyTool < Formula
  desc "Test"
  homepage "https://example.com"
  url "https://example.com/mytool-1.0.0.tar.gz"
  sha256 "abc123"
  license "MIT"

  bottle do
    sha256 arm64_sonoma: "def456789abc"
  end

  def install
  end
end
"#;

        let formula = parse_ruby_formula(source, "mytool").unwrap();

        let bottle = formula.bottle.stable.files.get("arm64_sonoma").unwrap();
        assert!(bottle.url.contains("mytool"), "URL should contain formula name");
        assert!(bottle.url.contains("def456789abc"), "URL should contain sha256");
    }

    #[test]
    fn version_extraction_handles_jq_style() {
        // jq uses version in the URL like jq-1.7.1
        assert_eq!(
            extract_version_from_url("https://github.com/jqlang/jq/releases/download/jq-1.7.1/jq-1.7.1.tar.gz"),
            Some("1.7.1".to_string())
        );
    }

    #[test]
    fn version_extraction_handles_python_style() {
        // Python uses Python-3.12.0.tgz
        assert_eq!(
            extract_version_from_url("https://www.python.org/ftp/python/3.12.0/Python-3.12.0.tgz"),
            Some("3.12.0".to_string())
        );
    }
}
