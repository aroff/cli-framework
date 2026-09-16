//! Putting the bin dir on PATH for Unix shells.
//!
//! The rules (ADR 0080): a shared `env` file in the bin dir, identical for
//! every framework app and for uv; one line sourcing it, appended once to rc
//! files that already exist; a per-app fish `conf.d` file when fish is
//! configured. No rc file is created or rewritten, and uninstall never
//! removes the shared line.

use super::env::InstallEnv;
use super::receipt::PathModification;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// What a PATH edit did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnixPathReport {
    pub modifications: Vec<PathModification>,
    /// Things a person should know, such as a foreign `env` file left alone.
    pub notes: Vec<String>,
    /// The command that puts the bin dir on PATH in the current shell.
    pub current_shell_hint: String,
}

/// `$HOME/...` when the directory is under home, the absolute path
/// otherwise. `None` when the path contains characters that cannot be placed
/// inside double quotes safely.
pub(crate) fn shell_dir_expr(home: Option<&Path>, dir: &Path) -> Option<String> {
    let expr = match home.and_then(|h| dir.strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "$HOME".to_string(),
        Some(rest) => format!("$HOME/{}", rest.to_string_lossy()),
        None => dir.to_string_lossy().into_owned(),
    };
    let body = expr.strip_prefix("$HOME").unwrap_or(&expr);
    if body.contains(['"', '$', '`', '\\', '\n']) {
        None
    } else {
        Some(expr)
    }
}

/// The POSIX `env` file, in the same shape uv writes.
pub fn posix_env_file_contents(bin_expr: &str) -> String {
    format!(
        "#!/bin/sh\n\
         # add binaries to PATH if they aren't added yet\n\
         # affix colons on either side of $PATH to simplify matching\n\
         case \":${{PATH}}:\" in\n\
         \x20   *:\"{bin_expr}\":*)\n\
         \x20       ;;\n\
         \x20   *)\n\
         \x20       # Prepending path in case a system-installed binary needs to be overridden\n\
         \x20       export PATH=\"{bin_expr}:$PATH\"\n\
         \x20       ;;\n\
         esac\n"
    )
}

/// The fish `env.fish` file.
pub fn fish_env_file_contents(bin_expr: &str) -> String {
    format!(
        "if not contains \"{bin_expr}\" $PATH\n\
         \x20   # Prepending path in case a system-installed binary needs to be overridden\n\
         \x20   set -x PATH \"{bin_expr}\" $PATH\n\
         end\n"
    )
}

/// The rc files that exist, in the order they are edited. Zsh uses
/// `.zshenv`, or `.zshrc` when only that one exists (the macOS default).
pub fn rc_candidates(home: &Path, zdotdir: Option<&Path>) -> Vec<PathBuf> {
    let zdot = zdotdir.unwrap_or(home);
    let mut names: Vec<PathBuf> = [".profile", ".bash_profile", ".bash_login", ".bashrc"]
        .iter()
        .map(|n| home.join(n))
        .collect();
    let zshenv = zdot.join(".zshenv");
    if zshenv.is_file() {
        names.push(zshenv);
    } else {
        names.push(zdot.join(".zshrc"));
    }
    names.into_iter().filter(|p| p.is_file()).collect()
}

/// Write the env files and rc lines that put `bin_dir` on PATH.
pub fn apply_unix_path(env: &InstallEnv, bin_dir: &Path) -> io::Result<UnixPathReport> {
    let mut report = UnixPathReport::default();
    let Some(home) = env.home.as_deref() else {
        report
            .notes
            .push("no home directory; add the bin dir to PATH yourself".into());
        return Ok(report);
    };
    let Some(bin_expr) = shell_dir_expr(Some(home), bin_dir) else {
        report.notes.push(format!(
            "{} contains characters that cannot be quoted in a shell file; add it to PATH yourself",
            bin_dir.display()
        ));
        return Ok(report);
    };
    let env_expr = format!("{bin_expr}/env");
    report.current_shell_hint = format!(". \"{env_expr}\"");

    if !ensure_env_file(
        &bin_dir.join("env"),
        &posix_env_file_contents(&bin_expr),
        &bin_expr,
        &mut report,
    )? {
        return Ok(report);
    }

    let line = format!(". \"{env_expr}\"");
    let alt = format!("source \"{env_expr}\"");
    let zdotdir = env.var("ZDOTDIR").map(PathBuf::from);
    let candidates = rc_candidates(home, zdotdir.as_deref());
    for rc in &candidates {
        let existing = std::fs::read_to_string(rc)?;
        if existing
            .lines()
            .any(|l| l.trim() == line || l.trim() == alt)
        {
            continue;
        }
        let mut f = std::fs::OpenOptions::new().append(true).open(rc)?;
        let sep = if existing.is_empty() || existing.ends_with('\n') {
            ""
        } else {
            "\n"
        };
        writeln!(f, "{sep}{line}")?;
        report.modifications.push(PathModification::RcLine {
            file: rc.clone(),
            line: line.clone(),
        });
    }
    if candidates.is_empty() {
        report.notes.push(format!(
            "no shell rc file found; add this line to your shell's startup file: {line}"
        ));
    }

    let fish_dir = env
        .var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"))
        .join("fish");
    if fish_dir.is_dir() {
        let fish_ok = ensure_env_file(
            &bin_dir.join("env.fish"),
            &fish_env_file_contents(&bin_expr),
            &bin_expr,
            &mut report,
        )?;
        if fish_ok {
            let conf = fish_dir.join("conf.d").join(format!("{}.fish", env.app));
            let wanted = format!("source \"{bin_expr}/env.fish\"\n");
            match std::fs::read_to_string(&conf) {
                Ok(c) if c == wanted => {}
                Ok(_) => report.notes.push(format!(
                    "{} exists with other content; left unchanged",
                    conf.display()
                )),
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    std::fs::create_dir_all(fish_dir.join("conf.d"))?;
                    std::fs::write(&conf, wanted)?;
                    report
                        .modifications
                        .push(PathModification::FishConf { file: conf });
                }
                Err(e) => return Err(e),
            }
        }
    }
    Ok(report)
}

/// Create the env file, or accept an existing one that already names the
/// bin dir (uv's, or a sibling app's). A file that does not is left alone
/// and PATH editing stops, because sourcing it would not do what we promise.
fn ensure_env_file(
    path: &Path,
    contents: &str,
    bin_expr: &str,
    report: &mut UnixPathReport,
) -> io::Result<bool> {
    match std::fs::read_to_string(path) {
        Ok(existing) if existing.contains(&format!("\"{bin_expr}\"")) => Ok(true),
        Ok(_) => {
            report.notes.push(format!(
                "{} exists but does not add {bin_expr} to PATH; left unchanged, add the directory to PATH yourself",
                path.display()
            ));
            Ok(false)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            std::fs::write(path, contents)?;
            report.modifications.push(PathModification::EnvFile {
                file: path.to_path_buf(),
            });
            Ok(true)
        }
        Err(e) => Err(e),
    }
}

/// Remove a per-app fish `conf.d` file. `Ok(false)` when it was already gone.
pub fn remove_fish_conf(file: &Path) -> io::Result<bool> {
    match std::fs::remove_file(file) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}
