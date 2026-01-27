//! Integration tests for info/list/search commands using TestContext.
//!
//! These tests verify the CLI's data layer operations work correctly
//! with a real (mocked) backend.

use zb_io::test_utils::TestContext;

// ============================================================================
// list_installed Tests
// ============================================================================

mod list_installed {
    use super::*;

    #[tokio::test]
    async fn test_list_empty() {
        let ctx = TestContext::new().await;
        let installed = ctx.installer().list_installed().unwrap();
        assert!(installed.is_empty());
    }

    #[tokio::test]
    async fn test_list_with_single_package() {
        let mut ctx = TestContext::new().await;
        ctx.mount_formula("git", "2.44.0", &[]).await;

        ctx.installer_mut().install("git", true).await.unwrap();

        let installed = ctx.installer().list_installed().unwrap();
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].name, "git");
        assert_eq!(installed[0].version, "2.44.0");
    }

    #[tokio::test]
    async fn test_list_with_multiple_packages() {
        let mut ctx = TestContext::new().await;
        ctx.mount_formula("git", "2.44.0", &[]).await;
        ctx.mount_formula("curl", "8.6.0", &[]).await;
        ctx.mount_formula("jq", "1.7", &[]).await;

        ctx.installer_mut().install("git", true).await.unwrap();
        ctx.installer_mut().install("curl", true).await.unwrap();
        ctx.installer_mut().install("jq", true).await.unwrap();

        let installed = ctx.installer().list_installed().unwrap();
        assert_eq!(installed.len(), 3);

        let names: Vec<&str> = installed.iter().map(|k| k.name.as_str()).collect();
        assert!(names.contains(&"git"));
        assert!(names.contains(&"curl"));
        assert!(names.contains(&"jq"));
    }

    #[tokio::test]
    async fn test_list_includes_dependencies() {
        let mut ctx = TestContext::new().await;
        ctx.mount_formula("openssl", "3.0.0", &[]).await;
        ctx.mount_formula("curl", "8.6.0", &["openssl"]).await;

        // Install curl which has openssl as a dependency
        ctx.installer_mut().install("curl", true).await.unwrap();

        let installed = ctx.installer().list_installed().unwrap();
        // Both curl and openssl should be installed
        assert_eq!(installed.len(), 2);

        let names: Vec<&str> = installed.iter().map(|k| k.name.as_str()).collect();
        assert!(names.contains(&"curl"));
        assert!(names.contains(&"openssl"));
    }

    #[tokio::test]
    async fn test_list_pinned_empty() {
        let ctx = TestContext::new().await;
        let pinned = ctx.installer().list_pinned().unwrap();
        assert!(pinned.is_empty());
    }

    #[tokio::test]
    async fn test_list_pinned_after_pinning() {
        let mut ctx = TestContext::new().await;
        ctx.mount_formula("node", "22.0.0", &[]).await;
        ctx.mount_formula("python", "3.12.0", &[]).await;

        ctx.installer_mut().install("node", true).await.unwrap();
        ctx.installer_mut().install("python", true).await.unwrap();

        // Pin only node
        ctx.installer_mut().pin("node").unwrap();

        let pinned = ctx.installer().list_pinned().unwrap();
        assert_eq!(pinned.len(), 1);
        assert_eq!(pinned[0].name, "node");
        assert!(pinned[0].pinned);

        // All installed should still have 2 packages
        let all_installed = ctx.installer().list_installed().unwrap();
        assert_eq!(all_installed.len(), 2);
    }

    #[tokio::test]
    async fn test_list_pinned_multiple() {
        let mut ctx = TestContext::new().await;
        ctx.mount_formula("a", "1.0.0", &[]).await;
        ctx.mount_formula("b", "2.0.0", &[]).await;
        ctx.mount_formula("c", "3.0.0", &[]).await;

        ctx.installer_mut().install("a", true).await.unwrap();
        ctx.installer_mut().install("b", true).await.unwrap();
        ctx.installer_mut().install("c", true).await.unwrap();

        // Pin a and c, but not b
        ctx.installer_mut().pin("a").unwrap();
        ctx.installer_mut().pin("c").unwrap();

        let pinned = ctx.installer().list_pinned().unwrap();
        assert_eq!(pinned.len(), 2);

        let names: Vec<&str> = pinned.iter().map(|k| k.name.as_str()).collect();
        assert!(names.contains(&"a"));
        assert!(names.contains(&"c"));
        assert!(!names.contains(&"b"));
    }
}

