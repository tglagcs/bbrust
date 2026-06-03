//! Console interface: `--list`, `--preview`, `--clean`.
//!
//! Consumes [`WorkerEvent`]s from the off-thread worker and prints them, so even
//! the CLI exercises the non-blocking architecture.

use std::path::PathBuf;

use crate::cleaner::Backends;
use crate::util::bytes_to_human;
use crate::worker::{self, Selection, WorkerEvent};

pub struct Args {
    /// Explicit `--cleaners-dir` override. `None` means use the embedded default
    /// set plus the user's custom dir.
    pub cleaners_dir: Option<PathBuf>,
    pub command: CliCommand,
}

pub enum CliCommand {
    List,
    Preview(Vec<Selection>),
    Clean(Vec<Selection>),
    Gui,
    Help,
}

/// Parse `std::env::args` (excluding the program name).
pub fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut cleaners_dir: Option<PathBuf> = None;
    let mut command: Option<CliCommand> = None;
    let mut selections = Vec::new();
    let mut mode_clean = false;

    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--cleaners-dir" => {
                i += 1;
                let dir = argv.get(i).ok_or("--cleaners-dir requires a path")?;
                cleaners_dir = Some(PathBuf::from(dir));
            }
            "--list" | "-l" => command = Some(CliCommand::List),
            "--gui" => command = Some(CliCommand::Gui),
            "--help" | "-h" => command = Some(CliCommand::Help),
            "--preview" | "-p" => mode_clean = false,
            "--clean" | "-c" => mode_clean = true,
            token if !token.starts_with('-') => {
                selections.push(parse_selection(token)?);
            }
            other => return Err(format!("unknown option: {other}")),
        }
        i += 1;
    }

    let command = command.unwrap_or_else(|| {
        if selections.is_empty() {
            CliCommand::Gui
        } else if mode_clean {
            CliCommand::Clean(std::mem::take(&mut selections))
        } else {
            CliCommand::Preview(std::mem::take(&mut selections))
        }
    });

    Ok(Args {
        cleaners_dir,
        command,
    })
}

fn parse_selection(token: &str) -> Result<Selection, String> {
    let (cleaner, option) = token
        .split_once('.')
        .ok_or_else(|| format!("expected cleaner.option, got '{token}'"))?;
    Ok(Selection {
        cleaner: cleaner.to_string(),
        option: option.to_string(),
    })
}

/// Build the cleaner registry for a CLI/GUI run. With an explicit
/// `--cleaners-dir`, load only that directory (used by tests with fixtures).
/// Otherwise use the embedded default set plus the user's custom dir.
pub fn load_backends(explicit_dir: &Option<PathBuf>) -> Backends {
    match explicit_dir {
        Some(dir) => Backends::load_from_dir(dir),
        None => Backends::load_default(&[crate::config::custom_cleaners_dir()]),
    }
}

pub fn print_help() {
    println!(
        "bbrust — lightweight BleachBit fork (Windows)\n\n\
         USAGE:\n\
         \x20 bbrust                         launch the GUI (Stage 3)\n\
         \x20 bbrust --list                  list cleaners and options\n\
         \x20 bbrust --preview <c.opt>...    show what would be deleted\n\
         \x20 bbrust --clean <c.opt>...      delete for real\n\n\
         OPTIONS:\n\
         \x20 --cleaners-dir <path>          override the cleaners directory\n\
         \x20 -h, --help                     show this help\n\n\
         EXAMPLE:\n\
         \x20 bbrust --preview google_chrome.cache firefox.cache"
    );
}

/// Run a `--list` request.
pub fn run_list(backends: &Backends) {
    if backends.cleaners.is_empty() {
        println!("(no cleaners loaded)");
        return;
    }
    for cleaner in &backends.cleaners {
        println!("{} ({})", cleaner.id, cleaner.name);
        for opt in &cleaner.options {
            if opt.actions.is_empty() {
                continue;
            }
            println!("  {}.{}\t{}", cleaner.id, opt.id, opt.name);
        }
    }
}

/// Run a preview or clean and print streamed results. Returns the process exit code.
pub fn run_clean(backends: &Backends, selections: &[Selection], really_delete: bool) -> i32 {
    let ops = worker::collect(backends, selections);
    if ops.is_empty() {
        eprintln!("No matching cleaner.option selections found.");
        return 1;
    }

    let running = worker::spawn(ops, really_delete);
    let mut errors = 0u64;

    for event in &running.events {
        match event {
            WorkerEvent::Status(s) => println!("{s}"),
            WorkerEvent::Line(line) => println!("{line}"),
            WorkerEvent::Done {
                total_bytes,
                files,
                special,
                errors: errs,
                aborted,
            } => {
                errors = errs;
                println!();
                let verb = if really_delete {
                    "Disk space recovered"
                } else {
                    "Disk space to be recovered"
                };
                println!("{verb}: {}", bytes_to_human(total_bytes as i64));
                let fverb = if really_delete {
                    "Files deleted"
                } else {
                    "Files to be deleted"
                };
                println!("{fverb}: {files}");
                if special > 0 {
                    println!("Special operations: {special}");
                }
                if errs > 0 {
                    println!("Errors: {errs}");
                }
                if aborted {
                    println!("(aborted)");
                }
            }
            // Progress/ItemSize are for the GUI; ignore in the CLI.
            _ => {}
        }
    }

    let _ = running.handle.join();
    if errors > 0 {
        1
    } else {
        0
    }
}
