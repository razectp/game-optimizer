# Game Optimizer (GPC)

Windows app that applies a **Game Performance Contract** to running games.
Never closes games. Large **browsers** (Chrome, Edge, Firefox, …) can be
trimmed to free RAM; FFmpeg, OBS, editors like Cursor, and other apps are
left alone by default.

**Default launch opens the GUI.** Closing or minimizing the window sends the
app to the system tray. Click the tray icon (or *Abrir*) to restore, including
if the window was maximized. Start with Windows is enabled by default.

## Requirements

- Windows 10/11
- **Administrator** recommended for full process-control rights on games

## Install (end users)

No Rust toolchain is required. Download a packaged build from
[GitHub Releases](https://github.com/razectp/game-optimizer/releases):

1. **Setup (recommended):** run `GameOptimizer-0.6.2-setup.exe` and follow the
   setup wizard (language, welcome, license, folder, shortcuts). Over an
   existing install it shows an update page and asks to close the running app.
2. **Portable:** run `GameOptimizer-0.6.2.exe`, or extract
   `GameOptimizer-0.6.2-windows.zip` and install:

```powershell
powershell -ExecutionPolicy Bypass -File .\install.ps1
```

From a source checkout, after a release binary exists (or with `-Build` if you
have Rust):

```powershell
powershell -ExecutionPolicy Bypass -File .\installer\install.ps1
powershell -ExecutionPolicy Bypass -File .\installer\install.ps1 -Build
```

Installs to `%LOCALAPPDATA%\GameOptimizer`, creates Start Menu / Desktop
shortcuts, registers **Apps & features** uninstall, and enables auto-start.
Use `-NoAutostart` to skip the Run key. Uninstall from Apps & features, or:

```powershell
powershell -ExecutionPolicy Bypass -File "$env:LOCALAPPDATA\GameOptimizer\uninstall.ps1"
```

The Inno installer is a full setup wizard (no admin). Portable launches show a
first-run wizard in the app; reopen it with `--setup` or **Mais opções**.
The GUI checks GitHub Releases for updates and can download the setup installer.

```powershell
.\target\release\game_optimizer.exe --setup
```

Maintainers can produce the zip + Inno inputs with `installer\package.ps1`,
then `iscc installer\GameOptimizer.iss`.

## Build / run (developers)

```powershell
cargo build --release

# GUI (default)
.\target\release\game_optimizer.exe
.\target\release\game_optimizer.exe --gui
.\target\release\game_optimizer.exe --setup

# CLI
.\target\release\game_optimizer.exe scan
.\target\release\game_optimizer.exe optimize
.\target\release\game_optimizer.exe optimize --trim-game-memory
.\target\release\game_optimizer.exe watch --interval 45
.\target\release\game_optimizer.exe games
.\target\release\game_optimizer.exe autostart
.\target\release\game_optimizer.exe autostart --on
.\target\release\game_optimizer.exe autostart --off
.\target\release\game_optimizer.exe tunnel
.\target\release\game_optimizer.exe tunnel --ping
.\target\release\game_optimizer.exe tunnel --export-conf .\GameOptTunnel.conf
```

Requires Rust stable 1.85+.

## What GPC does

| Action | Effect |
|--------|--------|
| High priority + priority boost | Scheduler prefers the game |
| Power throttling off | Avoids EcoQoS slowdowns |
| Memory priority | Biases OS to keep game pages resident |
| Residency floor | Soft minimum working set for the game |
| CPU-set / affinity partition | Split cores across multi-instance games |
| Non-game WS trim | Reclaims **browsers only** (Chrome/Edge/Firefox/…) |
| Trim memória do jogo | Optional empty of game working sets |
| Tunelamento VPS (WireGuard) | Roteia exclusivamente as conexões dos jogos pela VPS (zero injeção de DLL, seguro contra anti-cheats) |

## Tunelamento WireGuard (VPS)

Permite acelerar rotas de jogos usando uma VPS própria com WireGuard:
- **Isolamento por PID:** O programa monitora a tabela de sockets do Windows (`iphlpapi.dll`). Quando um jogo abre conexão para um servidor, apenas o tráfego daquele destino é roteado pela VPS. Navegadores, Discord e o resto do sistema continuam na internet normal.
- **Zero injeção de DLL:** Não toca na memória nem nos arquivos do jogo, sendo 100% seguro contra anti-cheats (Vanguard, EAC, BattlEye, Ricochet).
- **Configuração Embutida ou Importação .conf:** Configure pela interface gráfica (`🛡️ Túnel VPS`), importando seu arquivo `.conf` do WireGuard diretamente no app.

The GUI can also clean the **user Temp / INetCache** folders (confirm in the
UI). Shader caches, Prefetch, and Windows Update files are never touched.

## Adding games or apps

Select directly from active running processes in the GUI:
1. Open Game Optimizer and click **⚙️ Gerenciar Jogos & Processos** (or **🎯 Escolher dos Processos em Execução**).
2. The optimizer automatically detects the process and root PID/tree (e.g., detecting `mir4.exe` as root when picking child worker or shipping processes).
3. Click **🎮 Monitorar Jogo**, **🧹 Trim**, or **🛡️ Proteger** with 1 click. Settings are persisted automatically to `%LOCALAPPDATA%\GameOptimizer\config.toml`.

## Verify

```powershell
cargo check
cargo clippy -- -D warnings
cargo fmt --check
cargo test
```
