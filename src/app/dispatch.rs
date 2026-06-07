use crate::ailoop::AiloopClient;
use crate::app::context::AppContext;
use crate::command::Command;
use crate::parser::diagnostic::Diagnostic;
use crate::spec::value::ArgValue;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

pub(crate) struct DispatchEnv<'a> {
    pub(crate) command_registry: &'a crate::command::CommandRegistry,
    pub(crate) ailoop_client: &'a Option<AiloopClient>,
    pub(crate) global_args: &'a HashMap<String, ArgValue>,
    pub(crate) stdout_capture: Option<Arc<Mutex<Vec<u8>>>>,
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

        struct NoopContext;
        impl AppContext for NoopContext {}

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let mut ctx = NoopContext;
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
    #[test]
    fn cli_app_context_wrapper_drain_output_returns_captured_content() {
        let registry = CommandRegistry::new();
        let ailoop_client: Option<AiloopClient> = None;
        let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let global_args_map: HashMap<String, ArgValue> = HashMap::new();
        let env = DispatchEnv {
            command_registry: &registry,
            ailoop_client: &ailoop_client,
            global_args: &global_args_map,
            stdout_capture: Some(buf.clone()),
        };
        let mut inner = DummyCtx;
        let wrapper = CliAppContextWrapper::new(&mut inner, env);

        wrapper.framework_println("hello world");
        wrapper.framework_println("line two");

        let output = wrapper.drain_output();
        assert_eq!(output, "hello world\nline two\n");

        // Second drain must be empty — buffer was consumed.
        assert!(wrapper.drain_output().is_empty());
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
