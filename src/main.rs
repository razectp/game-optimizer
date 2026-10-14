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
use game_optimizer::win_window;

#[derive(Debug, Parser)]
#[command(
    name = "game_optimizer",
    about = "Optimize running games (GUI by default; never closes them).",
    version
)]
struct Cli {
    /// Path to optional config allowlist/settings.
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    /// Force the graphical interface.
    #[arg(long)]
    gui: bool,

    /// Open the first-run setup wizard in the GUI.
    #[arg(long)]
    setup: bool,

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
    /// Manage and inspect the embedded WireGuard VPS game tunnel.
    Tunnel {
        /// Test latency to the VPS endpoint.
        #[arg(long)]
        ping: bool,
        /// Export a standard WireGuard .conf file for the embedded configuration.
        #[arg(long)]
        export_conf: Option<PathBuf>,
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

    let config_path = cli
        .config
        .clone()
        .unwrap_or_else(AppConfig::user_config_path);

    if cli.setup || cli.gui || cli.command.is_none() {
        if !win_window::try_become_single_instance() {
            let _ = win_window::restore_main_window(false);
            return Ok(());
        }
        return gui::run(config_path).map_err(|err| anyhow::anyhow!("GUI failed: {err}"));
    }

    let config = if let Some(path) = &cli.config {
        AppConfig::load_or_default(path)?
    } else {
        AppConfig::load_or_default(&config_path)?
    };
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
                println!("Using built-in defaults (no external config file).");
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
        Commands::Tunnel { ping, export_conf } => {
            let wg = &config.wireguard;
            println!("=== Tunelamento WireGuard (VPS) ===");
            println!(
                "Status configurado: {}",
                if wg.is_configured() {
                    "Sim"
                } else {
                    "Incompleto"
                }
            );
            println!("Habilitado: {}", if wg.enabled { "Sim" } else { "Não" });
            println!(
                "Endpoint: {}",
                if wg.endpoint.is_empty() {
                    "(não definido)"
                } else {
                    &wg.endpoint
                }
            );
            println!("IP do Cliente na VPN: {}", wg.tunnel_ip);
            println!("Gateway VPS: {}", wg.tunnel_gateway);

            if ping {
                println!("\nTestando ping para o endpoint da VPS...");
                match wg.ping_vps(Duration::from_millis(2000)) {
                    Ok(ms) => println!("Sucesso! Latência VPS: {ms}ms"),
                    Err(err) => println!("Falha no teste de ping: {err}"),
                }
            }

            if let Some(path) = export_conf {
                match wg.write_conf_file(&path) {
                    Ok(p) => println!("\nConfiguração salva em: {}", p.display()),
                    Err(err) => eprintln!("Erro ao salvar arquivo de configuração: {err}"),
                }
            }
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
