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
- **Administrator authorization required** to install, uninstall, and run the app

## Install (end users)

No Rust toolchain is required. Download a packaged build from
[GitHub Releases](https://github.com/razectp/game-optimizer/releases):

1. **Setup (recommended):** run `GameOptimizer-0.6.5-setup.exe`, approve the
   Windows administrator prompt, and follow the setup wizard (language, welcome,
   license, folder, shortcuts). The app and preferences remain in the current
   user's `%LOCALAPPDATA%`. If you enter credentials for a different Windows
   account, per-user settings and shortcuts may apply to that account instead. Over an
   existing install it shows an update page and asks to close the running app.
2. **Portable:** extract `GameOptimizer-0.6.5-windows.zip` and install:

```powershell
powershell -ExecutionPolicy Bypass -File .\install.ps1
```

From a source checkout, after a release binary exists (or with `-Build` if you
have Rust):

```powershell
powershell -ExecutionPolicy Bypass -File .\installer\install.ps1
powershell -ExecutionPolicy Bypass -File .\installer\install.ps1 -Build
```

The portable install script also requires an elevated PowerShell session.
Installs to `%LOCALAPPDATA%\GameOptimizer`, creates Start Menu / Desktop
shortcuts, registers **Apps & features** uninstall, and enables auto-start.
Use `-NoAutostart` to skip the Run key. Uninstall from Apps & features, or:

```powershell
powershell -ExecutionPolicy Bypass -File "$env:LOCALAPPDATA\GameOptimizer\uninstall.ps1"
```

The setup executable has a language, license, install-folder, and shortcut wizard.
The app itself does not currently show a first-run onboarding wizard or an
in-app updater. Settings are stored in `%LOCALAPPDATA%\GameOptimizer\config.toml`;
upgrades and uninstall preserve that profile and unrelated files.

Windows requests administrator authorization (UAC) whenever the app is opened
from an unelevated session. Canceling that prompt leaves the app closed. When
launching the app at the end of Setup, it inherits the authorization already
granted to the installer.

Start with Windows remains optional and per-user. Its startup entry requests
normal UAC authorization for the app at sign-in; it does not bypass UAC or create
an elevated scheduled task. Command-line users must open an administrator
terminal before running the executable directly.

Maintainers can run `installer\package.ps1` to build the isolated release binary
under `target\package`, stage the allowlisted payload in `dist\GameOptimizer`,
and produce the ZIP. To build the setup, compile `installer\GameOptimizer.iss`
with Inno Setup 6 (`ISCC.exe installer\GameOptimizer.iss`). Both packages use
the supplied, unmodified `wintun.dll`; packaging refuses to proceed without it.

## Build / run (developers)

```powershell
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
.\target\release\game_optimizer.exe tunnel
.\target\release\game_optimizer.exe tunnel --ping
.\target\release\game_optimizer.exe tunnel --export-conf .\GameOptTunnel.conf
```

Requires Rust stable 1.95+, the Windows MSVC toolchain, and the Windows SDK.
Only the application binary requires elevation; the unit-test harness does not.

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
| Tunelamento VPS (WireGuard) | Encaminha destinos de rede associados ao jogo pela VPS, conforme configuração; não injeta código no processo do jogo |

## Tunelamento WireGuard (VPS)

Permite acelerar rotas de jogos usando uma VPS própria com WireGuard:
- **Seleção de destinos:** O programa observa conexões associadas aos processos de jogo e adiciona rotas do Windows para destinos selecionados. Isso não equivale a isolamento completo por processo; o comportamento depende da tabela de rotas e da configuração da VPS.
- **Sem injeção no processo do jogo:** O túnel não injeta código no processo do jogo. Isso não garante compatibilidade com todo anti-cheat; consulte as regras do jogo e do provedor.
- **Configuração Embutida ou Importação .conf:** Configure pela interface gráfica (`🛡️ Túnel VPS`), importando seu arquivo `.conf` do WireGuard diretamente no app.

The project contains cache-cleanup support, but it is not currently exposed as
a connected GUI action.

## Saved GUI preferences

The optional `[gui]` table in `%LOCALAPPDATA%\GameOptimizer\config.toml` stores
the process-watch interval and enabled state, plus normal window dimensions and
maximized state. For example:

```toml
[gui]
watch_interval_secs = 45
watch_enabled = false
window_width = 880
window_height = 560
window_maximized = false
```

The watch interval is normalized to 15–120 seconds. Existing configuration
files without `[gui]` continue to load with defaults; when watch is enabled, the
GUI resumes it at startup.

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
