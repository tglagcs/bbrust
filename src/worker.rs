//! The cleaning engine.
//!
//! Unlike BleachBit, which drives a generator from the GTK idle loop on the main
//! thread (the root cause of the UI freezing), this worker runs on its own
//! [`std::thread`] and streams [`WorkerEvent`]s back over a channel. The UI (or
//! CLI) consumes events without ever blocking, and can request abort at any time
//! via a shared [`AtomicBool`].

use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread::JoinHandle;

use crate::actions::{Action, Command};
use crate::cleaner::Backends;
use crate::util::{bytes_to_human, path_size};

/// A user selection: clean `option` within `cleaner`.
#[derive(Debug, Clone)]
pub struct Selection {
    pub cleaner: String,
    pub option: String,
}

/// A selection paired with a cheap clone of its actions. The expensive part —
/// expanding actions into commands, which walks the filesystem — is deferred to
/// the worker thread so the UI thread never blocks.
pub struct OpSpec {
    pub cleaner: String,
    pub option: String,
    pub label: String,
    pub actions: Vec<Action>,
}

/// Messages streamed from the worker thread to the UI/CLI.
#[derive(Debug, Clone)]
pub enum WorkerEvent {
    /// Overall progress in `0.0..=1.0`.
    Progress(f32),
    /// A human-readable status line ("Previewing Firefox.").
    Status(String),
    /// A line of detail for the log pane (already formatted).
    Line(String),
    /// Final size attributed to one option, for the tree display.
    ItemSize {
        cleaner: String,
        option: String,
        size: u64,
    },
    /// The run finished (or was aborted).
    Done {
        total_bytes: u64,
        files: u64,
        special: u64,
        errors: u64,
        aborted: bool,
    },
}

/// Pair each selection with a clone of its actions. Cheap: clones action metadata
/// (no filesystem access), which then runs on the worker thread.
pub fn collect(backends: &Backends, selections: &[Selection]) -> Vec<OpSpec> {
    let mut ops = Vec::new();
    for sel in selections {
        let Some(cleaner) = backends.get(&sel.cleaner) else {
            continue;
        };
        let Some(option) = cleaner.option(&sel.option) else {
            continue;
        };
        ops.push(OpSpec {
            cleaner: sel.cleaner.clone(),
            option: sel.option.clone(),
            label: format!("{} — {}", cleaner.name, option.name),
            actions: option.actions.clone(),
        });
    }
    ops
}

/// Handle to a running worker: event stream, abort switch, and thread join handle.
pub struct RunningWorker {
    pub events: Receiver<WorkerEvent>,
    pub abort: Arc<AtomicBool>,
    pub handle: JoinHandle<()>,
}

