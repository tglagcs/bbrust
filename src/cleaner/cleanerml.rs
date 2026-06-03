//! Parse CleanerML (`*.xml`) into [`Cleaner`]s.
//!
//! Ports the relevant parts of `CleanerML.py` for the Windows-only core. The XML
//! shape is shallow and regular:
//!
//! ```xml
//! <cleaner id="...">
//!   <label>..</label>
//!   <description>..</description>
//!   <option id="..">
//!     <label>..</label>
//!     <description>..</description>
//!     <warning>..</warning>
//!     <action command=".." search=".." path=".." regex=".." .../>
//!   </option>
//! </cleaner>
//! ```

use std::collections::HashMap;
use std::path::Path;

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use super::{os_match, Cleaner, CleanerOption};
use crate::actions::{Action, FileAction, FileKind, Searcher};

/// Where text content collected so far should be stored.
#[derive(Clone, Copy, PartialEq)]
enum Capture {
    None,
    CleanerLabel,
    CleanerDescription,
    OptionLabel,
    OptionDescription,
    OptionWarning,
}

/// Parse a CleanerML file into a [`Cleaner`].
pub fn parse_file(path: &Path) -> Result<Cleaner, String> {
    let mut reader = Reader::from_file(path).map_err(|e| e.to_string())?;
    reader.config_mut().trim_text(true);
    parse_reader(&mut reader)
}

/// Parse CleanerML from an in-memory string (used by tests).
pub fn parse_str(xml: &str) -> Result<Cleaner, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    parse_reader(&mut reader)
}

