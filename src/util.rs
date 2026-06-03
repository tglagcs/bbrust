//! Small helpers: human-readable sizes and path expansion.
//!
//! Ports the pieces of BleachBit's `FileUtilities.py` and `Action._set_paths`
//! that the Windows-only core needs.

use std::path::{Path, PathBuf};

/// Format a byte count like BleachBit's `bytes_to_human` (SI units, base 1000).
///
/// Examples: `0B`, `512B`, `1.5kB`, `3.2MB`, `1.10GB`.
pub fn bytes_to_human(bytes: i64) -> String {
    if bytes < 0 {
        return format!("-{}", bytes_to_human(-bytes));
    }
    if bytes == 0 {
        return "0B".to_string();
    }
    const BASE: f64 = 1000.0;
    const PREFIXES: [&str; 6] = ["", "k", "M", "G", "T", "P"];

    let b = bytes as f64;
    // BleachBit chooses decimals by magnitude of the original byte count.
    let decimals = if b >= BASE.powi(3) {
        2
    } else if b >= BASE {
        1
    } else {
        0
    };

    let mut value = b;
    for prefix in PREFIXES {
        if value < BASE {
            return format!("{:.*}{}B", decimals, value, prefix);
        }
        value /= BASE;
    }
    "A lot.".to_string()
}

/// Expand a raw CleanerML path into a concrete filesystem path.
///
/// Handles a leading `~` (user profile) and `%VAR%` environment variables,
/// then normalizes separators to backslash on Windows. Returns `None` when a
/// referenced environment variable is missing, so the caller can skip the path.
pub fn expand_path(raw: &str) -> Option<PathBuf> {
    if raw.is_empty() {
        return Some(PathBuf::new());
    }

    let mut expanded = expand_env_vars(raw)?;

    if expanded == "~" {
        expanded = home_dir()?;
    } else if let Some(rest) = expanded.strip_prefix("~/") {
        expanded = join_str(&home_dir()?, rest);
    } else if let Some(rest) = expanded.strip_prefix("~\\") {
        expanded = join_str(&home_dir()?, rest);
    }

    Some(normalize(&expanded))
}

/// Replace every `%VAR%` occurrence with its environment value.
/// Returns `None` if any referenced variable is unset (matching BleachBit,
/// which would otherwise produce a path that cannot exist).
fn expand_env_vars(input: &str) -> Option<String> {
    if !input.contains('%') {
        return Some(input.to_string());
    }
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => {
                let name = &after[..end];
                if name.is_empty() {
                    // "%%" -> literal "%"
                    out.push('%');
                } else {
                    let val = std::env::var(name).ok()?;
                    out.push_str(&val);
                }
                rest = &after[end + 1..];
            }
            None => {
                // Trailing unmatched '%': keep it verbatim.
                out.push('%');
                out.push_str(after);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    Some(out)
}

fn home_dir() -> Option<String> {
    std::env::var("USERPROFILE")
        .ok()
        .or_else(|| std::env::var("HOME").ok())
}

fn join_str(base: &str, rest: &str) -> String {
    let sep = if base.ends_with(['/', '\\']) { "" } else { "\\" };
    format!("{base}{sep}{rest}")
}

/// Normalize separators to the platform default and collapse redundant parts.
fn normalize(path: &str) -> PathBuf {
    #[cfg(windows)]
    let path = path.replace('/', "\\");
    #[cfg(not(windows))]
    let path = path.to_string();
    PathBuf::from(path)
}

/// True when a path string contains glob wildcards.
pub fn has_glob(s: &str) -> bool {
    s.bytes().any(|b| matches!(b, b'*' | b'?' | b'[' | b']'))
}

/// Best-effort size of a file or symlink in bytes; 0 if it cannot be read.
pub fn path_size(path: &Path) -> u64 {
    std::fs::symlink_metadata(path).map(|m| m.len()).unwrap_or(0)
}