/// Spawn the worker thread. `really_delete=false` is a dry-run preview.
pub fn spawn(ops: Vec<OpSpec>, really_delete: bool) -> RunningWorker {
    let (tx, rx) = mpsc::channel();
    let abort = Arc::new(AtomicBool::new(false));
    let abort_thread = Arc::clone(&abort);

    let handle = std::thread::spawn(move || {
        let mut total_bytes: u64 = 0;
        let mut files: u64 = 0;
        let mut special: u64 = 0;
        let mut errors: u64 = 0;
        let mut aborted = false;

        let total_ops = ops.len().max(1);
        for (i, op) in ops.iter().enumerate() {
            if abort_thread.load(Ordering::Relaxed) {
                aborted = true;
                break;
            }
            let _ = tx.send(WorkerEvent::Progress(i as f32 / total_ops as f32));
            let verb = if really_delete { "Cleaning" } else { "Previewing" };
            let _ = tx.send(WorkerEvent::Status(format!("{verb} {}.", op.label)));

            // Expand actions into commands here, on the worker thread (this walks
            // the filesystem and must not run on the UI thread).
            let commands: Vec<Command> =
                op.actions.iter().flat_map(|a| a.commands()).collect();

            let mut op_size: u64 = 0;
            for cmd in &commands {
                if abort_thread.load(Ordering::Relaxed) {
                    aborted = true;
                    break;
                }
                match run_command(cmd, really_delete) {
                    Ok(Some(line)) => {
                        op_size += line.size;
                        total_bytes += line.size;
                        if line.deleted {
                            files += 1;
                        }
                        if line.special {
                            special += 1;
                        }
                        let size_str = bytes_to_human(line.size as i64);
                        let _ = tx.send(WorkerEvent::Line(
                            format!("{} {} {}", line.label, size_str, line.path)
                                .trim_end()
                                .to_string(),
                        ));
                    }
                    Ok(None) => {}
                    Err(e) => {
                        errors += 1;
                        let _ = tx.send(WorkerEvent::Line(format!("Error: {e}")));
                    }
                }
            }
            let _ = tx.send(WorkerEvent::ItemSize {
                cleaner: op.cleaner.clone(),
                option: op.option.clone(),
                size: op_size,
            });
            if aborted {
                break;
            }
        }

        let _ = tx.send(WorkerEvent::Progress(1.0));
        let _ = tx.send(WorkerEvent::Done {
            total_bytes,
            files,
            special,
            errors,
            aborted,
        });
    });

    RunningWorker {
        events: rx,
        abort,
        handle,
    }
}

/// One line of result to surface to the UI/CLI. `deleted` counts toward files,
/// `special` toward special operations (registry, vacuum, etc.).
struct Line {
    size: u64,
    label: String,
    path: String,
    deleted: bool,
    special: bool,
}

/// Run (or preview) a single command. `Ok(None)` means "nothing to report".
fn run_command(cmd: &Command, really_delete: bool) -> Result<Option<Line>, String> {
    match cmd {
        Command::Delete { path, shred } => {
            let size = path_size(path);
            let display = path.display().to_string();
            let verb = if really_delete { "Deleted" } else { "Delete" };
            if really_delete {
                delete_path(path, *shred).map_err(|e| format!("{display}: {e}"))?;
            }
            Ok(Some(Line {
                size,
                label: verb.to_string(),
                path: display,
                deleted: really_delete,
                special: false,
            }))
        }
        Command::Truncate { path } => {
            let size = path_size(path);
            let display = path.display().to_string();
            if really_delete {
                OpenOptions::new()
                    .write(true)
                    .open(path)
                    .and_then(|f| f.set_len(0))
                    .map_err(|e| format!("{display}: {e}"))?;
            }
            Ok(Some(Line {
                size,
                label: "Truncate".to_string(),
                path: display,
                deleted: really_delete,
                special: false,
            }))
        }
        Command::Vacuum { path } => file_mutation(path, really_delete, "Vacuum", |p| {
            crate::actions::special::vacuum(p)
        }),
        Command::Json { path, address } => {
            let address = address.clone();
            file_mutation(path, really_delete, "Clean file", move |p| {
                crate::actions::special::clean_json(p, &address)
            })
        }
        Command::Ini {
            path,
            section,
            parameter,
        } => {
            let section = section.clone();
            let parameter = parameter.clone();
            file_mutation(path, really_delete, "Clean file", move |p| {
                crate::actions::special::clean_ini(p, &section, parameter.as_deref())
            })
        }
        Command::Office { path } => file_mutation(path, really_delete, "Clean file", |p| {
            crate::actions::special::clean_office_registrymodifications(p)
        }),
        Command::Process { cmd, wait } => {
            if really_delete {
                crate::actions::special::run_process(cmd, *wait)?;
            }
            Ok(Some(Line {
                size: 0,
                label: format!("Run external command: {cmd}"),
                path: String::new(),
                deleted: false,
                special: true,
            }))
        }
        Command::Clipboard => {
            #[cfg(windows)]
            if really_delete {
                crate::actions::system::empty_clipboard()?;
            }
            Ok(Some(Line {
                size: 0,
                label: "Clipboard".to_string(),
                path: String::new(),
                deleted: false,
                special: true,
            }))
        }
        Command::RecycleBin => {
            #[cfg(windows)]
            {
                let size = crate::actions::system::recycle_bin_size();
                if really_delete {
                    crate::actions::system::empty_recycle_bin()?;
                }
                Ok(Some(Line {
                    size,
                    label: if really_delete { "Emptied recycle bin" } else { "Empty recycle bin" }
                        .to_string(),
                    path: String::new(),
                    deleted: false,
                    special: true,
                }))
            }
            #[cfg(not(windows))]
            {
                let _ = really_delete;
                Ok(None)
            }
        }
        Command::Winreg { keyname, valuename } => run_winreg(keyname, valuename, really_delete),
        Command::ShellNotify => {
            if really_delete {
                #[cfg(windows)]
                crate::actions::winreg::shell_change_notify();
            }
            Ok(None) // hidden operation; nothing to list
        }
    }
}

