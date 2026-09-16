# Changelog

## 0.5.2 — 2026-09-16

- Setup detects a previous install (Inno or PowerShell) and shows an **update**
  welcome (old version → new). Reinstalling the same or an older build asks
  first.
- If Game Optimizer is running (including the tray), Setup **asks** before
  closing it. Closing the window is not enough — the app hides to the tray —
  so the prompt is explicit. Same question on uninstall.

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