fn parse_reader<R: std::io::BufRead>(reader: &mut Reader<R>) -> Result<Cleaner, String> {
    let mut cleaner = Cleaner {
        id: String::new(),
        name: String::new(),
        description: String::new(),
        options: Vec::new(),
    };

    let mut cleaner_os_ok = true;
    let mut in_option = false;
    let mut current_option: Option<CleanerOption> = None;
    let mut capture = Capture::None;
    let mut text = String::new();

    let mut buf = Vec::new();
    loop {
        let event = reader.read_event_into(&mut buf).map_err(|e| e.to_string())?;
        match event {
            Event::Start(e) => {
                let name = local_name(&e);
                match name.as_str() {
                    "cleaner" => {
                        let attrs = attrs(&e);
                        cleaner.id = attrs.get("id").cloned().unwrap_or_default();
                        cleaner_os_ok = os_match(attrs.get("os").map(String::as_str).unwrap_or(""));
                    }
                    "option" if cleaner_os_ok => {
                        let attrs = attrs(&e);
                        in_option = true;
                        current_option = Some(CleanerOption {
                            id: attrs.get("id").cloned().unwrap_or_default(),
                            name: String::new(),
                            description: String::new(),
                            warning: None,
                            actions: Vec::new(),
                        });
                    }
                    "label" => capture = if in_option { Capture::OptionLabel } else { Capture::CleanerLabel },
                    "description" => {
                        capture = if in_option { Capture::OptionDescription } else { Capture::CleanerDescription }
                    }
                    "warning" if in_option => capture = Capture::OptionWarning,
                    "action" if cleaner_os_ok => {
                        // An action may be written as a Start element with a body.
                        if let Some(opt) = current_option.as_mut() {
                            if let Some(action) = parse_action(&attrs(&e)) {
                                opt.actions.push(action);
                            }
                        }
                    }
                    _ => {}
                }
                text.clear();
            }
            Event::Empty(e) => {
                let name = local_name(&e);
                if name == "action" && cleaner_os_ok {
                    if let Some(opt) = current_option.as_mut() {
                        if let Some(action) = parse_action(&attrs(&e)) {
                            opt.actions.push(action);
                        }
                    }
                }
            }
            Event::Text(e) => {
                if capture != Capture::None {
                    let chunk = e.unescape().map_err(|err| err.to_string())?;
                    text.push_str(&chunk);
                }
            }
            Event::End(e) => {
                let name = local_name_end(&e);
                match name.as_str() {
                    "label" => {
                        match capture {
                            Capture::CleanerLabel => cleaner.name = text.clone(),
                            Capture::OptionLabel => {
                                if let Some(o) = current_option.as_mut() {
                                    o.name = text.clone();
                                }
                            }
                            _ => {}
                        }
                        capture = Capture::None;
                    }
                    "description" => {
                        match capture {
                            Capture::CleanerDescription => cleaner.description = text.clone(),
                            Capture::OptionDescription => {
                                if let Some(o) = current_option.as_mut() {
                                    o.description = text.clone();
                                }
                            }
                            _ => {}
                        }
                        capture = Capture::None;
                    }
                    "warning" => {
                        if let Some(o) = current_option.as_mut() {
                            o.warning = Some(text.clone());
                        }
                        capture = Capture::None;
                    }
                    "option" => {
                        if let Some(opt) = current_option.take() {
                            cleaner.options.push(opt);
                        }
                        in_option = false;
                    }
                    _ => {}
                }
                text.clear();
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    Ok(cleaner)
}

/// Build an [`Action`] from an `<action>` element's attributes, honoring `os=`.
fn parse_action(attrs: &HashMap<String, String>) -> Option<Action> {
    let os = attrs.get("os").map(String::as_str).unwrap_or("");
    if !os_match(os) {
        return None;
    }
    let command = attrs.get("command").map(String::as_str).unwrap_or("");
    let get = |k: &str| attrs.get(k).map(String::as_str).unwrap_or("");

    // A searcher built from the standard file-locating attributes, reused by the
    // file-based special actions.
    let searcher = || {
        Searcher::new(
            get("search"),
            get("path"),
            get("regex"),
            get("nregex"),
            get("wholeregex"),
            get("nwholeregex"),
            get("type"),
        )
    };

    if let Some(kind) = FileKind::from_command(command) {
        return Some(Action::File(FileAction::new(
            kind,
            get("search"),
            get("path"),
            get("regex"),
            get("nregex"),
            get("wholeregex"),
            get("nwholeregex"),
            get("type"),
        )));
    }

    match command {
        "sqlite.vacuum" => Some(Action::Vacuum(searcher())),
        "json" => Some(Action::Json {
            searcher: searcher(),
            address: get("address").to_string(),
        }),
        "ini" => Some(Action::Ini {
            searcher: searcher(),
            section: get("section").to_string(),
            parameter: {
                let p = get("parameter");
                if p.is_empty() {
                    None
                } else {
                    Some(p.to_string())
                }
            },
        }),
        "office_registrymodifications" => Some(Action::Office(searcher())),
        "winreg" => Some(Action::Winreg {
            keyname: get("path").to_string(),
            valuename: get("name").to_string(),
        }),
        "win.shell.change.notify" => Some(Action::ShellNotify),
        "clipboard.clear" => Some(Action::Clipboard),
        "recycle.bin.empty" => Some(Action::RecycleBin),
        "process" => Some(Action::Process {
            cmd: get("cmd").to_string(),
            wait: !matches!(
                get("wait").chars().next().map(|c| c.to_ascii_lowercase()),
                Some('f') | Some('n')
            ),
        }),
        // Browser history cleaning (chrome.*, mozilla.*, cookie) is intentionally
        // dropped from this fork: it needs intricate, version-dependent SQL that is
        // easy to get wrong and risks deleting bookmarks. Skipped entirely, so
        // options that contain only these actions disappear from the UI.
        c if is_browser_command(c) => None,
        // Any other unrecognized command: keep as a no-op so the cleaner still loads.
        other => Some(Action::Unsupported {
            command: other.to_string(),
        }),
    }
}

/// Browser-history commands deliberately excluded from this fork.
fn is_browser_command(command: &str) -> bool {
    command == "cookie"
        || command.starts_with("chrome.")
        || command.starts_with("mozilla.")
}

fn attrs(e: &BytesStart) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for attr in e.attributes().flatten() {
        let key = String::from_utf8_lossy(attr.key.local_name().as_ref()).into_owned();
        let value = attr.unescape_value().map(|v| v.into_owned()).unwrap_or_default();
        map.insert(key, value);
    }
    map
}

fn local_name(e: &BytesStart) -> String {
    String::from_utf8_lossy(e.local_name().as_ref()).into_owned()
}

fn local_name_end(e: &quick_xml::events::BytesEnd) -> String {
    String::from_utf8_lossy(e.local_name().as_ref()).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::Action;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<cleaner id="claude">
  <label>Claude</label>
  <description>AI assistant</description>
  <option id="cache">
    <label>Cache</label>
    <description>Delete the cache</description>
    <action command="delete" search="walk.all" path="~/.claude/cache/"/>
  </option>
  <option id="session">
    <label>Session</label>
    <description>Delete the conversation history</description>
    <warning>You would lose past sessions</warning>
    <action command="delete" search="walk.files" path="~/.claude/projects/"/>
  </option>
</cleaner>"#;

    #[test]
    fn parses_sample_cleaner() {
        let c = parse_str(SAMPLE).unwrap();
        assert_eq!(c.id, "claude");
        assert_eq!(c.name, "Claude");
        assert_eq!(c.options.len(), 2);

        let cache = c.option("cache").unwrap();
        assert_eq!(cache.name, "Cache");
        assert_eq!(cache.actions.len(), 1);
        assert!(matches!(cache.actions[0], Action::File(_)));

        let session = c.option("session").unwrap();
        assert_eq!(session.warning.as_deref(), Some("You would lose past sessions"));
        assert!(c.is_usable());
    }

    #[test]
    fn browser_commands_are_dropped() {
        // chrome.*/mozilla.*/cookie are intentionally excluded; an option with only
        // such actions ends up empty.
        let xml = r#"<cleaner id="x"><label>X</label><description>d</description>
        <option id="o"><label>O</label><description>d</description>
        <action command="chrome.history" search="glob" path="~/foo/*.db"/>
        </option></cleaner>"#;
        let c = parse_str(xml).unwrap();
        assert!(c.option("o").unwrap().actions.is_empty());
    }

    #[test]
    fn unknown_command_is_kept_as_unsupported() {
        let xml = r#"<cleaner id="x"><label>X</label><description>d</description>
        <option id="o"><label>O</label><description>d</description>
        <action command="totally.unknown" search="glob" path="~/foo/*"/>
        </option></cleaner>"#;
        let c = parse_str(xml).unwrap();
        assert!(matches!(c.option("o").unwrap().actions[0], Action::Unsupported { .. }));
    }

    #[test]
    fn parses_special_commands() {
        let xml = r#"<cleaner id="x"><label>X</label><description>d</description>
        <option id="o"><label>O</label><description>d</description>
        <action command="sqlite.vacuum" search="glob" path="~/foo/*.db"/>
        <action command="winreg" path="HKCU\Software\Foo" name="Bar"/>
        <action command="json" search="file" path="~/c.json" address="a/b"/>
        </option></cleaner>"#;
        let c = parse_str(xml).unwrap();
        let o = c.option("o").unwrap();
        assert!(matches!(o.actions[0], Action::Vacuum(_)));
        assert!(matches!(o.actions[1], Action::Winreg { .. }));
        assert!(matches!(o.actions[2], Action::Json { .. }));
    }
}
