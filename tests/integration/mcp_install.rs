//! Integration tests for `mcp install` end-to-end (AC8, Stage 3).
//!
//! These tests exercise the full dispatch path through `app.run_with_args` and
//! verify that `aikit_sdk::add_mcp_server` is called with a valid
//! `AddMcpServerOptions`, writing a config file to a temporary directory
//! rather than the real agent config location.

#[cfg(feature = "mcp-install")]
mod tests {
    use cli_framework::app::{AppBuilder, AppContext};

    struct DummyCtx;
    impl AppContext for DummyCtx {}

    /// AC8: `prog mcp install` calls `add_mcp_server` with a valid `AddMcpServerOptions`
    /// (HTTP transport, project scope) and exits 0. A temp directory is used as the
    /// project root so no real agent config is modified.
    #[tokio::test]
    async fn mcp_install_http_project_scope_succeeds() {
        let tempdir = tempfile::tempdir().expect("could not create temp dir");

        let mut app = AppBuilder::new()
            .with_version("testapp", "0.1.0")
            .build(DummyCtx)
            .unwrap();

        let result = app
            .run_with_args(vec![
                "testapp".to_string(),
                "mcp".to_string(),
                "install".to_string(),
                "--scope".to_string(),
                "project".to_string(),
                "--project".to_string(),
                tempdir.path().to_str().unwrap().to_string(),
                "--agent".to_string(),
                "claude".to_string(),
                "--overwrite".to_string(),
            ])
            .await;

        assert!(
            result.is_ok(),
            "mcp install (HTTP, project scope) failed: {:?}",
            result
        );
    }

    /// AC9: `prog mcp register` (alias) calls the same install logic as `mcp install`.
    #[tokio::test]
    async fn mcp_register_alias_project_scope_succeeds() {
        let tempdir = tempfile::tempdir().expect("could not create temp dir");

        let mut app = AppBuilder::new()
            .with_version("testapp", "0.1.0")
            .build(DummyCtx)
            .unwrap();

        let result = app
            .run_with_args(vec![
                "testapp".to_string(),
                "mcp".to_string(),
                "register".to_string(),
                "--scope".to_string(),
                "project".to_string(),
                "--project".to_string(),
                tempdir.path().to_str().unwrap().to_string(),
                "--agent".to_string(),
                "claude".to_string(),
                "--overwrite".to_string(),
            ])
            .await;

        assert!(
            result.is_ok(),
            "mcp register alias (HTTP, project scope) failed: {:?}",
            result
        );
    }
}
