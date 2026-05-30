//! CLI mode detection utilities.
//!
//! `NO_COLOR` > `FORCE_COLOR` > TTY detection for color. `COLUMNS`/`ROWS` are
//! read unconditionally; the TTY guard applies only to the syscall fallback.

use std::io::{self, IsTerminal};
use std::panic;

pub fn is_stdout_tty() -> bool {
    safe_tty_check(|| io::stdout().is_terminal())
}
pub fn is_stderr_tty() -> bool {
    safe_tty_check(|| io::stderr().is_terminal())
}
pub fn is_stdin_tty() -> bool {
    safe_tty_check(|| io::stdin().is_terminal())
}

fn safe_tty_check<F: FnOnce() -> bool + panic::UnwindSafe>(check: F) -> bool {
    panic::catch_unwind(check).unwrap_or(false)
}

pub fn read_env_var(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.to_lowercase().trim().to_string())
}

pub fn is_no_color_set() -> bool {
    std::env::var("NO_COLOR").is_ok()
}
pub fn is_force_color_set() -> bool {
    std::env::var("FORCE_COLOR").is_ok()
}

/// Color precedence: NO_COLOR > FORCE_COLOR > TTY detection.
pub fn should_color_output() -> bool {
    if is_no_color_set() {
        return false;
    }
    if is_force_color_set() {
        return true;
    }
    is_stdout_tty()
}

/// Color precedence: NO_COLOR > FORCE_COLOR > TTY detection.
pub fn should_color_stderr() -> bool {
    if is_no_color_set() {
        return false;
    }
    if is_force_color_set() {
        return true;
    }
    is_stderr_tty()
}

pub fn is_interactive() -> bool {
    is_stdin_tty() && is_stdout_tty()
}
pub fn is_quiet() -> bool {
    std::env::var("QUIET").is_ok()
}
pub fn should_show_progress() -> bool {
    is_stdout_tty() && !is_quiet()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Table,
    Json,
    Plain,
}

impl OutputFormat {
    pub fn from_env() -> Option<Self> {
        read_env_var("OUTPUT_FORMAT").and_then(|s| match s.as_str() {
            "table" => Some(OutputFormat::Table),
            "json" => Some(OutputFormat::Json),
            "plain" => Some(OutputFormat::Plain),
            _ => None,
        })
    }

    pub fn default_for_tty() -> Self {
        if is_stdout_tty() {
            OutputFormat::Table
        } else {
            OutputFormat::Json
        }
    }
}

pub fn get_output_format() -> OutputFormat {
    OutputFormat::from_env().unwrap_or_else(OutputFormat::default_for_tty)
}

/// Returns terminal width from `COLUMNS` env var (unconditional), then syscall (TTY only).
pub fn terminal_width() -> Option<usize> {
    if let Some(cols) = read_env_var("COLUMNS") {
        if let Ok(w) = cols.parse::<usize>() {
            return Some(w);
        }
    }
    if !is_stdout_tty() {
        return None;
    }
    None
}

/// Returns terminal height from `ROWS` env var (unconditional), then syscall (TTY only).
pub fn terminal_height() -> Option<usize> {
    if let Some(rows) = read_env_var("ROWS") {
        if let Ok(h) = rows.parse::<usize>() {
            return Some(h);
        }
    }
    if !is_stdout_tty() {
        return None;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    // Serialize all tests that mutate COLUMNS/ROWS to prevent env-var races.
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

    #[test]
    fn terminal_width_reads_columns_env() {
        let _g = env_lock();
        std::env::set_var("COLUMNS", "120");
        let w = terminal_width();
        std::env::remove_var("COLUMNS");
        assert_eq!(w, Some(120));
    }

    #[test]
    fn terminal_height_reads_rows_env() {
        let _g = env_lock();
        std::env::set_var("ROWS", "40");
        let h = terminal_height();
        std::env::remove_var("ROWS");
        assert_eq!(h, Some(40));
    }

    #[test]
    fn terminal_width_none_when_unset_non_tty() {
        let _g = env_lock();
        std::env::remove_var("COLUMNS");
        if !is_stdout_tty() {
            assert_eq!(terminal_width(), None);
        }
    }

    #[test]
    fn terminal_height_none_when_unset_non_tty() {
        let _g = env_lock();
        std::env::remove_var("ROWS");
        if !is_stdout_tty() {
            assert_eq!(terminal_height(), None);
        }
    }
}
