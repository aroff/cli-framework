//! A minimal end-user app with self-install enabled, for
//! `tests/integration/self_install_cli.rs` and the CI installer-script job.
//!
//! `CFW_DEMO_NAMESPACE=cli` places the built-ins under `cli`, so the group
//! is reached as `cfw-self-install-demo cli self ...`.

use cli_framework::app::AppContext;
use cli_framework::prelude::*;

struct Ctx;
impl AppContext for Ctx {}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut builder = AppBuilder::new()
        .with_version("cfw-self-install-demo", env!("CARGO_PKG_VERSION"))
        .with_self_install(SelfInstallOptions::github("aroff/cli-framework").completions(true));
    if let Ok(ns) = std::env::var("CFW_DEMO_NAMESPACE") {
        if !ns.is_empty() {
            builder = builder.with_builtin_command_namespace(&CommandPath::root_for(&ns));
        }
    }
    let mut app = builder.build(Ctx)?;
    app.run().await?;
    Ok(())
}
