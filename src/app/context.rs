//! AppContext trait and helpers
//!
//! AppContext represents application-owned state and service clients.

/// Application context trait
///
/// Applications implement this trait to provide their own state and service clients.
/// The framework uses this to pass context to views, datasources, and commands.
///
/// # Thread Safety
///
/// AppContext implementations must be `Send + Sync` to ensure thread safety in the async runtime.
/// This allows the framework to safely share context across async tasks and threads.
pub trait AppContext: Send + Sync {
    // Applications define their own structure
    // Framework only requires the trait to exist

    /// Provides access to the frozen command registry, populated by `AppBuilder::build`.
    /// Returns `None` for contexts that do not expose the registry (e.g., user-defined contexts).
    fn opt_registry(&self) -> Option<&crate::command::CommandRegistry> {
        None
    }

    /// Write a line of user-visible output.
    ///
    /// Commands should prefer this over `println!` so framework consumers can
    /// capture output deterministically in tests.
    fn framework_println(&self, s: &str) {
        use std::io::Write;
        let mut stdout = std::io::stdout();
        let _ = writeln!(stdout, "{}", s);
    }

    /// Drain and return any output captured since the last call.
    ///
    /// Contexts that capture `framework_println` output override this to return
    /// and clear the internal buffer. The default returns an empty string.
    fn drain_output(&self) -> String {
        String::new()
    }

    /// Attach a structured-content value to the current command result.
    ///
    /// A command sets this to return machine-facing structured data (an MCP
    /// `structuredContent` object) *separately* from the human/model-facing text
    /// emitted via `framework_println`. The default is a no-op; only contexts
    /// that participate in the MCP tool bridge capture it (see
    /// `drain_structured_content`). Calling this on a non-capturing context is
    /// harmless and simply discards the value.
    fn framework_set_structured_content(&self, _value: serde_json::Value) {}

    /// Drain and return any structured content set since the last call.
    ///
    /// Contexts that capture `framework_set_structured_content` override this to
    /// return and clear the stored value. The default returns `None`.
    fn drain_structured_content(&self) -> Option<serde_json::Value> {
        None
    }

    /// Return the global args parsed for the current invocation, if available.
    ///
    /// Returns `None` for contexts that do not carry global args (e.g., user-defined
    /// contexts outside the dispatch path). The dispatch wrapper always provides `Some`.
    fn opt_global_args(
        &self,
    ) -> Option<&std::collections::HashMap<String, crate::spec::value::ArgValue>> {
        None
    }

    /// Return the [`TokenProvider`][crate::auth::TokenProvider] configured for this
    /// app, if any.
    ///
    /// Commands that need to acquire bearer tokens call this instead of holding
    /// a direct reference to the provider. Returns `None` when the `auth` feature
    /// is disabled or when no provider was registered via
    /// [`AppBuilder::with_token_provider`][crate::app::AppBuilder::with_token_provider].
    #[cfg(feature = "auth")]
    fn opt_token_provider(&self) -> Option<std::sync::Arc<dyn crate::auth::TokenProvider>> {
        None
    }

    /// Return the telemetry handle for the current invocation.
    ///
    /// The default implementation returns a no-op handle. The dispatch wrapper
    /// overrides this with the active telemetry provider when one is configured.
    fn telemetry(&self) -> &dyn crate::telemetry::Telemetry {
        static NOOP: crate::telemetry::NoopTelemetry = crate::telemetry::NoopTelemetry;
        &NOOP
    }

    /// Return the active telemetry handle as a cloneable `Arc`, if available.
    ///
    /// The default returns `None`. The dispatch wrapper overrides this so that
    /// command closures can extract and forward the handle across async boundaries
    /// (e.g., to the MCP serve path where `ctx` cannot be moved into the future).
    fn opt_telemetry_arc(
        &self,
    ) -> Option<std::sync::Arc<dyn crate::telemetry::Telemetry + Send + Sync>> {
        None
    }