/// Perform an in-place file mutation and report the bytes recovered
/// (size before − size after). Previews report nothing, matching BleachBit's
/// `Command.Function` which has no preview for these.
fn file_mutation<F>(
    path: &Path,
    really_delete: bool,
    label: &str,
    op: F,
) -> Result<Option<Line>, String>
where
    F: FnOnce(&Path) -> Result<(), String>,
{
    if !really_delete {
        return Ok(None);
    }
    let oldsize = path_size(path);
    op(path)?;
    let newsize = path_size(path);
    Ok(Some(Line {
        size: oldsize.saturating_sub(newsize),
        label: label.to_string(),
        path: path.display().to_string(),
        deleted: false,
        special: true,
    }))
}

fn run_winreg(
    keyname: &str,
    valuename: &str,
    really_delete: bool,
) -> Result<Option<Line>, String> {
    #[cfg(windows)]
    {
        use crate::actions::winreg;
        let (display, affected) = if !valuename.is_empty() {
            let d = format!("{keyname}<{valuename}>");
            let a = if really_delete {
                winreg::delete_value(keyname, valuename)?
            } else {
                winreg::value_exists(keyname, valuename)
            };
            (d, a)
        } else {
            let a = if really_delete {
                winreg::delete_key(keyname)?
            } else {
                winreg::key_exists(keyname)
            };
            (keyname.to_string(), a)
        };
        if !affected {
            return Ok(None);
        }
        Ok(Some(Line {
            size: 0,
            label: "Delete registry key".to_string(),
            path: display,
            deleted: false,
            special: true,
        }))
    }
    #[cfg(not(windows))]
    {
        let _ = (keyname, valuename, really_delete);
        Ok(None)
    }
}

/// Delete a file, symlink, or (empty) directory. When `shred`, overwrite a
/// regular file's contents with zeros first.
///
/// NOTE: a single zero pass is a placeholder for Stage 2's secure-delete module;
/// it removes the data path but is not a forensic-grade wipe.
fn delete_path(path: &Path, shred: bool) -> std::io::Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    let ft = meta.file_type();

    if ft.is_dir() && !ft.is_symlink() {
        return std::fs::remove_dir(path);
    }

    if shred && ft.is_file() {
        overwrite_zeros(path, meta.len())?;
    }

    // Files and symlinks (including directory symlinks on Windows) go here.
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(_) if ft.is_symlink() => std::fs::remove_dir(path),
        Err(e) => Err(e),
    }
}

fn overwrite_zeros(path: &Path, len: u64) -> std::io::Result<()> {
    let mut f = OpenOptions::new().write(true).open(path)?;
    f.seek(SeekFrom::Start(0))?;
    let zeros = [0u8; 64 * 1024];
    let mut remaining = len;
    while remaining > 0 {
        let chunk = remaining.min(zeros.len() as u64) as usize;
        f.write_all(&zeros[..chunk])?;
        remaining -= chunk as u64;
    }
    f.flush()
}
