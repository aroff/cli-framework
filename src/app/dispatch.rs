use crate::ailoop::AiloopClient;
use crate::app::context::AppContext;
use crate::command::Command;
use crate::parser::diagnostic::Diagnostic;
use crate::spec::value::ArgValue;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum InvocationSurface {
    Cli,
    Chat,
    Mcp,
    Api,
}

impl InvocationSurface {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Chat => "chat",
            Self::Mcp => "mcp",
            Self::Api => "api",
        }
    }
}

pub(crate) struct DispatchEnv<'a> {
    pub(crate) command_registry: &'a crate::command::CommandRegistry,
    pub(crate) ailoop_client: &'a Option<AiloopClient>,
    pub(crate) global_args: &'a HashMap<String, ArgValue>,
    pub(crate) stdout_capture: Option<Arc<Mutex<Vec<u8>>>>,
    pub(crate) telemetry: Option<Arc<dyn crate::telemetry::Telemetry + Send + Sync>>,
    #[cfg(feature = "telemetry")]
    pub(crate) probe_registry: &'a crate::telemetry::ProbeRegistry,
    #[allow(dead_code)]
    pub(crate) surface: InvocationSurface,
    #[cfg(feature = "auth")]
    pub(crate) token_provider: Option<Arc<dyn crate::auth::TokenProvider>>,
    #[cfg(feature = "config")]
    pub(crate) config_handle: Option<Arc<dyn crate::config::ConfigHandle>>,
    #[cfg(feature = "config")]
    pub(crate) config_manifest: Option<Arc<crate::config::manifest::ConfigManifest>>,
    #[cfg(feature = "config-managed")]
    pub(crate) policy_client: Option<Arc<crate::config::managed::PolicyClient>>,
}

pub(crate) struct CliAppContextWrapper<'a> {
    _inner: &'a mut dyn AppContext,
    env: DispatchEnv<'a>,
}

impl<'a> CliAppContextWrapper<'a> {
    pub(crate) fn new(inner: &'a mut dyn AppContext, env: DispatchEnv<'a>) -> Self {
        Self { _inner: inner, env }
    }
}

impl<'a> AppContext for CliAppContextWrapper<'a> {
    fn opt_registry(&self) -> Option<&crate::command::CommandRegistry> {
        Some(self.env.command_registry)
    }

    fn opt_global_args(&self) -> Option<&HashMap<String, ArgValue>> {
        Some(self.env.global_args)
    }

    fn framework_println(&self, s: &str) {
        use std::io::Write;

        if let Some(ref buf) = self.env.stdout_capture {
            let mut lock = buf.lock().unwrap_or_else(|e| e.into_inner());
            lock.extend_from_slice(s.as_bytes());
            lock.push(b'\n');
            return;
        }

        let mut stdout = std::io::stdout();
        let _ = writeln!(stdout, "{}", s);
    }

    fn try_framework_println(&self, s: &str) -> std::io::Result<()> {
        if let Some(ref buf) = self.env.stdout_capture {
            let mut lock = buf.lock().unwrap_or_else(|error| error.into_inner());
            lock.extend_from_slice(s.as_bytes());
            lock.push(b'\n');
            return Ok(());
        }
        crate::app::context::write_output_line(&mut std::io::stdout().lock(), s)
    }

    #[cfg(feature = "testkit")]
    fn drain_output(&self) -> String {
        if let Some(ref buf) = self.env.stdout_capture {
            let mut lock = buf.lock().unwrap();
            let data = std::mem::take(&mut *lock);
            String::from_utf8_lossy(&data).into_owned()
        } else {
            String::new()
        }
    }

    #[cfg(feature = "auth")]
    fn opt_token_provider(&self) -> Option<Arc<dyn crate::auth::TokenProvider>> {
        self.env.token_provider.clone()
    }

    fn telemetry(&self) -> &dyn crate::telemetry::Telemetry {
        self.env
            .telemetry
            .as_deref()
            .unwrap_or(&crate::telemetry::NoopTelemetry)
    }

    fn opt_telemetry_arc(
        &self,
    ) -> Option<std::sync::Arc<dyn crate::telemetry::Telemetry + Send + Sync>> {
        self.env.telemetry.clone()
    }

    #[cfg(feature = "telemetry")]
    fn opt_probe_registry(&self) -> Option<&crate::telemetry::ProbeRegistry> {
        Some(self.env.probe_registry)
    }

    #[cfg(feature = "config")]
    fn opt_config_handle(&self) -> Option<&dyn crate::config::ConfigHandle> {
        self.env.config_handle.as_deref()
    }

    #[cfg(feature = "config")]
    fn opt_config_manifest(&self) -> Option<&crate::config::manifest::ConfigManifest> {
        self.env.config_manifest.as_deref()
    }

    #[cfg(feature = "config-managed")]
    fn opt_policy_client(&self) -> Option<Arc<crate::config::managed::PolicyClient>> {
        self.env.policy_client.clone()
    }
}

impl<'a> crate::app::context::CommandRegistryContext for CliAppContextWrapper<'a> {
    fn command_registry(&self) -> &crate::command::CommandRegistry {
        self.env.command_registry
    }

    fn execute_command_sync(
        &self,
        command_id: &str,
        args: HashMap<String, ArgValue>,
    ) -> anyhow::Result<()> {
        let command = self
            .command_registry()
            .get(command_id)
            .ok_or_else(|| anyhow::anyhow!("Command '{}' not found", command_id))?
            .clone();

        #[cfg(feature = "auth")]
        let provider = self.env.token_provider.clone();

        struct NestedContext {
            #[cfg(feature = "auth")]
            token_provider: Option<Arc<dyn crate::auth::TokenProvider>>,
        }
        impl AppContext for NestedContext {
            #[cfg(feature = "auth")]
            fn opt_token_provider(&self) -> Option<Arc<dyn crate::auth::TokenProvider>> {
                self.token_provider.clone()
            }
        }

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let mut ctx = NestedContext {
                    #[cfg(feature = "auth")]
                    token_provider: provider,
                };
                (command.execute)(&mut ctx, args).await
            })
        })
    }
}

