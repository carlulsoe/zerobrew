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
    use proptest::prelude::*;
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

    #[test]
    fn resolves_single_package_with_no_deps() {
        let mut formulas = BTreeMap::new();
        formulas.insert("standalone".to_string(), formula("standalone", &[]));

        let order = resolve_closure("standalone", &formulas).unwrap();
        assert_eq!(order, vec!["standalone"]);
    }

    #[test]
    fn returns_error_for_missing_root() {
        let formulas: BTreeMap<String, Formula> = BTreeMap::new();

        let err = resolve_closure("nonexistent", &formulas).unwrap_err();
        assert!(matches!(err, Error::MissingFormula { name } if name == "nonexistent"));
    }

    #[test]
    fn skips_missing_dependencies_gracefully() {
        let mut formulas = BTreeMap::new();
        // 'foo' depends on 'bar' (exists) and 'missing' (doesn't exist)
        formulas.insert("foo".to_string(), formula("foo", &["bar", "missing"]));
        formulas.insert("bar".to_string(), formula("bar", &[]));

        // Should succeed, skipping 'missing'
        let order = resolve_closure("foo", &formulas).unwrap();
        assert_eq!(order, vec!["bar", "foo"]);
    }

    #[test]
    fn handles_diamond_dependency() {
        // Diamond: root -> [a, b], a -> c, b -> c
        let mut formulas = BTreeMap::new();
        formulas.insert("root".to_string(), formula("root", &["a", "b"]));
        formulas.insert("a".to_string(), formula("a", &["c"]));
        formulas.insert("b".to_string(), formula("b", &["c"]));
        formulas.insert("c".to_string(), formula("c", &[]));

        let order = resolve_closure("root", &formulas).unwrap();
        // 'c' must come first, then 'a' and 'b' (alphabetical), then 'root'
        assert_eq!(order, vec!["c", "a", "b", "root"]);
    }

    #[test]
    fn detects_self_referential_cycle() {
        let mut formulas = BTreeMap::new();
        formulas.insert("selfref".to_string(), formula("selfref", &["selfref"]));

        let err = resolve_closure("selfref", &formulas).unwrap_err();
        assert!(matches!(err, Error::DependencyCycle { .. }));
    }

    #[test]
    fn order_is_deterministic_across_runs() {
        let mut formulas = BTreeMap::new();
        formulas.insert("pkg".to_string(), formula("pkg", &["z", "a", "m"]));
        formulas.insert("z".to_string(), formula("z", &[]));
        formulas.insert("a".to_string(), formula("a", &[]));
        formulas.insert("m".to_string(), formula("m", &[]));

        // Run multiple times and verify same order
        let order1 = resolve_closure("pkg", &formulas).unwrap();
        let order2 = resolve_closure("pkg", &formulas).unwrap();
        let order3 = resolve_closure("pkg", &formulas).unwrap();

        assert_eq!(order1, order2);
        assert_eq!(order2, order3);
        // Alphabetical order for same-level deps: a, m, z
        assert_eq!(order1, vec!["a", "m", "z", "pkg"]);
    }

    // =========================================================================
    // Property-based tests with proptest
    // =========================================================================

    /// Strategy to generate valid formula names (lowercase, alphanumeric with - and _)
    fn formula_name_strategy() -> impl Strategy<Value = String> {
        "[a-z][a-z0-9_-]{0,10}".prop_filter("non-empty", |s| !s.is_empty())
    }

    /// Generate a dependency graph with no cycles (DAG)
    /// Returns (root_name, formulas_map)
    fn acyclic_graph_strategy() -> impl Strategy<Value = (String, BTreeMap<String, Formula>)> {
        // Generate 1-5 package names
        prop::collection::vec(formula_name_strategy(), 1..=5)
            .prop_filter_map("unique names", |names| {
                let unique: BTreeSet<String> = names.into_iter().collect();
                if unique.is_empty() {
                    return None;
                }
                let names: Vec<_> = unique.into_iter().collect();
                Some(names)
            })
            .prop_flat_map(|names| {
                let n = names.len();
                // Generate dependency matrix: deps[i] can only depend on deps[j] where j < i
                // This guarantees no cycles
                let dep_bits = prop::collection::vec(
                    prop::collection::vec(prop::bool::ANY, n),
                    n,
                );
                (Just(names), dep_bits)
            })
            .prop_map(|(names, dep_matrix)| {
                let mut formulas = BTreeMap::new();
                for (i, name) in names.iter().enumerate() {
                    let deps: Vec<&str> = names
                        .iter()
                        .enumerate()
                        .filter(|(j, _)| *j < i && dep_matrix[i][*j])
                        .map(|(_, n)| n.as_str())
                        .collect();
                    formulas.insert(name.clone(), formula(name, &deps));
                }
                // Root is the last element (can depend on all others)
                let root = names.last().unwrap().clone();
                (root, formulas)
            })
    }

    proptest! {
        #[test]
        fn resolution_output_has_no_cycles((root, formulas) in acyclic_graph_strategy()) {
            if let Ok(order) = resolve_closure(&root, &formulas) {
                // Verify: each package appears only once
                let unique: BTreeSet<_> = order.iter().collect();
                prop_assert_eq!(unique.len(), order.len(), "Duplicate packages in output");

                // Verify: dependencies come before dependents
                let pos: BTreeMap<_, _> = order.iter().enumerate().map(|(i, n)| (n.clone(), i)).collect();
                for name in &order {
                    if let Some(f) = formulas.get(name) {
                        for dep in &f.dependencies {
                            if let Some(&dep_pos) = pos.get(dep) {
                                let pkg_pos = pos[name];
                                prop_assert!(
                                    dep_pos < pkg_pos,
                                    "Dependency {} (pos {}) should come before {} (pos {})",
                                    dep, dep_pos, name, pkg_pos
                                );
                            }
                        }
                    }
                }
            }
        }

        #[test]
        fn resolution_is_deterministic((root, formulas) in acyclic_graph_strategy()) {
            let result1 = resolve_closure(&root, &formulas);
            let result2 = resolve_closure(&root, &formulas);
            prop_assert_eq!(result1.is_ok(), result2.is_ok());
            if let (Ok(order1), Ok(order2)) = (result1, result2) {
                prop_assert_eq!(order1, order2, "Resolution should be deterministic");
            }
        }

        #[test]
        fn root_is_always_last_in_output((root, formulas) in acyclic_graph_strategy()) {
            if let Ok(order) = resolve_closure(&root, &formulas) {
                prop_assert!(!order.is_empty());
                prop_assert_eq!(
                    order.last().unwrap(),
                    &root,
                    "Root package should be last in install order"
                );
            }
        }
    }
}
