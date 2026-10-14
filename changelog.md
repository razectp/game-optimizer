# Changelog

## 0.6.1 — 2026-10-14

- **Zero External Configuration Dependency**: Compiles and runs out of the box with safe
  built-in defaults (`AppConfig::default()`). No external `games.toml` file is required on disk.
- **Interactive Running Process Picker & Root Process Tree Detection**: Users can select
  directly from active running processes in the GUI without navigating file explorer directories.
  The optimizer automatically resolves parent process chains to detect the root game process
  and root PID (e.g. recognizing the root launcher and shipping binaries), allowing 1-click
  monitoring, memory trim, or protection.
- **Direct WireGuard Tunnel Import (.conf) in GUI**: Users can import standard
  WireGuard `.conf` configuration files directly via a native Windows file dialog or paste
  the text into the UI, identical to the official WireGuard client.
- **Secure Per-User Profile Storage**: All user settings and imported WireGuard tunnels
  are persisted securely in `%LOCALAPPDATA%\GameOptimizer\config.toml`.
- **Granular Memory Trim & Protection Controls**: Users can configure game-only trim,
  background trim thresholds (in MB), custom reclaim process lists, and a protected "never trim"
  whitelist (Discord, OBS, stream software, IDEs) directly in the user interface.

## 0.6.0 — 2026-10-14

- **Autonomous Embedded WireGuard Engine**: Fully embedded userspace WireGuard
  powered by `boringtun` and `wintun.dll` (`GameOptimizerTun`). Operates
  autonomously without requiring external WireGuard client installation, background
  services, or any DLL injection.
- **Isolated Per-PID Game Routing**: Dynamic socket inspection monitors active
  connections of detected game PIDs (e.g. MIR4 / port 33000), dynamically injecting
  host routing table entries (`/32`) directed to the WireGuard TUN adapter without
  capturing other system traffic or interfering with external WireGuard instances.
  Routes are automatically cleanly removed when games terminate.
- **Embedded Default Configuration**: Direct WireGuard configuration pre-embedded
  with UI modal to configure or test VPS ping latency.
- **Pure GUI Subsystem**: Binary built with `windows_subsystem = "windows"` and
  dynamic `AttachConsole` fallback to completely eliminate the background command
  prompt console window while preserving CLI functionality when launched from a terminal.
- **Strict Game-Only Memory Trim**: Memory trimming is focused exclusively on
  detected games when enabled (`trim_game_memory`), with background application memory
  trim disabled (`trim_non_game_memory = false`).
- **Tray & Window Restoration Fixes**:
  - Wired `TrayIconEvent::set_event_handler` and `MenuEvent::set_event_handler` to
    immediately awake winit's hidden event loop via `EventLoopProxy` (`request_repaint()`),
    resolving the issue where clicking "Abrir" or the tray icon failed to restore the window.
  - Left-clicking the tray icon restores the window cleanly without opening the popup menu
    (`with_menu_on_left_click(false)`).
  - Right-clicking the tray icon exclusively opens the context menu (`with_menu_on_right_click(true)`).
  - Restoration commands issue `Visible(true)`, `Minimized(false)`, and `Focus`.

## 0.5.3 — 2026-09-16

- Fix Setup crash on upgrade: `Runtime error (at …): Type Mismatch` when
  the wizard filled the welcome text. Version placeholders are now replaced
  with `StringChangeEx` instead of passing a Pascal array into the format
  helper (that type is rejected at runtime on Inno Setup 6.7).

## 0.5.2 — 2026-09-16

- Setup detects a previous install (Inno or PowerShell) and shows an **update**
  welcome (old version → new). Reinstalling the same or an older build asks
  first.
- If Game Optimizer is running (including the tray), Setup **asks** before
  closing it. Closing the window is not enough — the app hides to the tray —
  so the prompt is explicit. Same question on uninstall.
- Non-game memory trim is **browsers only** (Chrome, Edge, Firefox, …). FFmpeg,
  OBS, Premiere and other large apps are never emptied — `EmptyWorkingSet` on
  an encoder pages its RAM to disk and can stall video export.

## 0.5.1 — 2026-09-16

- Window layout scrolls when options expand, so you no longer have to drag the
  window taller to reach buttons.
- Upgrading over an old install no longer starts two copies at login: autostart
  is HKCU-only, leftover Run keys / Startup shortcuts are removed, and a
  single-instance mutex reuses the window that is already open.
- In-app update check (banner + **Verificar atualização**) downloads the setup
  installer from GitHub Releases when a newer version exists.

## 0.5.0 — 2026-09-14

- Setup wizard: Inno installer now walks through language, welcome, summary,
  license, install folder, desktop shortcut, and start-with-Windows.
- First-run wizard in the GUI (portable builds and `game_optimizer --setup`);
  skipped after the Inno installer, which writes `.setup-wizard-complete`.

## 0.4.0 — 2026-09-13

- Release packaging: per-user PowerShell installer (no Rust required), Inno Setup
  script, Apps & features registration, version/icon/manifest resources.
- Friendlier Portuguese GUI: warmer layout, no activity-log noise, quieter status.
- Tray restore keeps maximized state via `SetWindowPlacement` (not `SW_SHOW`) plus
  an event pump so the window comes back after hide-to-tray.
- Optional user Temp / INetCache cleanup (confirmed in UI). Shader caches, Prefetch,
  and Windows Update folders are never touched.
- Watch loop no longer blocks Stop; tray left-click restores the window.
- Autostart opt-out is sticky (installer/GUI write a first-run marker).
- Temp cleanup refuses `%WINDIR%` fallback roots, not only named system subfolders.
- Windows release `.exe` uses the GUI subsystem (no console flash; CLI still
  prints when launched from a terminal).

## 0.3.0 — 2026-09-13

- End-user GUI (Portuguese) opens by default; CLI remains via subcommands.
- Minimize/close hides to the system tray; double-click or menu restores.
- Start with Windows enabled by default (installer + first GUI run); toggle in UI.
- PowerShell installer/uninstaller under `installer/`.
- Denser control-panel GUI: compact header, filled activity area, amber theme.

## 0.2.3 — 2026-09-13

- Generalize product copy and docs for all games (no MIR4-specific framing).
- MIR4 and other titles remain in the built-in detection allowlist.

## 0.2.2 — 2026-09-13

- Restore non-game working-set trim (Chrome/Edge/…) by default
  (`trim_non_game_memory = true`); Cursor/VS Code stay excluded.
- GPC game path unchanged; optional `--trim-game-memory` still available.

## 0.2.1 — 2026-09-13

- Fix `memory_priority` failure (`0x80070057`): used wrong
  `ProcessMemoryPriority` class (39 → 0) and official
  `MEMORY_PRIORITY_INFORMATION`.
- Add optional game-only working-set trim: `--trim-game-memory` /
  `settings.trim_game_memory` (skips residency floor when enabled).

## 0.2.0 — 2026-09-13

- Introduces **Game Performance Contract (GPC)**: game-only planner.
- Dual same-name instances get disjoint CPU slices.
- Soft residency floor + memory priority for games.

## 0.1.0 — 2026-09-13

- Initial Windows Rust CLI for game detection and performance optimization.
