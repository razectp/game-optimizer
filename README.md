# Game Optimizer (GPC)

Windows app that applies a **Game Performance Contract** to running games.
Never closes games. Large non-game apps (e.g. Chrome) can be trimmed to free
RAM; editors like Cursor are left alone by default.

**Default launch opens the GUI.** Closing the window minimizes to the system
tray. Start with Windows is enabled by default (installer + first GUI run).

## Requirements

- Windows 10/11
- Rust stable 1.85+ (to build)
- **Administrator** recommended for full process-control rights on games

## Install (end users)

```powershell
cd game_optimizer
powershell -ExecutionPolicy Bypass -File .\installer\install.ps1
```

Installs to `%LOCALAPPDATA%\GameOptimizer`, creates Start Menu / Desktop
shortcuts, and enables auto-start. Use `-NoAutostart` to skip the Run key.

Uninstall:

```powershell
powershell -ExecutionPolicy Bypass -File .\installer\uninstall.ps1
```

## Build / run (developers)

```powershell
cd game_optimizer
cargo build --release

# GUI (default)
.\target\release\game_optimizer.exe
.\target\release\game_optimizer.exe --gui

# CLI
.\target\release\game_optimizer.exe scan
.\target\release\game_optimizer.exe optimize
.\target\release\game_optimizer.exe optimize --trim-game-memory
.\target\release\game_optimizer.exe watch --interval 45
.\target\release\game_optimizer.exe games
.\target\release\game_optimizer.exe autostart
.\target\release\game_optimizer.exe autostart --on
.\target\release\game_optimizer.exe autostart --off
```

## What GPC does

| Action | Effect |
|--------|--------|
| High priority + priority boost | Scheduler prefers the game |
| Power throttling off | Avoids EcoQoS slowdowns |
| Memory priority | Biases OS to keep game pages resident |
| Residency floor | Soft minimum working set for the game |
| CPU-set / affinity partition | Split cores across multi-instance games |
| Non-game WS trim | Reclaims Chrome/Edge/… (configurable) |
| Trim memória do jogo | Optional empty of game working sets |

## Adding a game

Edit `games.toml` (next to the exe after install):

```toml
[games]
names = ["MyGame.exe"]
```

## Verify

```powershell
cargo check
cargo clippy -- -D warnings
cargo fmt --check
cargo test
```
