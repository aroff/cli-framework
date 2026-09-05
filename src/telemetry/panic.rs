//! The `cli.panic` probe.
//!
//! A panic is the event a CLI most needs to report and is least able to: the
//! process is on its way down, and the hook that records it must not itself
//! panic. Three things follow.
//!
//! * **Location at usage, message at debug.** A location is the framework's own
//!   source coordinates and says what broke without saying anything about the
//!   person running it. A message is formatted from program data and routinely
//!   quotes it — a path, a value, an argument.
//! * **Chain, never replace.** An application's own panic hook, including the
//!   default backtrace printer, still runs.
//! * **A pure `panic_record`.** You cannot assert against a hook that ended the
//!   process, but you can assert against the value it would have recorded.

/// What the panic probe records.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PanicRecord {
    /// `file:line:column` from the panic's location.
    pub location: Option<String>,
    /// The panic payload, when it is a string. `None` for any other payload —
    /// a placeholder like `Box<dyn Any>` is noise that looks like data.
    pub message: Option<String>,
}

/// Extract the record from a panic, without touching global state.
pub fn panic_record(info: &std::panic::PanicHookInfo<'_>) -> PanicRecord {
    let location = info
        .location()
        .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()));

    let payload = info.payload();
    let message = payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned());

    PanicRecord { location, message }
}

/// Install a panic hook that records, then defers to whatever was there.
///
/// `record` must not panic and must not block for long: it runs while the
/// process is unwinding.
pub fn install_panic_hook(record: impl Fn(PanicRecord) + Send + Sync + 'static) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Recording must never turn a panic into a double panic, which aborts
        // the process before the application's own hook can print anything.
        let captured = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            record(panic_record(info));
        }));
        debug_assert!(captured.is_ok(), "the panic recorder must not panic");
        previous(info);
    }));
}
