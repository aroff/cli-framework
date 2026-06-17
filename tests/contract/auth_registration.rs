//! Contract tests for auth command auto-registration, collision detection,
//! and hard-exclusion from MCP/chat tool surfaces.

use cli_framework::app::{AppBuilder, AppContext};
use cli_framework::auth::{AccessToken, AuthError, TokenProvider};
use std::sync::Arc;

struct Ctx;
impl AppContext for Ctx {}

struct StubProvider;
#[async_trait::async_trait]
impl TokenProvider for StubProvider {
    async fn token(&self) -> Result<AccessToken, AuthError> {
        Err(AuthError::NotAuthenticated)
    }
    async fn invalidate(&self) {}
}

fn app_with_provider() -> cli_framework::app::App<Ctx> {
    AppBuilder::new()
        .with_version("testapp", "1.0")
        .with_token_provider(Arc::new(StubProvider))
        .build(Ctx)
        .unwrap()
}

// ── Auto-registration ─────────────────────────────────────────────────────────

#[test]
fn with_token_provider_registers_exactly_four_auth_commands() {
    let app = app_with_provider();
    let registry = app.command_registry();

    let auth_paths: Vec<&str> = ["auth/login", "auth/logout", "auth/status", "auth/token"]
        .iter()
        .copied()
        .collect();

    for path in &auth_paths {
        let resolved = registry.resolve(
            &cli_framework::spec::command_tree::CommandPath::new(
                &path.split('/').collect::<Vec<_>>(),
            )
            .unwrap(),
        );
        assert!(
            resolved.is_some(),
            "auth command '{path}' must be registered after with_token_provider"
        );
    }
}

#[test]
fn auth_group_metadata_present() {
    let app = app_with_provider();
    let registry = app.command_registry();
    let meta = registry.group_metadata_for("auth");
    assert!(meta.is_some(), "auth group must have metadata registered");
}

#[test]
fn without_token_provider_no_auth_commands() {
    let app = AppBuilder::new()
        .with_version("testapp", "1.0")
        .build(Ctx)
        .unwrap();
    let registry = app.command_registry();

    for leaf in &["auth/login", "auth/logout", "auth/status", "auth/token"] {
        let parts: Vec<_> = leaf.split('/').collect();
        let path = cli_framework::spec::command_tree::CommandPath::new(&parts).unwrap();
        assert!(
            registry.resolve(&path).is_none(),
            "'{leaf}' must NOT be registered without with_token_provider"
        );
    }
}

#[test]
fn pre_registered_auth_group_causes_build_error() {
    use cli_framework::spec::command_tree::{CommandPath, GroupMetadata};

    let result = AppBuilder::new()
        .with_version("testapp", "1.0")
        .register_group(
            &CommandPath::root_for("auth"),
            GroupMetadata {
                summary: "pre-existing auth",
                hidden: false,
            },
        )
        .unwrap()
        .with_token_provider(Arc::new(StubProvider))
        .build(Ctx);

    assert!(
        result.is_err(),
        "build() must return Err when 'auth' group is pre-registered"
    );
}

// ── MCP hard-exclusion ────────────────────────────────────────────────────────

#[cfg(feature = "mcp-server")]
#[test]
fn auth_commands_absent_from_mcp_tools_with_all_commands_policy() {
    use cli_framework::mcp::{McpToolExportPolicy, McpToolRegistry};

    let app = app_with_provider();
    let registry = app.command_registry();

    let mcp_registry = McpToolRegistry::from_command_registry_with_policy(
        registry,
        "testapp",
        McpToolExportPolicy::AllCommands,
    );

    for auth_cmd in &[
        "testapp_auth_login",
        "testapp_auth_logout",
        "testapp_auth_status",
        "testapp_auth_token",
    ] {
        assert!(
            mcp_registry.resolve_tool(auth_cmd).is_none(),
            "auth command '{auth_cmd}' must NOT appear in MCP tool list even with AllCommands policy"
        );
    }
}

// ── Auth commands carry expose_chat=false ────────────────────────────────────

#[test]
fn auth_commands_have_expose_chat_false() {
    let app = app_with_provider();
    let registry = app.command_registry();

    for leaf in &["auth/login", "auth/logout", "auth/status", "auth/token"] {
        let parts: Vec<_> = leaf.split('/').collect();
        let path = cli_framework::spec::command_tree::CommandPath::new(&parts).unwrap();
        let cmd = registry
            .resolve(&path)
            .unwrap_or_else(|| panic!("'{leaf}' must be registered"));
        assert!(
            !cmd.expose_chat,
            "auth command '{leaf}' must have expose_chat=false"
        );
        assert!(
            !cmd.expose_mcp,
            "auth command '{leaf}' must have expose_mcp=false"
        );
    }
}
