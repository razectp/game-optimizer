# Game Optimizer — Specification

## Goal

Windows end-user app that improves responsiveness of **running games only**,
without terminating them. Ships a Portuguese GUI (default), system tray,
installer with Start-with-Windows **on by default**, and a CLI for power users.
Works for any detected game (allowlist, `games.toml`, or common install paths),
including multi-instance sessions of the same title.

## Technology: Game Performance Contract (GPC)

For each detected game identity, GPC builds a per-PID contract that:

1. Raises scheduling priority and disables EcoQoS / execution-speed throttling
2. Raises memory priority (resident bias)
3. Sets a soft working-set floor so Windows is less eager to page the game out
4. On hybrid CPUs, partitions **performance** CPU sets across same-name
   instances so they stop fighting over the same cores; on homogeneous CPUs,
   falls back to disjoint affinity masks
5. Optionally trims large non-game working sets (browsers) and, if requested,
   game working sets (`--trim-game-memory` / GUI toggle)

## Product surface

| Surface | Behavior |
|---------|----------|
| Default launch (no args) / `--gui` | Native GUI; close/minimize hides to tray |
| Tray | Click, double-click, or menu restores; maximized state is preserved; Quit exits |
| Autostart | Enabled by default (installer + first GUI run); toggle in GUI |
| CLI subcommands | `scan`, `optimize`, `watch`, `games`, `autostart` |
| Installer | Per-user `%LOCALAPPDATA%\GameOptimizer`; PowerShell + Inno Setup; Apps & features |
| Temp cleanup | Optional, confirmed GUI action on user Temp / INetCache only |

## Non-goals

- Closing, killing, suspending, or injecting into game processes
- Forcing game working-set MiB down by default (pages the game out → more stutter)
- Overclocking, GPU driver changes, or cheat/anti-cheat bypass
- Deleting shader caches, Prefetch, or Windows Update files
- Cross-platform support (Windows only)

## Acceptance criteria

1. Detects games via built-in allowlist + `games.toml` + install-path heuristics.
2. `optimize` / `watch` / GUI apply GPC to matched game PIDs; optional non-game reclaim.
3. Dual same-name instances receive disjoint CPU slices when enough cores exist.
4. Never terminates processes.
5. GUI scan reports games and reclaim candidates without a noisy activity log.
6. GUI is usable for non-technical users (PT-BR labels); minimize/close → tray;
   restore from tray returns visibility, focus, and maximized state when it was maximized.
7. End-user installer does not require Rust; registers uninstall; Start with Windows
   is on by default (`-NoAutostart` to skip).
8. Optional Temp cleanup only targets allowlisted user cache folders and refuses
   shader/OS-critical paths.
9. `cargo check`, `cargo clippy -- -D warnings`, `cargo fmt --check`, `cargo test` pass.
