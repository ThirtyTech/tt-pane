#![forbid(unsafe_code)]
#![cfg_attr(
    all(target_os = "windows", feature = "gui"),
    windows_subsystem = "windows"
)]

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::{path::PathBuf, process::ExitCode};
use stile::{config::Config, geometry::Action};

#[derive(Parser)]
#[command(
    version,
    about = "Native window positioning; starts in the system tray without a command"
)]
struct Cli {
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Position the foreground window, or a specific Windows HWND.
    Apply {
        action: Action,
        /// Decimal native HWND (Windows only); avoids relying on foreground focus.
        #[arg(long)]
        window: Option<u64>,
        /// Emit a versioned JSON result on stdout, including execution errors.
        #[arg(long)]
        json: bool,
    },
    /// List supported positioning actions as JSON, without connecting to the desktop.
    Actions,
    /// Open settings immediately instead of starting with only a tray icon.
    Settings,
    /// Print validated configuration (defaults if no file exists).
    Config,
    /// Export native Wayland compositor shortcuts to stdout.
    Export {
        #[arg(value_parser = ["sway", "hyprland"])]
        compositor: String,
    },
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Some(Command::Apply {
            action,
            window,
            json,
        }) => {
            let result = match window {
                Some(handle) => stile::platform::apply_to_window(action, handle),
                None => stile::platform::apply(action),
            };
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "schema_version": 1, "ok": result.is_ok(), "action": action.name(),
                        "window": window, "error": result.as_ref().err().map(|e| format!("{e:#}"))
                    })
                );
            }
            return result;
        }
        Some(Command::Actions) => {
            println!(
                "{}",
                serde_json::json!({"schema_version": 1, "actions": Action::ALL.into_iter()
                .map(|a| serde_json::json!({"id": a.name(), "title": a.label()})).collect::<Vec<_>>()})
            );
            return Ok(());
        }
        _ => {}
    }
    let path = match cli.config {
        Some(path) => path,
        None => Config::path()?,
    };
    let config = Config::load(&path)?;
    match cli.command {
        Some(Command::Config) => print!("{}", toml::to_string_pretty(&config)?),
        Some(Command::Export { compositor }) => print!(
            "{}",
            stile::shortcuts::wayland_bindings(&config, &compositor, &std::env::current_exe()?)?
        ),
        Some(Command::Apply { .. } | Command::Actions) => unreachable!(),
        command @ (None | Some(Command::Settings)) => {
            #[cfg(feature = "gui")]
            stile::ui::run(config, path, command.is_some())?;
            #[cfg(not(feature = "gui"))]
            {
                let _ = command;
                anyhow::bail!("Built without GPUI; use apply, actions, config, or export");
            }
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    // GUI-subsystem builds never allocate a console. CLI invocations can attach to
    // an existing terminal; inherited redirection pipes must be preserved.
    #[cfg(all(target_os = "windows", feature = "gui"))]
    if std::env::args_os().len() > 1 {
        stile::platform::attach_parent_console();
    }
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error:#}");
            ExitCode::FAILURE
        }
    }
}
