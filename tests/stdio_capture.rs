use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::sync::{Mutex, MutexGuard, OnceLock};

fn stdio_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub struct StdoutCapture {
    _guard: MutexGuard<'static, ()>,
    saved_fd: i32,
    tmp: tempfile::NamedTempFile,
}

impl StdoutCapture {
    pub fn new() -> Self {
        let guard = stdio_lock().lock().unwrap();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let _ = std::io::stdout().flush();
        let stdout_fd = std::io::stdout().as_raw_fd();
        let saved_fd = unsafe { libc::dup(stdout_fd) };
        unsafe {
            libc::dup2(tmp.as_raw_fd(), stdout_fd);
        }
        Self {
            _guard: guard,
            saved_fd,
            tmp,
        }
    }

    pub fn finish(self) -> String {
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

/// Strip lines emitted by the Rust test harness from output captured via a
/// process-wide `dup2` on fd 1. When tests run in parallel within the same
/// binary, the harness writes per-test status lines (e.g. `test foo ... ok`)
/// to the real fd 1 while another test holds the dup2 redirection, so they
/// land in the capture file and must be filtered out before assertions.
#[allow(dead_code)]
pub fn strip_test_harness_noise(s: &str) -> String {
    s.lines()
        .filter(|l| {
            let t = l.trim_start_matches('.').trim();
            if t.is_empty() {
                return true; // keep blank lines for shape
            }
            !(t.starts_with("test ")
                || t.starts_with("running ")
                || t.starts_with("test result:")
                || t == "failures:"
                || t == "successes:")
        })
        .map(|l| l.trim_start_matches('.'))
        .collect::<Vec<_>>()
        .join("\n")
}

#[allow(dead_code)]
pub struct StderrCapture {
    _guard: MutexGuard<'static, ()>,
    saved_fd: i32,
    tmp: tempfile::NamedTempFile,
}

impl StderrCapture {
    #[allow(dead_code)]
    pub fn new() -> Self {
        let guard = stdio_lock().lock().unwrap();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let _ = std::io::stderr().flush();
        let stderr_fd = std::io::stderr().as_raw_fd();
        let saved_fd = unsafe { libc::dup(stderr_fd) };
        unsafe {
            libc::dup2(tmp.as_raw_fd(), stderr_fd);
        }
        Self {
            _guard: guard,
            saved_fd,
            tmp,
        }
    }

    #[allow(dead_code)]
    pub fn finish(self) -> String {
        let _ = std::io::stderr().flush();
        let stderr_fd = std::io::stderr().as_raw_fd();
        unsafe {
            libc::dup2(self.saved_fd, stderr_fd);
            libc::close(self.saved_fd);
        }
        let contents = std::fs::read_to_string(self.tmp.path()).unwrap_or_default();
        drop(self.tmp);
        contents
    }
}
