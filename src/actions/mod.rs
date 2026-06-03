//! Cleaning actions and the commands they produce.
//!
//! Mirrors BleachBit's `Action.py` / `Command.py` split: an [`Action`] is parsed
//! from a CleanerML `<action>` element and expands into zero or more [`Command`]s
//! that the worker can preview or execute.

pub mod file;
pub mod special;
#[cfg(windows)]
pub mod system;
#[cfg(windows)]
pub mod winreg;

use std::path::PathBuf;

pub use file::{FileAction, FileKind, Searcher};

/// A parsed `<action>` element.
#[derive(Debug, Clone)]
pub enum Action {
    /// `delete`, `shred`, `truncate` — operate on files found by a searcher.
    File(FileAction),
    /// `sqlite.vacuum` — compact each matched SQLite database.
    Vacuum(Searcher),
    /// `json` — remove a key (given by `/`-separated `address`) from JSON files.
    Json { searcher: Searcher, address: String },
    /// `ini` — remove a section or a parameter from `.ini` files.
    Ini {
        searcher: Searcher,
        section: String,
        parameter: Option<String>,
    },
    /// `office_registrymodifications` — strip LibreOffice MRU history from the XML.
    Office(Searcher),
    /// `winreg` — delete a registry key, or a single value when `valuename` is set.
    Winreg { keyname: String, valuename: String },
    /// `win.shell.change.notify` — tell Explorer to refresh.
    ShellNotify,
    /// `process` — run an external command.
    Process { cmd: String, wait: bool },
    /// `clipboard.clear` — empty the desktop clipboard (Windows API).
    Clipboard,
    /// `recycle.bin.empty` — empty the recycle bin (Windows API).
    RecycleBin,
    /// A command not yet implemented (e.g. `chrome.history`, `cookie`). Parsed and
    /// remembered so the rest of the cleaner still loads, but produces no commands.
    Unsupported { command: String },
}

impl Action {
    /// Expand this action into concrete commands to preview or run.
    pub fn commands(&self) -> Vec<Command> {
        match self {
            Action::File(fa) => fa.commands(),
            Action::Vacuum(s) => s
                .find()
                .into_iter()
                .map(|path| Command::Vacuum { path })
                .collect(),
            Action::Json { searcher, address } => searcher
                .find()
                .into_iter()
                .map(|path| Command::Json {
                    path,
                    address: address.clone(),
                })
                .collect(),
            Action::Ini {
                searcher,
                section,
                parameter,
            } => searcher
                .find()
                .into_iter()
                .map(|path| Command::Ini {
                    path,
                    section: section.clone(),
                    parameter: parameter.clone(),
                })
                .collect(),
            Action::Office(s) => s
                .find()
                .into_iter()
                .map(|path| Command::Office { path })
                .collect(),
            Action::Winreg { keyname, valuename } => vec![Command::Winreg {
                keyname: keyname.clone(),
                valuename: valuename.clone(),
            }],
            Action::ShellNotify => vec![Command::ShellNotify],
            Action::Process { cmd, wait } => vec![Command::Process {
                cmd: cmd.clone(),
                wait: *wait,
            }],
            Action::Clipboard => vec![Command::Clipboard],
            Action::RecycleBin => vec![Command::RecycleBin],
            Action::Unsupported { .. } => Vec::new(),
        }
    }
}

/// A single unit of work the worker can preview (estimate size) or execute.
#[derive(Debug, Clone)]
pub enum Command {
    /// Delete a file, directory, or symlink. `shred` overwrites first.
    Delete { path: PathBuf, shred: bool },
    /// Truncate a file to zero length without removing it.
    Truncate { path: PathBuf },
    /// `VACUUM` a SQLite database; recovered bytes = size before − size after.
    Vacuum { path: PathBuf },
    /// Remove a JSON key addressed by a `/`-separated path.
    Json { path: PathBuf, address: String },
    /// Remove an `.ini` section, or a single parameter within it.
    Ini {
        path: PathBuf,
        section: String,
        parameter: Option<String>,
    },
    /// Strip LibreOffice MRU history entries from `registrymodifications.xcu`.
    Office { path: PathBuf },
    /// Delete a registry key, or one value when `valuename` is non-empty.
    Winreg { keyname: String, valuename: String },
    /// Notify the Windows shell of changes.
    ShellNotify,
    /// Run an external command line.
    Process { cmd: String, wait: bool },
    /// Empty the desktop clipboard.
    Clipboard,
    /// Empty the recycle bin on all drives.
    RecycleBin,
}
