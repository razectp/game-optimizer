# Changelog

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