// ============================================================================
// get_installed / Info Lookup Tests
// ============================================================================

mod get_installed {
    use super::*;

    #[tokio::test]
    async fn test_get_installed_not_found() {
        let ctx = TestContext::new().await;
        let keg = ctx.installer().get_installed("nonexistent");
        assert!(keg.is_none());
    }

    #[tokio::test]
    async fn test_get_installed_basic() {
        let mut ctx = TestContext::new().await;
        ctx.mount_formula("ripgrep", "14.1.0", &[]).await;

        ctx.installer_mut().install("ripgrep", true).await.unwrap();

        let keg = ctx.installer().get_installed("ripgrep");
        assert!(keg.is_some());

        let keg = keg.unwrap();
        assert_eq!(keg.name, "ripgrep");
        assert_eq!(keg.version, "14.1.0");
        assert!(keg.explicit); // Installed explicitly, not as dependency
        assert!(!keg.pinned); // Not pinned by default
    }

    #[tokio::test]
    async fn test_get_installed_dependency_not_explicit() {
        let mut ctx = TestContext::new().await;
        ctx.mount_formula("zlib", "1.3.0", &[]).await;
        ctx.mount_formula("libpng", "1.6.0", &["zlib"]).await;

        // Install libpng which pulls in zlib as dependency
        ctx.installer_mut().install("libpng", true).await.unwrap();

        // libpng should be explicit
        let libpng = ctx.installer().get_installed("libpng").unwrap();
        assert!(libpng.explicit);

        // zlib should NOT be explicit (it's a dependency)
        let zlib = ctx.installer().get_installed("zlib").unwrap();
        assert!(!zlib.explicit);
    }

    #[tokio::test]
    async fn test_get_installed_pinned_status() {
        let mut ctx = TestContext::new().await;
        ctx.mount_formula("nodejs", "20.0.0", &[]).await;

        ctx.installer_mut().install("nodejs", true).await.unwrap();

        // Initially not pinned
        let keg = ctx.installer().get_installed("nodejs").unwrap();
        assert!(!keg.pinned);

        // Pin it
        ctx.installer_mut().pin("nodejs").unwrap();

        // Now should be pinned
        let keg = ctx.installer().get_installed("nodejs").unwrap();
        assert!(keg.pinned);

        // Unpin it
        ctx.installer_mut().unpin("nodejs").unwrap();

        // Back to not pinned
        let keg = ctx.installer().get_installed("nodejs").unwrap();
        assert!(!keg.pinned);
    }