    /// Return the caller identity established for the current request, as an
    /// opaque type-erased value.
    ///
    /// The default returns `None`. Only the MCP tool-dispatch context
    /// (`McpAppContext`, used by both the HTTP and stdio transports) overrides
    /// this — and only when the host installed a per-request identity hook via
    /// [`crate::app::AppBuilder::with_mcp_request_authenticator`] (HTTP only;
    /// under stdio there is no HTTP request to authenticate, so this remains
    /// `None` even when a hook is installed).
    ///
    /// cli-framework never inspects the value: it is produced by the host's
    /// own authenticator closure (headers → identity) and is opaque here.
    /// Prefer the typed [`RequestIdentityExt::request_identity`] helper over
    /// calling this directly.
    fn opt_request_identity(&self) -> Option<std::sync::Arc<dyn std::any::Any + Send + Sync>> {
        None
    }

    /// Return the type-erased configuration handle for the current
    /// invocation, if a config store was wired via
    /// [`AppBuilder::with_config`][crate::app::AppBuilder::with_config].
    ///
    /// The default implementation returns `None`. The dispatch wrapper
    /// overrides this with the active [`ConfigHandle`][crate::config::ConfigHandle]
    /// when one is configured. This cannot return the **typed** resolved
    /// value: `T` differs per application and a generic method is not
    /// object-safe on a trait used polymorphically (see spec 016, "Access,
    /// and why it cannot mirror the `Telemetry` handle exactly"). A handler
    /// that needs the typed value reaches it through the application's own
    /// context type instead — see `AppBuilder::build_with_config` and
    /// `App::config_store`.
    #[cfg(feature = "config")]
    fn opt_config_handle(&self) -> Option<&dyn crate::config::ConfigHandle> {
        None
    }

    /// Return the application's registered
    /// [`ConfigManifest`][crate::config::manifest::ConfigManifest], if one
    /// was declared via
    /// [`AppBuilder::with_config_manifest`][crate::app::AppBuilder::with_config_manifest].
    ///
    /// The default implementation returns `None`. The dispatch wrapper
    /// overrides this with the registered manifest when one is configured.
    /// This is what the built-in `config` command group (spec 021, "Command
    /// surface") resolves and renders against — `config show`/`config
    /// manifest` report [`CFG001`][crate::parser::error_codes::CFG001] when
    /// this returns `None`, which should not normally happen since that
    /// command group is only auto-registered once a manifest has been
    /// declared.
    #[cfg(feature = "config")]
    fn opt_config_manifest(&self) -> Option<&crate::config::manifest::ConfigManifest> {
        None
    }

    /// Return the [`PolicyClient`][crate::config::managed::PolicyClient]
    /// registered for this app, if one was wired via
    /// [`AppBuilder::with_policy_client`][crate::app::AppBuilder::with_policy_client].
    ///
    /// The default implementation returns `None`. Returned as an owned
    /// `Arc` (mirroring [`Self::opt_token_provider`]) so a handler can move
    /// it into an async block that outlives the `&mut dyn AppContext`
    /// borrow — exactly the case `config profile`/`config refresh` are in.
    #[cfg(feature = "config-managed")]
    fn opt_policy_client(&self) -> Option<std::sync::Arc<crate::config::managed::PolicyClient>> {
        None
    }

    /// Return the registered feature-probe catalog for the current
    /// invocation, if one is available.
    ///
    /// The default returns `None`. The dispatch wrapper overrides this with
    /// the app's [`ProbeRegistry`][crate::telemetry::ProbeRegistry] — the
    /// same registry [`AppBuilder::with_telemetry_ops`][crate::app::AppBuilder]
    /// extends — so [`mark_feature`](Self::mark_feature) can tell a
    /// registered feature name from one nobody declared. The method itself is
    /// not gated on the `telemetry` feature — `ProbeRegistry` is an
    /// always-compiled type (see its module doc), so the signature costs
    /// nothing to keep available in every build. The dispatch wrapper's
    /// override *is* gated (there is no live registry to hand back without
    /// `telemetry`), so a build with the feature off simply keeps the `None`
    /// default here, same as any other context that never wires this up.
    fn opt_probe_registry(&self) -> Option<&crate::telemetry::ProbeRegistry> {
        None
    }

