use crate::ailoop::AiloopClient;
use crate::app::context::AppContext;
use crate::command::{Command, CommandArgs};
use crate::parser::validator::SpecValidator;
use crate::spec::value::ArgValue;
use std::collections::HashMap;
#[cfg(feature = "testkit")]
use std::sync::Arc;
#[cfg(feature = "testkit")]
use std::sync::Mutex;

pub(crate) struct DispatchEnv<'a> {
    pub(crate) command_registry: &'a crate::command::CommandRegistry,
    pub(crate) ailoop_client: &'a Option<AiloopClient>,
    #[cfg(feature = "testkit")]
    pub(crate) stdout_capture: Option<Arc<Mutex<Vec<u8>>>>,
}

pub(crate) struct CliAppContextWrapper<'a> {
    inner: &'a mut dyn AppContext,
    env: DispatchEnv<'a>,
}

impl<'a> CliAppContextWrapper<'a> {
    pub(crate) fn new(inner: &'a mut dyn AppContext, env: DispatchEnv<'a>) -> Self {
        Self { inner, env }
    }
}

impl<'a> AppContext for CliAppContextWrapper<'a> {
    fn opt_registry(&self) -> Option<&crate::command::CommandRegistry> {
        Some(self.env.command_registry)
    }

    fn framework_println(&self, s: &str) {
        use std::io::Write;

        #[cfg(feature = "testkit")]
        if let Some(ref buf) = self.env.stdout_capture {
            let mut lock = buf.lock().unwrap();
            lock.extend_from_slice(s.as_bytes());
            lock.push(b'\n');
            return;
        }

        let mut stdout = std::io::stdout();
        let _ = writeln!(stdout, "{}", s);
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        self.inner.as_any()
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        self.inner.as_any_mut()
    }
}

impl<'a> crate::app::context::CommandRegistryContext for CliAppContextWrapper<'a> {
    fn command_registry(&self) -> &crate::command::CommandRegistry {
        self.env.command_registry
    }

    fn execute_command_sync(
        &self,
        command_id: &str,
        args: crate::command::CommandArgs,
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
    fn ailoop_client(&self) -> &AiloopClient {
        self.env
            .ailoop_client
            .as_ref()
            .expect("Ailoop client not configured")
    }
}

pub(crate) fn validate_typed_args(
    command: &Command,
    typed_args: &HashMap<String, ArgValue>,
) -> anyhow::Result<Vec<crate::parser::diagnostic::Diagnostic>> {
    let mut diags = Vec::new();

    if let Some(ref spec) = command.spec {
        diags.extend(SpecValidator::validate(spec, typed_args));
    }

    if let Some(ref validator) = command.validator {
        diags.extend(validator(typed_args));
    }

    Ok(diags)
}

pub(crate) fn effective_args_for_execution(
    args: CommandArgs,
    typed_args: Option<&HashMap<String, ArgValue>>,
) -> CommandArgs {
    let Some(typed_map) = typed_args else {
        return args;
    };

    let mut named = HashMap::new();
    for (k, v) in typed_map {
        let s = match v {
            ArgValue::Bool(b) => b.to_string(),
            ArgValue::Str(s) => s.clone(),
            ArgValue::Int(i) => i.to_string(),
            ArgValue::Float(f) => f.to_string(),
            ArgValue::Enum(e) => e.clone(),
            ArgValue::Count(c) => c.to_string(),
            ArgValue::List(items) => items
                .iter()
                .map(|i| match i {
                    ArgValue::Str(s) => s.clone(),
                    ArgValue::Int(i) => i.to_string(),
                    ArgValue::Float(f) => f.to_string(),
                    ArgValue::Enum(e) => e.clone(),
                    _ => String::new(),
                })
                .collect::<Vec<_>>()
                .join(","),
        };
        named.insert(k.clone(), s);
    }

    CommandArgs {
        positional: Vec::new(),
        named,
    }
}
