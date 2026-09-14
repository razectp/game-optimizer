//! Entry point: GUI by default; CLI when a subcommand is given.

#![cfg_attr(windows, windows_subsystem = "windows")]

use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use clap::{Parser, Subcommand};
use game_optimizer::autostart;
use game_optimizer::config::AppConfig;
use game_optimizer::gui;
use game_optimizer::optimize::{optimize_system, OptimizeRequest};
use game_optimizer::report::render_report;

#[derive(Debug, Parser)]
#[command(
    name = "game_optimizer",
    about = "Optimize running games (GUI by default; never closes them).",
    version
)]
struct Cli {
    /// Path to optional games.toml allowlist/settings.
    #[arg(long, global = true, default_value = "games.toml")]
    config: PathBuf,

    /// Force the graphical interface.
    #[arg(long)]
    gui: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// List detected games and planned actions without changing anything.
    Scan,
    /// Apply GPC optimizations to games only.
    Optimize {
        /// Plan actions but do not call Windows APIs.
        #[arg(long)]
        dry_run: bool,
        /// Empty each game working set (may hitch when pages fault back in).
        #[arg(long)]
        trim_game_memory: bool,
    },
    /// Re-apply game-only optimizations on an interval while games stay open.
    Watch {
        /// Seconds between passes.
        #[arg(long, default_value_t = 45)]
        interval: u64,
        /// Plan only.
        #[arg(long)]
        dry_run: bool,
        /// Empty each game working set each pass (may hitch).
        #[arg(long)]
        trim_game_memory: bool,
    },
    /// Print the effective game allowlist.
    Games,
    /// Enable or disable start-with-Windows (Registry Run key).
    Autostart {
        /// Turn auto-start on.
        #[arg(long, conflicts_with = "off")]
        on: bool,
        /// Turn auto-start off.
        #[arg(long)]
        off: bool,
    },
}

fn apply_cli_overrides(mut config: AppConfig, trim_game_memory: bool) -> AppConfig {
    if trim_game_memory {
        config.trim_game_memory = true;
    }
    config
}

fn main() -> anyhow::Result<()> {
    attach_parent_console();
    let cli = Cli::parse();

    if cli.gui || cli.command.is_none() {
        return gui::run(cli.config).map_err(|err| anyhow::anyhow!("GUI failed: {err}"));
    }

    let config = AppConfig::load_or_default(&cli.config)?;
    let command = cli
        .command
        .expect("subcommand present when not launching GUI");

    match command {
        Commands::Scan => {
            let report = optimize_system(&OptimizeRequest {
                config,
                dry_run: true,
            });
            println!("{}", render_report(&report));
        }
        Commands::Optimize {
            dry_run,
            trim_game_memory,
        } => {
            let config = apply_cli_overrides(config, trim_game_memory);
            let report = optimize_system(&OptimizeRequest { config, dry_run });
            println!("{}", render_report(&report));
            if report.games.is_empty() {
                anyhow::bail!("no games detected; refusing to optimize non-game workload");
            }
        }
        Commands::Watch {
            interval,
            dry_run,
            trim_game_memory,
        } => {
            if interval == 0 {
                anyhow::bail!("--interval must be >= 1");
            }
            let config = apply_cli_overrides(config, trim_game_memory);
            println!("Watching every {interval}s (Ctrl+C to stop). Games are never closed.");
            loop {
                let report = optimize_system(&OptimizeRequest {
                    config: config.clone(),
                    dry_run,
                });
                println!("{}", render_report(&report));
                if report.games.is_empty() {
                    println!("No games detected this pass; waiting...");
                }
                thread::sleep(Duration::from_secs(interval));
            }
        }
        Commands::Games => {
            println!("Effective game process names (normalized):");
            for name in &config.game_names {
                println!("  {name}");
            }
            if let Some(path) = &config.source_path {
                println!("Loaded from: {}", path.display());
            } else {
                println!("Using built-in defaults (no games.toml loaded).");
            }
        }
        Commands::Autostart { on, off } => {
            if !on && !off {
                let enabled = autostart::is_enabled();
                println!(
                    "Auto-start is {}",
                    if enabled { "enabled" } else { "disabled" }
                );
                return Ok(());
            }
            autostart::set_enabled(on)?;
            println!("Auto-start {}", if on { "enabled" } else { "disabled" });
        }
    }

    Ok(())
}

fn attach_parent_console() {
    #[cfg(windows)]
    {
        use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
        // SAFETY: best-effort attach so CLI/help still print from a terminal.
        let _ = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
    }
}
