//! The per-user `Path` in `HKEY_CURRENT_USER\Environment`.
//!
//! Written as `REG_EXPAND_SZ` so entries such as `%USERPROFILE%\bin` keep
//! expanding, and never through `setx`, which truncates at 1024 characters.
//! Every change broadcasts `WM_SETTINGCHANGE` so new terminals see it.
//! The key is a parameter so tests edit a scratch key, not the real one.

use std::io;
use std::path::Path;
use winreg::enums::{RegType, HKEY_CURRENT_USER};
use winreg::{RegKey, RegValue};

fn open(key_path: &str) -> io::Result<RegKey> {
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(key_path)?;
    Ok(key)
}

fn read_entries(key: &RegKey) -> io::Result<Vec<String>> {
    let value = match key.get_raw_value("Path") {
        Ok(v) => v,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let wide: Vec<u16> = value
        .bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&c| c != 0)
        .collect();
    Ok(String::from_utf16_lossy(&wide)
        .split(';')
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
        .collect())
}

fn write_entries(key: &RegKey, entries: &[String]) -> io::Result<()> {
    let joined = entries.join(";");
    let bytes: Vec<u8> = joined
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect();
    key.set_raw_value(
        "Path",
        &RegValue {
            bytes,
            vtype: RegType::REG_EXPAND_SZ,
        },
    )?;
    broadcast_environment_change();
    Ok(())
}

fn normalize(entry: &str) -> String {
    let expanded = expand_env(entry.trim());
    expanded
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

/// Expand `%VAR%` references from the process environment, for comparison.
fn expand_env(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => {
                let name = &after[..end];
                match std::env::var(name) {
                    Ok(v) if !name.is_empty() => out.push_str(&v),
                    _ => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Whether the user `Path` under `key_path` already lists `dir`.
pub fn user_path_contains(key_path: &str, dir: &Path) -> io::Result<bool> {
    let wanted = normalize(&dir.to_string_lossy());
    Ok(read_entries(&open(key_path)?)?
        .iter()
        .any(|e| normalize(e) == wanted))
}

/// Prepend `dir` to the user `Path`. `Ok(false)` when it was already there.
pub fn add_to_user_path(key_path: &str, dir: &Path) -> io::Result<bool> {
    let key = open(key_path)?;
    let mut entries = read_entries(&key)?;
    let wanted = normalize(&dir.to_string_lossy());
    if entries.iter().any(|e| normalize(e) == wanted) {
        return Ok(false);
    }
    entries.insert(0, dir.to_string_lossy().into_owned());
    write_entries(&key, &entries)?;
    Ok(true)
}

/// Remove every entry equal to `dir`. `Ok(false)` when none was present.
pub fn remove_from_user_path(key_path: &str, dir: &Path) -> io::Result<bool> {
    let key = open(key_path)?;
    let entries = read_entries(&key)?;
    let wanted = normalize(&dir.to_string_lossy());
    let kept: Vec<String> = entries
        .iter()
        .filter(|e| normalize(e) != wanted)
        .cloned()
        .collect();
    if kept.len() == entries.len() {
        return Ok(false);
    }
    write_entries(&key, &kept)?;
    Ok(true)
}

fn broadcast_environment_change() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
    };
    let param: Vec<u16> = "Environment\0".encode_utf16().collect();
    let mut result: usize = 0;
    // SAFETY: `param` outlives the call and is NUL-terminated; a hung window
    // is skipped after five seconds and the result is ignored.
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            param.as_ptr() as isize,
            SMTO_ABORTIFHUNG,
            5000,
            &mut result,
        );
    }
}
