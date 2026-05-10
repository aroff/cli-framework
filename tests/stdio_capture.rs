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

pub struct StderrCapture {
    _guard: MutexGuard<'static, ()>,
    saved_fd: i32,
    tmp: tempfile::NamedTempFile,
}

impl StderrCapture {
    pub fn new() -> Self {
        let guard = stdio_lock().lock().unwrap();
        let tmp = tempfile::NamedTempFile::new().unwrap();
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
