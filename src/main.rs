//! bbrust — a lightweight, Windows-only fork of BleachBit written in Rust.
//!
//! Stage 1 delivers the core (CleanerML parser, file actions, off-thread worker)
//! and a working CLI. The GUI (Stage 3) and special actions (Stage 2) plug into
//! the same model and worker.

// In release builds use the Windows GUI subsystem so launching the app does not
// pop up a console window. CLI invocations re-attach to the parent console (see
// `attach_parent_console`). Debug builds keep the console for development.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod actions;
mod category;
mod cleaner;
mod cli;
mod config;
mod gui;
mod i18n;
mod translations_ru;
mod util;
mod worker;

use cli::CliCommand;

/// When started from a terminal (i.e. with arguments), attach to the parent
/// console so `--list`/`--preview`/`--clean` output is visible. No-op when the
/// app is double-clicked (GUI mode).
#[cfg(windows)]
fn attach_parent_console() {
    use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    // Safe: failure (no parent console) is fine and ignored.
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

fn main() {
    #[cfg(windows)]
    if std::env::args_os().len() > 1 {
        attach_parent_console();
    }

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match cli::parse_args(&argv) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}\n");
            cli::print_help();
            std::process::exit(2);
        }
    };

    match args.command {
        CliCommand::Help => {
            cli::print_help();
        }
        CliCommand::List => {
            let backends = cli::load_backends(&args.cleaners_dir);
            cli::run_list(&backends);
        }
        CliCommand::Preview(selections) => {
            let backends = cli::load_backends(&args.cleaners_dir);
            std::process::exit(cli::run_clean(&backends, &selections, false));
        }
        CliCommand::Clean(selections) => {
            let backends = cli::load_backends(&args.cleaners_dir);
            std::process::exit(cli::run_clean(&backends, &selections, true));
        }
        CliCommand::Gui => {
            if let Err(e) = gui::run(args.cleaners_dir) {
                eprintln!("GUI error: {e}");
                std::process::exit(1);
            }
        }
    }
}