impl<'a> crate::ailoop::AiloopContext for CliAppContextWrapper<'a> {
    fn ailoop_client(&self) -> Option<&AiloopClient> {
        self.env.ailoop_client.as_ref()
    }
}

#[cfg(all(test, feature = "testkit"))]
mod tests {
    use super::*;
    use crate::app::context::AppContext;
    use crate::command::CommandRegistry;

    struct DummyCtx;
    impl AppContext for DummyCtx {}

    /// Finding 5: CliAppContextWrapper must override drain_output so that
    /// content written via framework_println is returned, not silently lost.
    #[tokio::test(flavor = "multi_thread")]
    async fn cli_app_context_wrapper_drain_output_returns_captured_content() {
        use crate::ailoop::AiloopContext;
        use crate::app::context::CommandRegistryContext;
        let mut registry = CommandRegistry::new();
        let mut nested = Command {
            id: "nested".into(),
            spec: Arc::new(crate::spec::command_tree::CommandSpec::default()),
            validator: None,
            execute: Arc::new(|ctx, _| {
                Box::pin(async move {
                    #[cfg(feature = "auth")]
                    assert!(ctx.opt_token_provider().is_none());
                    let _ = ctx;
                    Ok(())
                })
            }),
            expose_mcp: false,
            expose_chat: false,
            meta: None,
            visibility: None,
        };
        assert!(validate_typed_args(&nested, &HashMap::new()).is_empty());
        registry.register(nested.clone());
        nested.validator = Some(Arc::new(|_| {
            vec![Diagnostic {
                code: "TEST",
                category: crate::parser::diagnostic::DiagnosticCategory::Validation,
                message: "expected custom validator".into(),
                suggestion: None,
                span: None,
            }]
        }));
        assert_eq!(validate_typed_args(&nested, &HashMap::new()).len(), 1);
        let ailoop_client: Option<AiloopClient> = None;
        let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let global_args_map: HashMap<String, ArgValue> = HashMap::new();
        #[cfg(feature = "telemetry")]
        let probe_registry = crate::telemetry::ProbeRegistry::with_builtins();
        let env = DispatchEnv {
            command_registry: &registry,
            ailoop_client: &ailoop_client,
            global_args: &global_args_map,
            stdout_capture: Some(buf.clone()),
            telemetry: None,
            #[cfg(feature = "telemetry")]
            probe_registry: &probe_registry,
            surface: InvocationSurface::Cli,
            #[cfg(feature = "auth")]
            token_provider: None,
            #[cfg(feature = "config")]
            config_handle: None,
            #[cfg(feature = "config")]
            config_manifest: None,
            #[cfg(feature = "config-managed")]
            policy_client: None,
        };
        let mut inner = DummyCtx;
        let mut wrapper = CliAppContextWrapper::new(&mut inner, env);

        wrapper.framework_println("hello world");
        wrapper.framework_println("line two");

        let output = wrapper.drain_output();
        assert_eq!(output, "hello world\nline two\n");

        // Second drain must be empty — buffer was consumed.
        assert!(wrapper.drain_output().is_empty());
        wrapper.try_framework_println("fallible 世界").unwrap();
        assert_eq!(wrapper.drain_output(), "fallible 世界\n");
        assert!(wrapper.opt_registry().is_some());
        assert!(wrapper.opt_global_args().unwrap().is_empty());
        assert!(wrapper.ailoop_client().is_none());
        assert!(wrapper.opt_telemetry_arc().is_none());
        wrapper.telemetry().event("contract", &[]);
        wrapper.env.telemetry = Some(Arc::new(crate::telemetry::NoopTelemetry));
        assert!(wrapper.opt_telemetry_arc().is_some());
        wrapper.telemetry().event("contract", &[]);
        #[cfg(feature = "auth")]
        assert!(wrapper.opt_token_provider().is_none());
        #[cfg(feature = "config")]
        {
            assert!(wrapper.opt_config_handle().is_none());
            assert!(wrapper.opt_config_manifest().is_none());
        }
        #[cfg(feature = "config-managed")]
        assert!(wrapper.opt_policy_client().is_none());
        #[cfg(feature = "telemetry")]
        assert!(wrapper.opt_probe_registry().is_some());
        assert!(wrapper
            .execute_command_sync("missing", HashMap::new())
            .is_err());
        wrapper
            .execute_command_sync("nested", HashMap::new())
            .unwrap();
        wrapper.env.stdout_capture = None;
        assert!(wrapper.drain_output().is_empty());
        wrapper
            .try_framework_println("framework live flushed-output contract")
            .unwrap();
        wrapper.framework_println("framework legacy-output contract");
        for (surface, expected) in [
            (InvocationSurface::Cli, "cli"),
            (InvocationSurface::Chat, "chat"),
            (InvocationSurface::Mcp, "mcp"),
            (InvocationSurface::Api, "api"),
        ] {
            assert_eq!(surface.as_str(), expected);
        }
    }
}

pub(crate) fn validate_typed_args(
    command: &Command,
    typed_args: &HashMap<String, ArgValue>,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();

    diags.extend(command.spec.validate_typed_args(typed_args));

    if let Some(ref validator) = command.validator {
        diags.extend(validator(typed_args));
    }

    diags
}
