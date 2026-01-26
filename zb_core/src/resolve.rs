//! Dependency resolution using topological sort.
//!
//! This module implements Kahn's algorithm to order package dependencies so that
//! dependencies are always installed before the packages that depend on them.
//!
//! # Algorithm Overview
//!
//! 1. **Closure computation**: Find all transitive dependencies of the root package
//! 2. **Graph construction**: Build a directed graph where edges point from
//!    dependencies to dependents
//! 3. **Topological sort**: Process packages with no remaining dependencies first,
//!    removing them from the graph until all packages are ordered
//! 4. **Cycle detection**: If not all packages can be ordered, a dependency cycle exists
//!
//! # Determinism
//!
//! The output order is deterministic: packages at the same dependency level are
//! sorted alphabetically using `BTreeSet` for stable iteration order.

use crate::{Error, Formula};
use std::collections::{BTreeMap, BTreeSet};

/// Map from package name to number of unprocessed dependencies
type InDegreeMap = BTreeMap<String, usize>;

/// Map from package name to set of packages that depend on it
type AdjacencyMap = BTreeMap<String, BTreeSet<String>>;

/// Resolve the transitive dependency closure for a package and return in install order.
///
/// Uses Kahn's algorithm for topological sorting, which naturally handles cycles
/// by detecting when not all packages can be processed.
///
/// # Returns
/// A vector of package names in installation order (dependencies before dependents).
///
/// # Errors
/// - `MissingFormula` if the root package is not found
/// - `DependencyCycle` if a circular dependency is detected
pub fn resolve_closure(
    root: &str,
    formulas: &BTreeMap<String, Formula>,
) -> Result<Vec<String>, Error> {
    let closure = compute_closure(root, formulas)?;
    let (mut indegree, adjacency) = build_graph(&closure, formulas)?;

    let mut ready: BTreeSet<String> = indegree
        .iter()
        .filter_map(|(name, count)| {
            if *count == 0 {
                Some(name.clone())
            } else {
                None
            }
        })
        .collect();

    let mut ordered = Vec::with_capacity(closure.len());
    while let Some(name) = ready.iter().next().cloned() {
        ready.take(&name);
        ordered.push(name.clone());
        if let Some(children) = adjacency.get(&name) {
            for child in children {
                if let Some(count) = indegree.get_mut(child) {
                    *count -= 1;
                    if *count == 0 {
                        ready.insert(child.clone());
                    }
                }
            }
        }
    }

    if ordered.len() != closure.len() {
        let cycle: Vec<String> = indegree
            .into_iter()
            .filter_map(|(name, count)| if count > 0 { Some(name) } else { None })
            .collect();
        return Err(Error::DependencyCycle { cycle });
    }

    Ok(ordered)
}

/// Compute the transitive closure of dependencies for a package.
///
/// Uses depth-first traversal starting from the root package.
/// Missing dependencies (e.g., `uses_from_macos` packages without Homebrew formulas)
/// are skipped with a warning, but a missing root package is an error.
fn compute_closure(
    root: &str,
    formulas: &BTreeMap<String, Formula>,
) -> Result<BTreeSet<String>, Error> {
    let mut closure = BTreeSet::new();
    let mut stack = vec![root.to_string()];

    while let Some(name) = stack.pop() {
        if !closure.insert(name.clone()) {
            continue;
        }

        // Root package must exist; dependencies can be skipped if missing
        // (e.g., uses_from_macos deps that don't have Homebrew formulas)
        let Some(formula) = formulas.get(&name) else {
            if name == root {
                return Err(Error::MissingFormula { name });
            }
            // Skip missing dependency - remove from closure since we can't process it
            eprintln!("    Note: skipping unavailable dependency '{}'", name);
            closure.remove(&name);
            continue;
        };

        // Use effective_dependencies() to include uses_from_macos on Linux
        let mut deps = formula.effective_dependencies();
        deps.sort();
        for dep in deps {
            if !closure.contains(&dep) {
                stack.push(dep);
            }
        }
    }

    Ok(closure)
}

/// Build the dependency graph for topological sorting.
///
/// Returns two maps:
/// - `indegree`: Count of unprocessed dependencies for each package
/// - `adjacency`: Reverse edges (dependency -> dependents) for decrementing indegrees
///
/// Packages with indegree=0 have no unprocessed dependencies and can be installed.
fn build_graph(
    closure: &BTreeSet<String>,
    formulas: &BTreeMap<String, Formula>,
) -> Result<(InDegreeMap, AdjacencyMap), Error> {
    let mut indegree: InDegreeMap = closure.iter().map(|name| (name.clone(), 0)).collect();
    let mut adjacency: AdjacencyMap = BTreeMap::new();

    for name in closure {
        // Skip formulas not in the map (shouldn't happen after compute_closure filtering)
        let Some(formula) = formulas.get(name) else {
            continue;
        };
        // Use effective_dependencies() to include uses_from_macos on Linux
        let mut deps = formula.effective_dependencies();
        deps.sort();
        for dep in deps {
            if !closure.contains(&dep) {
                continue;
            }
            if let Some(count) = indegree.get_mut(name) {
                *count += 1;
            }
            adjacency.entry(dep).or_default().insert(name.clone());
        }
    }

    Ok((indegree, adjacency))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formula::{Bottle, BottleFile, BottleStable, Versions};
    use std::collections::BTreeMap;

    fn formula(name: &str, deps: &[&str]) -> Formula {
        formula_with_macos_deps(name, deps, &[])
    }

    fn formula_with_macos_deps(name: &str, deps: &[&str], macos_deps: &[&str]) -> Formula {
        let mut files = BTreeMap::new();
        files.insert(
            "arm64_sonoma".to_string(),
            BottleFile {
                url: format!("https://example.com/{name}.tar.gz"),
                sha256: "deadbeef".repeat(8),
            },
        );

        Formula {
            name: name.to_string(),
            versions: Versions {
                stable: "1.0.0".to_string(),
            },
            dependencies: deps.iter().map(|dep| dep.to_string()).collect(),
            uses_from_macos: macos_deps.iter().map(|dep| dep.to_string()).collect(),
            bottle: Bottle {
                stable: BottleStable { files, rebuild: 0 },
            },
            ..Default::default()
        }
    }

    #[test]
    fn resolves_transitive_closure_in_stable_order() {
        let mut formulas = BTreeMap::new();
        formulas.insert("foo".to_string(), formula("foo", &["baz", "bar"]));
        formulas.insert("bar".to_string(), formula("bar", &["qux"]));
        formulas.insert("baz".to_string(), formula("baz", &["qux"]));
        formulas.insert("qux".to_string(), formula("qux", &[]));

        let order = resolve_closure("foo", &formulas).unwrap();
        assert_eq!(order, vec!["qux", "bar", "baz", "foo"]);
    }

    #[test]
    fn detects_cycles() {
        let mut formulas = BTreeMap::new();
        formulas.insert("alpha".to_string(), formula("alpha", &["beta"]));
        formulas.insert("beta".to_string(), formula("beta", &["gamma"]));
        formulas.insert("gamma".to_string(), formula("gamma", &["alpha"]));

        let err = resolve_closure("alpha", &formulas).unwrap_err();
        assert!(matches!(err, Error::DependencyCycle { .. }));
    }
}