    /// Record that a named, author-defined feature ran.
    ///
    /// `name` becomes a `cli.feature` event unconditionally. It becomes a
    /// `feature` metric label too, but only when `name` was registered via
    /// `AppBuilder::with_telemetry_ops` — an unregistered name is unbounded
    /// cardinality as a label (`mark_feature(&user_input)` in a loop would
    /// mint one time series per input) but stays useful as an event, which
    /// is bounded by the trace it sits in.
    ///
    /// An unregistered name also warns once per distinct name — not once per
    /// process, so a second, different mistyped name still gets its own
    /// warning — and trips a `debug_assert!`, so the framework's own test
    /// suites catch an unregistered `mark_feature` call before it ships. The
    /// warning state lives in a function-local `static`, one per concrete
    /// `Self` (the usual trait-default-method monomorphization), which is
    /// exactly "once per distinct name for as long as this context type is
    /// in use" — the same process, in every case this framework builds
    /// today.
    #[cfg(feature = "telemetry")]
    fn mark_feature(&self, name: &str) {
        let registered: Vec<&str> = self
            .opt_probe_registry()
            .map(crate::telemetry::registered_feature_names)
            .unwrap_or_default();
        let outcome = crate::telemetry::feature_outcome(&registered, name);

        if outcome == crate::telemetry::FeatureOutcome::Unregistered {
            static WARNED: std::sync::OnceLock<
                std::sync::Mutex<std::collections::HashSet<String>>,
            > = std::sync::OnceLock::new();
            let warned =
                WARNED.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()));
            let is_new_name = warned
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(name.to_string());
            if is_new_name {
                tracing::warn!(
                    feature = name,
                    "telemetry: this feature name is not registered, so it will not \
                     appear as a metric label. Register it with \
                     AppBuilder::with_telemetry_ops."
                );
            }
            debug_assert!(false, "unregistered telemetry feature name: {name}");
        }

        self.telemetry().event(
            "cli.feature",
            &crate::telemetry::feature_attrs(name, outcome),
        );
    }

    /// `mark_feature` without the `telemetry` feature: still callable so
    /// handler code never needs to `cfg`-gate the call, but there is no
    /// probe catalog and no handle worth warning through, so it is a no-op.
    #[cfg(not(feature = "telemetry"))]
    fn mark_feature(&self, _name: &str) {}
}

/// Typed accessor over [`AppContext::opt_request_identity`].
///
/// Blanket-implemented for every `AppContext`, so any command's `execute`
/// closure (which receives `&mut dyn AppContext`) can downcast the opaque
/// per-request identity into its own type without needing the concrete
/// context type.
///
/// Returns `None` when: no authenticator hook is installed, the request
/// carried no identity (the host's closure returned `None`), the current
/// context doesn't participate in per-request identity (e.g. CLI/stdio), or
/// `T` does not match the type the host's closure actually produced.
pub trait RequestIdentityExt {
    /// Downcast the current request's opaque identity into `T`.
    fn request_identity<T: std::any::Any + Send + Sync>(&self) -> Option<std::sync::Arc<T>>;
}

impl<C: AppContext + ?Sized> RequestIdentityExt for C {
    fn request_identity<T: std::any::Any + Send + Sync>(&self) -> Option<std::sync::Arc<T>> {
        self.opt_request_identity()?.downcast::<T>().ok()
    }
}

/// Extension trait for AppContext to provide command registry access
pub trait CommandRegistryContext {
    /// Get the command registry for command lookup and metadata
    fn command_registry(&self) -> &crate::command::CommandRegistry;

    /// Execute another command by ID
    fn execute_command_sync(
        &self,
        command_id: &str,
        args: std::collections::HashMap<String, crate::spec::value::ArgValue>,
    ) -> anyhow::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct PlainCtx;
    impl AppContext for PlainCtx {}

    struct MyIdentity;

    /// Opt-in default: a context that never overrides `opt_request_identity`
    /// (every context outside the MCP tool-dispatch path) always yields
    /// `None` through the typed accessor too — the seam changes nothing for
    /// existing `AppContext` implementors that don't participate in it.
    #[test]
    fn default_opt_request_identity_yields_none_via_typed_accessor() {
        let ctx = PlainCtx;
        assert!(ctx.request_identity::<MyIdentity>().is_none());
    }
}
