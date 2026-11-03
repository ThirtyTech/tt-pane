#![forbid(unsafe_code)]
#![cfg_attr(
    all(target_os = "windows", feature = "gui"),
    windows_subsystem = "windows"
)]

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::{path::PathBuf, process::ExitCode};
use tt_pane::{config::Config, geometry::Action};

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
    /// GNOME Wayland: install the tt-pane Shell extension and register shortcuts.
    #[cfg(target_os = "linux")]
    Gnome {
        #[command(subcommand)]
        command: GnomeCommand,
    },
}

#[cfg(target_os = "linux")]
#[derive(Subcommand)]
enum GnomeCommand {
    /// Install or update the extension and replace tt-pane's GNOME custom shortcuts.
    Setup,
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Some(Command::Apply {
            action,
            window,
            json,
        }) => {
            let result = (|| -> Result<()> {
                let path = cli.config.clone().map_or_else(Config::path, Ok)?;
                let config = Config::load(&path)?;
                tt_pane::positioning::apply(&config, &path, action, window)
            })();
            #[cfg(windows)]
            tt_pane::platform::wait_for_focus_glow();
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
            tt_pane::shortcuts::wayland_bindings(
                &config,
                &compositor,
                &std::env::current_exe()?,
                Some(&std::path::absolute(&path)?)
            )?
        ),
        #[cfg(target_os = "linux")]
        Some(Command::Gnome {
            command: GnomeCommand::Setup,
        }) => println!(
            "{}",
            tt_pane::gnome::setup(&config, &std::env::current_exe()?, &path)?.summary()
        ),
        Some(Command::Apply { .. } | Command::Actions) => unreachable!(),
        command @ (None | Some(Command::Settings)) => {
            #[cfg(feature = "gui")]
            tt_pane::ui::run(config, path, command.is_some())?;
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
    // Attach terminal launches (including no-argument cargo run) so Ctrl+C reaches
    // the GUI process. Explorer launches still do not allocate a console.
    #[cfg(all(target_os = "windows", feature = "gui"))]
    tt_pane::platform::attach_parent_console();
    let cli = Cli::parse();
    #[cfg(all(target_os = "macos", feature = "gui"))]
    let gui = matches!(cli.command, None | Some(Command::Settings));
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error:#}");
            // Opened from Finder there is no terminal to read stderr in.
            #[cfg(all(target_os = "macos", feature = "gui"))]
            if gui && !std::io::IsTerminal::is_terminal(&std::io::stderr()) {
                tt_pane::platform::show_alert("tt-pane could not start", &format!("{error:#}"));
            }
            ExitCode::FAILURE
        }
    }
}
