//! Integration tests for the built-in `version` command.

use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::sync::{Arc, Mutex};

use cli_framework::app::{AppBuilder, AppContext};
use cli_framework::command::CommandArgs;

struct DummyCtx;
impl AppContext for DummyCtx {}

struct StdoutCapture {
    saved_fd: i32,
    tmp: tempfile::NamedTempFile,
}

impl StdoutCapture {
    fn new() -> Self {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let stdout_fd = std::io::stdout().as_raw_fd();
        let saved_fd = unsafe { libc::dup(stdout_fd) };
        unsafe {
            libc::dup2(tmp.as_raw_fd(), stdout_fd);
        }
        Self { saved_fd, tmp }
    }

    fn finish(self) -> String {
        let _ = std::io::stdout().flush();
        let stdout_fd = std::io::stdout().as_raw_fd();
        unsafe {
            libc::dup2(self.saved_fd, stdout_fd);
            libc::close(self.saved_fd);
        }
        let contents = std::fs::read_to_string(self.tmp.path()).unwrap_or_default();
        drop(self.tmp);
        contents
    }
}

struct LogCollector {
    records: Arc<Mutex<Vec<String>>>,
}

impl log::Log for LogCollector {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Warn
    }
    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            self.records
                .lock()
                .unwrap()
                .push(format!("{}", record.args()));
        }
    }
    fn flush(&self) {}
}

#[tokio::test]
async fn version_dispatch_with_version_configured() {
    let mut app = AppBuilder::new()
        .with_version("myapp", "1.2.3")
        .build(DummyCtx)
        .unwrap();

    let cap = StdoutCapture::new();
    app.run_with_args(vec!["myapp".to_string(), "version".to_string()])
        .await
        .unwrap();
    let output = cap.finish();

    assert_eq!(output, "myapp 1.2.3\n");
}

#[tokio::test]
async fn version_dispatch_double_dash_version() {
    let mut app = AppBuilder::new()
        .with_version("myapp", "1.2.3")
        .build(DummyCtx)
        .unwrap();

    let cap = StdoutCapture::new();
    app.run_with_args(vec!["myapp".to_string(), "--version".to_string()])
        .await
        .unwrap();
    let output = cap.finish();

    assert_eq!(output, "myapp 1.2.3\n");
}

#[tokio::test]
async fn version_dispatch_without_with_version_prints_unknown() {
    let mut app = AppBuilder::new().build(DummyCtx).unwrap();

    let cap = StdoutCapture::new();
    app.run_with_args(vec!["myapp".to_string(), "version".to_string()])
        .await
        .unwrap();
    let output = cap.finish();

    assert_eq!(output, "unknown unknown\n");
}

#[tokio::test]
async fn execute_command_version_returns_not_found() {
    let mut app = AppBuilder::new()
        .with_version("myapp", "1.2.3")
        .build(DummyCtx)
        .unwrap();

    let result = app.execute_command("version", CommandArgs::default()).await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));
}

#[test]
fn show_help_contains_version_entry() {
    let app = AppBuilder::new()
        .with_version("myapp", "1.2.3")
        .build(DummyCtx)
        .unwrap();

    let help = app.render_help();
    assert!(help.contains("version"));

    let version_line = help.lines().next().unwrap();
    assert_eq!(version_line, "  version - Print version information");
}

#[test]
fn show_help_version_appears_before_registered_commands() {
    use cli_framework::command::Command;

    let cmd = Command {
        id: "alpha",
        summary: "Alpha command",
        syntax: None,
        category: Some("test"),
        execute: Arc::new(|_ctx, _args| Box::pin(async move { Ok(()) })),
    };

    let app = AppBuilder::new()
        .with_version("myapp", "1.2.3")
        .register_command(cmd)
        .build(DummyCtx)
        .unwrap();

    let help = app.render_help();
    let version_pos = help.find("version - Print version information").unwrap();
    let alpha_pos = help.find("alpha").unwrap();
    assert!(version_pos < alpha_pos);
}

#[test]
fn warn_log_emitted_when_version_not_configured() {
    let records: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let collector = LogCollector {
        records: records.clone(),
    };
    log::set_boxed_logger(Box::new(collector)).unwrap();
    log::set_max_level(log::LevelFilter::Warn);

    let mut app = AppBuilder::new().build(DummyCtx).unwrap();

    let cap = StdoutCapture::new();
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(app.run_with_args(vec!["myapp".to_string(), "version".to_string()]))
        .unwrap();
    let _output = cap.finish();

    let msgs = records.lock().unwrap();
    assert!(msgs
        .iter()
        .any(|m| m.contains("with_version() was not configured")));

    log::set_max_level(log::LevelFilter::Off);
}