    #[tokio::test]
    async fn test_get_installed_store_key_present() {
        let mut ctx = TestContext::new().await;
        ctx.mount_formula("fd", "9.0.0", &[]).await;

        ctx.installer_mut().install("fd", true).await.unwrap();

        let keg = ctx.installer().get_installed("fd").unwrap();
        // Store key should be a non-empty SHA256 hash (64 hex chars)
        assert!(!keg.store_key.is_empty());
        assert!(keg.store_key.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[tokio::test]
    async fn test_get_installed_timestamp_present() {
        let mut ctx = TestContext::new().await;
        ctx.mount_formula("bat", "0.24.0", &[]).await;

        ctx.installer_mut().install("bat", true).await.unwrap();

        let keg = ctx.installer().get_installed("bat").unwrap();
        // Timestamp should be non-zero and reasonable
        assert!(keg.installed_at > 0);
        // Should be within the last few minutes (sanity check)
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        assert!(keg.installed_at <= now);
        assert!(keg.installed_at > now - 3600); // Within last hour
    }

    #[tokio::test]
    async fn test_is_installed_true() {
        let mut ctx = TestContext::new().await;
        ctx.mount_formula("exa", "0.10.0", &[]).await;

        ctx.installer_mut().install("exa", true).await.unwrap();

        assert!(ctx.installer().is_installed("exa"));
    }

    #[tokio::test]
    async fn test_is_installed_false() {
        let ctx = TestContext::new().await;
        assert!(!ctx.installer().is_installed("nonexistent"));
    }

    #[tokio::test]
    async fn test_is_installed_after_uninstall() {
        let mut ctx = TestContext::new().await;
        ctx.mount_formula("htop", "3.3.0", &[]).await;

        ctx.installer_mut().install("htop", true).await.unwrap();
        assert!(ctx.installer().is_installed("htop"));

        ctx.installer_mut().uninstall("htop").unwrap();
        assert!(!ctx.installer().is_installed("htop"));
    }
}

// ============================================================================
// JSON Output Structure Tests
// ============================================================================

mod json_output {
    use super::*;
    use serde_json::Value;

    #[tokio::test]
    async fn test_installed_keg_serializable() {
        let mut ctx = TestContext::new().await;
        ctx.mount_formula("wget", "1.21.0", &[]).await;

        ctx.installer_mut().install("wget", true).await.unwrap();

        let keg = ctx.installer().get_installed("wget").unwrap();

        // Verify the keg fields can be used to build valid JSON
        let json = serde_json::json!({
            "name": keg.name,
            "version": keg.version,
            "store_key": keg.store_key,
            "installed_at": keg.installed_at,
            "pinned": keg.pinned,
            "explicit": keg.explicit,
        });

        assert_eq!(json["name"], "wget");
        assert_eq!(json["version"], "1.21.0");
        assert!(json["store_key"].is_string());
        assert!(json["installed_at"].is_number());
        assert_eq!(json["pinned"], false);
        assert_eq!(json["explicit"], true);
    }

    #[tokio::test]
    async fn test_list_to_json_array() {
        let mut ctx = TestContext::new().await;
        ctx.mount_formula("pkg1", "1.0.0", &[]).await;
        ctx.mount_formula("pkg2", "2.0.0", &[]).await;

        ctx.installer_mut().install("pkg1", true).await.unwrap();
        ctx.installer_mut().install("pkg2", true).await.unwrap();

        let installed = ctx.installer().list_installed().unwrap();

        // Convert to JSON array
        let json: Value = serde_json::to_value(
            installed
                .iter()
                .map(|k| {
                    serde_json::json!({
                        "name": k.name,
                        "version": k.version,
                        "pinned": k.pinned,
                    })
                })
                .collect::<Vec<_>>(),
        )
        .unwrap();

        assert!(json.is_array());
        let arr = json.as_array().unwrap();
        assert_eq!(arr.len(), 2);
    }

    #[tokio::test]
    async fn test_not_found_json_structure() {
        let ctx = TestContext::new().await;
        let keg = ctx.installer().get_installed("missing");

        // When formula not found, we should represent it as not installed
        let json = serde_json::json!({
            "name": "missing",
            "installed": keg.is_some(),
        });

        assert_eq!(json["name"], "missing");
        assert_eq!(json["installed"], false);
    }
}

// ============================================================================
// Search Preconditions Tests
// ============================================================================

mod search_preconditions {
    use super::*;

    #[tokio::test]
    async fn test_installer_can_check_installed_for_search() {
        let mut ctx = TestContext::new().await;
        ctx.mount_formula("ripgrep", "14.1.0", &[]).await;
        ctx.mount_formula("grep", "3.11", &[]).await;

        // Install only ripgrep
        ctx.installer_mut().install("ripgrep", true).await.unwrap();

        // When filtering search results, we need to check is_installed
        let is_rg_installed = ctx.installer().is_installed("ripgrep");
        let is_grep_installed = ctx.installer().is_installed("grep");

        assert!(is_rg_installed);
        assert!(!is_grep_installed);
    }

    #[tokio::test]
    async fn test_installer_state_consistent_after_install_uninstall() {
        let mut ctx = TestContext::new().await;
        ctx.mount_formula("testpkg", "1.0.0", &[]).await;

        // Install
        ctx.installer_mut().install("testpkg", true).await.unwrap();
        assert!(ctx.installer().is_installed("testpkg"));
        assert_eq!(ctx.installer().list_installed().unwrap().len(), 1);

        // Uninstall
        ctx.installer_mut().uninstall("testpkg").unwrap();
        assert!(!ctx.installer().is_installed("testpkg"));
        assert_eq!(ctx.installer().list_installed().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_get_formula_from_api() {
        let mut ctx = TestContext::new().await;
        ctx.mount_formula("jq", "1.7.1", &[]).await;

        // Can fetch formula info from API even if not installed
        let formula = ctx.installer_mut().get_formula("jq").await;
        assert!(formula.is_ok());

        let formula = formula.unwrap();
        assert_eq!(formula.name, "jq");
        assert_eq!(formula.versions.stable, "1.7.1");
    }

    #[tokio::test]
    async fn test_get_formula_not_found() {
        let mut ctx = TestContext::new().await;

        // Formula not mounted = 404
        let result = ctx.installer_mut().get_formula("nonexistent").await;
        assert!(result.is_err());
    }
}

// ============================================================================
// Edge Cases
// ============================================================================

mod edge_cases {
    use super::*;

    #[tokio::test]
    async fn test_versioned_formula_name() {
        let mut ctx = TestContext::new().await;
        ctx.mount_formula("python@3.11", "3.11.9", &[]).await;

        ctx.installer_mut()
            .install("python@3.11", true)
            .await
            .unwrap();

        let keg = ctx.installer().get_installed("python@3.11");
        assert!(keg.is_some());
        assert_eq!(keg.unwrap().name, "python@3.11");
    }

    #[tokio::test]
    async fn test_formula_with_hyphen_in_name() {
        let mut ctx = TestContext::new().await;
        ctx.mount_formula("clang-format", "18.0.0", &[]).await;

        ctx.installer_mut()
            .install("clang-format", true)
            .await
            .unwrap();

        assert!(ctx.installer().is_installed("clang-format"));
    }

    #[tokio::test]
    async fn test_reinstall_same_version() {
        let mut ctx = TestContext::new().await;
        ctx.mount_formula("pkg", "1.0.0", &[]).await;

        // Install twice
        ctx.installer_mut().install("pkg", true).await.unwrap();
        let first_install = ctx.installer().get_installed("pkg").unwrap();

        ctx.installer_mut().install("pkg", true).await.unwrap();
        let second_install = ctx.installer().get_installed("pkg").unwrap();

        // Should still be installed with same version
        assert_eq!(first_install.version, second_install.version);
        assert_eq!(first_install.store_key, second_install.store_key);
    }

    #[tokio::test]
    async fn test_linked_files_after_install() {
        let mut ctx = TestContext::new().await;
        ctx.mount_formula("testcli", "1.0.0", &[]).await;

        ctx.installer_mut().install("testcli", true).await.unwrap();

        // Should have linked files (at minimum the binary)
        let linked = ctx.installer().get_linked_files("testcli");
        assert!(linked.is_ok());

        let linked = linked.unwrap();
        // The mock tarball creates a bin/<name> file
        assert!(!linked.is_empty());
    }

    #[tokio::test]
    async fn test_get_dependents_no_dependents() {
        let mut ctx = TestContext::new().await;
        ctx.mount_formula("standalone", "1.0.0", &[]).await;

        ctx.installer_mut()
            .install("standalone", true)
            .await
            .unwrap();

        let dependents = ctx.installer_mut().get_dependents("standalone").await;
        assert!(dependents.is_ok());
        assert!(dependents.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_get_dependents_with_dependents() {
        let mut ctx = TestContext::new().await;
        ctx.mount_formula("libbase", "1.0.0", &[]).await;
        ctx.mount_formula("app1", "2.0.0", &["libbase"]).await;
        ctx.mount_formula("app2", "3.0.0", &["libbase"]).await;

        // Install both apps (which pulls in libbase)
        ctx.installer_mut().install("app1", true).await.unwrap();
        ctx.installer_mut().install("app2", true).await.unwrap();

        // libbase should have app1 and app2 as dependents
        let dependents = ctx.installer_mut().get_dependents("libbase").await;
        assert!(dependents.is_ok());

        let deps = dependents.unwrap();
        assert_eq!(deps.len(), 2);
        assert!(deps.contains(&"app1".to_string()));
        assert!(deps.contains(&"app2".to_string()));
    }
}
