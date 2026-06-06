//! Implementations of the special, non-delete file operations.
//!
//! Ports the relevant parts of BleachBit's `FileUtilities` (`vacuum_sqlite3`,
//! `clean_json`, `clean_ini`) and `Special.delete_office_registrymodifications`,
//! plus the `process` action. Each function performs the mutation in place; the
//! worker measures recovered space as size-before minus size-after.

use std::io::Write;
use std::path::Path;

use quick_xml::events::Event;
use quick_xml::{Reader, Writer};

/// `VACUUM` a SQLite database to reclaim free pages.
pub fn vacuum(path: &Path) -> Result<(), String> {
    let conn = rusqlite::Connection::open(path).map_err(|e| e.to_string())?;
    conn.execute_batch("VACUUM").map_err(|e| e.to_string())?;
    Ok(())
}

/// Remove the key addressed by `address` (a `/`-separated path) from a JSON file.
///
/// Ports `FileUtilities.clean_json`: descend dict by dict, delete the terminal
/// key, and rewrite only if something changed.
pub fn clean_json(path: &Path, address: &str) -> Result<(), String> {
    let raw = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let raw = raw.strip_prefix('\u{feff}').unwrap_or(&raw); // utf-8-sig
    let mut root: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| format!("invalid JSON: {e}"))?;

    let targets: Vec<&str> = address.split('/').collect();
    let changed = delete_json_target(&mut root, &targets);

    if changed {
        let out = serde_json::to_string(&root).map_err(|e| e.to_string())?;
        std::fs::write(path, out).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Walk `targets` through nested objects; delete the final key. Returns whether
/// anything was removed. Mirrors the descend/delete loop in `clean_json`.
fn delete_json_target(root: &mut serde_json::Value, targets: &[&str]) -> bool {
    let mut pos = root;
    for (i, key) in targets.iter().enumerate() {
        let Some(obj) = pos.as_object_mut() else {
            return false;
        };
        let last = i == targets.len() - 1;
        if last {
            return obj.remove(*key).is_some();
        }
        if !obj.contains_key(*key) {
            return false;
        }
        pos = obj.get_mut(*key).unwrap();
    }
    false
}

/// Remove an `.ini` section, or a single parameter within it.
///
/// A line-based pass (case-insensitive section/key match) that, unlike
/// BleachBit's `clean_ini`, also preserves comments and ordering.
pub fn clean_ini(path: &Path, section: &str, parameter: Option<&str>) -> Result<(), String> {
    let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut out = String::with_capacity(content.len());
    let mut in_target_section = false;
    let mut changed = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            let name = &trimmed[1..trimmed.len() - 1];
            in_target_section = name.eq_ignore_ascii_case(section);
            // Drop the header itself only when removing the whole section.
            if in_target_section && parameter.is_none() {
                changed = true;
                continue;
            }
            out.push_str(line);
            out.push('\n');
            continue;
        }

        if in_target_section {
            if parameter.is_none() {
                // Removing the whole section: skip its body.
                changed = true;
                continue;
            }
            if let Some(param) = parameter {
                let key = trimmed.split(['=', ':']).next().unwrap_or("").trim();
                if key.eq_ignore_ascii_case(param) {
                    changed = true;
                    continue;
                }
            }
        }

        out.push_str(line);
        out.push('\n');
    }

    if changed {
        std::fs::write(path, out).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Strip LibreOffice/OpenOffice MRU history `<item>` entries from
/// `registrymodifications.xcu`.
///
/// Ports `Special.delete_office_registrymodifications`: remove every `<item>`
/// whose `oor:path` begins with the Histories prefix, then rewrite if changed.
pub fn clean_office_registrymodifications(path: &Path) -> Result<(), String> {
    const PREFIX: &str = "/org.openoffice.Office.Histories/Histories/";

    let data = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut reader = Reader::from_str(&data);
    let mut writer = Writer::new(Vec::new());
    let mut changed = false;
    // >0 while inside an <item> subtree being dropped.
    let mut skipping_depth = 0usize;

    let mut buf = Vec::new();
    loop {
        let event = reader.read_event_into(&mut buf).map_err(|e| e.to_string())?;
        match &event {
            Event::Start(e) if e.local_name().as_ref() == b"item" => {
                if skipping_depth > 0 {
                    skipping_depth += 1;
                } else if item_is_history(e, PREFIX) {
                    changed = true;
                    skipping_depth = 1;
                } else {
                    writer.write_event(event.borrow()).map_err(|e| e.to_string())?;
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"item" => {
                if skipping_depth > 0 {
                    skipping_depth -= 1;
                } else {
                    writer.write_event(event.borrow()).map_err(|e| e.to_string())?;
                }
            }
            Event::Empty(e) if e.local_name().as_ref() == b"item" => {
                if skipping_depth > 0 {
                    // part of a dropped subtree
                } else if item_is_history(e, PREFIX) {
                    changed = true;
                } else {
                    writer.write_event(event.borrow()).map_err(|e| e.to_string())?;
                }
            }
            Event::Eof => break,
            _ => {
                if skipping_depth == 0 {
                    writer.write_event(event.borrow()).map_err(|e| e.to_string())?;
                }
            }
        }
        buf.clear();
    }

    if changed {
        std::fs::write(path, writer.into_inner()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn item_is_history(e: &quick_xml::events::BytesStart, prefix: &str) -> bool {
    for attr in e.attributes().flatten() {
        if attr.key.as_ref() == b"oor:path" {
            if let Ok(val) = attr.unescape_value() {
                return val.starts_with(prefix);
            }
        }
    }
    false
}

/// Run an external command line. When `wait`, block and warn on non-zero exit.
pub fn run_process(cmd: &str, wait: bool) -> Result<(), String> {
    let cmd = crate::util::expand_env_command(cmd);
    let args = shell_split(&cmd);
    let Some((program, rest)) = args.split_first() else {
        return Err("empty command".to_string());
    };
    let mut command = std::process::Command::new(program);
    command.args(rest);

    if wait {
        let status = command.status().map_err(|e| e.to_string())?;
        if !status.success() {
            log::warn!("command '{cmd}' exited with {status}");
        }
    } else {
        command.spawn().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Minimal command-line splitter honoring single and double quotes.
fn shell_split(input: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut has_token = false;

    for c in input.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => cur.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                has_token = true;
            }
            None if c.is_whitespace() => {
                if has_token {
                    args.push(std::mem::take(&mut cur));
                    has_token = false;
                }
            }
            None => {
                cur.push(c);
                has_token = true;
            }
        }
    }
    if has_token {
        args.push(cur);
    }
    args
}

/// Helper used by tests and the worker: write a file (utility kept local).
#[allow(dead_code)]
pub(crate) fn write_all(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut f = std::fs::File::create(path)?;
    f.write_all(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_deletes_nested_key() {
        let dir = std::env::temp_dir().join(format!("bbrust_json_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("c.json");
        std::fs::write(&p, r#"{"a":{"b":1,"c":2},"d":3}"#).unwrap();
        clean_json(&p, "a/b").unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert!(v["a"].get("b").is_none());
        assert_eq!(v["a"]["c"], 2);
        assert_eq!(v["d"], 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ini_removes_section_and_param() {
        let dir = std::env::temp_dir().join(format!("bbrust_ini_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("c.ini");
        std::fs::write(&p, "[keep]\nx=1\n\n[drop]\ny=2\nz=3\n").unwrap();
        clean_ini(&p, "drop", None).unwrap();
        let s = std::fs::read_to_string(&p).unwrap();
        assert!(s.contains("[keep]"));
        assert!(!s.contains("[drop]"));
        assert!(!s.contains("y=2"));

        std::fs::write(&p, "[s]\nkeepme=1\ndelme=2\n").unwrap();
        clean_ini(&p, "s", Some("delme")).unwrap();
        let s = std::fs::read_to_string(&p).unwrap();
        assert!(s.contains("keepme=1"));
        assert!(!s.contains("delme"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shell_split_handles_quotes() {
        assert_eq!(shell_split(r#"a "b c" d"#), vec!["a", "b c", "d"]);
    }

    #[cfg(windows)]
    #[test]
    fn run_process_expands_env_vars() {
        // `%WINDIR%\system32\cmd.exe /c exit` only resolves to a runnable program
        // if %WINDIR% is expanded first; otherwise Command::new fails with "program
        // not found" — the same failure that left Explorer killed but not relaunched.
        run_process("%WINDIR%\\system32\\cmd.exe /c exit", true).unwrap();
    }

    #[test]
    fn vacuum_reclaims_space() {
        let dir = std::env::temp_dir().join(format!("bbrust_vac_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("t.db");
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            conn.execute_batch("CREATE TABLE t(x BLOB);").unwrap();
            conn.execute_batch("BEGIN;").unwrap();
            for _ in 0..2000 {
                conn.execute("INSERT INTO t(x) VALUES (randomblob(4000));", [])
                    .unwrap();
            }
            conn.execute_batch("COMMIT; DELETE FROM t;").unwrap();
        }
        let before = std::fs::metadata(&db).unwrap().len();
        vacuum(&db).unwrap();
        let after = std::fs::metadata(&db).unwrap().len();
        assert!(after < before, "vacuum should shrink: {before} -> {after}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
