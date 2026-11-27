# Game Optimizer (GPC)

Game optimizer and WireGuard tunnel for Windows and Linux.
Never closes games. Large **browsers** (Chrome, Edge, Firefox, …) can be
trimmed to free RAM; FFmpeg, OBS, editors like Cursor, and other apps are
left alone by default.

**Default launch opens the GUI.** On Windows, closing or minimizing the window sends the
app to the system tray. Linux desktops with StatusNotifierItem support the same
behavior. Click the tray icon or *Abrir* to restore the window. Start with the
desktop session is enabled by default.

## Linux e Proton

O Game Optimizer tem interface nativa no Linux e detecta o MIR4 (`Mir4G.exe`)
e outros jogos Proton já abertos. Jogos em bibliotecas Steam são detectados pelo
caminho de instalação. Para outros launchers, cadastre o nome do executável pela
interface se o jogo ainda não estiver na lista.

### Instalar no Linux Mint ou Ubuntu

Em uma instalação Linux limpa (Ubuntu, Linux Mint ou Debian 64 bits), instale diretamente a partir do último release do GitHub com um comando:

```bash
curl -fsSL https://github.com/razectp/game-optimizer/releases/latest/download/install-linux.sh | bash
```

Ou baixe o pacote `game-optimizer_0.6.7-1_amd64.deb` em [GitHub Releases](https://github.com/razectp/game-optimizer/releases) e instale com o instalador gráfico de pacotes ou via terminal (`bash install-linux.sh ./game-optimizer_0.6.7-1_amd64.deb`). O instalador solicita autorização de administrador (`sudo`) e resolve todas as dependências automaticamente pelo APT. Abra **Game Optimizer** pelo menu de aplicativos após a instalação.

O pacote autoriza o componente de rede pelo Polkit para membros do grupo
`sudo` em sessões locais ativas. A interface e os macros rodam na conta que
abriu o programa. Em ambientes com bandeja StatusNotifierItem, use **Bandeja**
ou o botão de minimizar. O **X** também recolhe a janela. Para encerrar,
escolha **Sair** no menu da bandeja. **Iniciar ao entrar na sessão** abre a
interface recolhida quando esse recurso está disponível.

Para gerar um pacote a partir do código, use `bash installer/package-linux.sh`.
O pacote pronto é gravado em `dist/`.

### Compilar e detectar

No Linux Mint, Ubuntu e Debian, instale as ferramentas do túnel e use Rust 1.95
ou mais recente. A interface usa OpenGL via X11 ou Wayland.

```sh
sudo apt install build-essential pkg-config libgl1 libegl1 libxkbcommon0 iproute2 nftables iputils-ping
cargo build --release --locked --bin game_optimizer
./target/release/game_optimizer scan
./target/release/game_optimizer
```

A interface roda com seu usuário. A configuração padrão fica em
`${XDG_CONFIG_HOME:-$HOME/.config}/game-optimizer/config.toml`. No Linux,
o sistema operacional gerencia a prioridade dos processos.

Em uma sessão **X11**, abra **Hotkeys** no cartão do jogo para configurar as
teclas e os modos sequencial ou paralelo. Use **F8** (ou o atalho escolhido)
dentro do jogo para iniciar ou pausar. O macro pode continuar em segundo plano. No Wine/Proton x86-64, o envio em
segundo plano usa mensagens Win32 para a janela do jogo dentro do mesmo prefixo.

### Tunelar jogos detectados

Na interface, importe o arquivo WireGuard da VPS e habilite o túnel no perfil.
O túnel é iniciado automaticamente ao abrir o Game Optimizer com um perfil
habilitado. Ao executar um binário fora do pacote, autorize a alteração de rede
na janela do Linux. A VPS precisa encaminhar o tráfego IPv4 do cliente para a
internet.

O Game Optimizer vincula ao túnel os processos reconhecidos como jogos,
inclusive os já abertos pelo Proton fora do Steam. Abra normalmente pelo
launcher ou atalho de sua escolha. O aplicativo usa cgroup v2, systemd,
nftables e sua própria interface WireGuard no kernel. Se houver outra conexão
WireGuard com a mesma VPS, use um perfil de cliente distinto para os jogos.

Use **Ativar túnel** ou **Desativar túnel** no cartão para trocar a rota da
instância atual. A troca solicita a reconexão das sessões TCP elegíveis do
jogo. O cliente do jogo decide quando e como se reconectar. Novos sockets
TCP e UDP IPv4 elegíveis seguem a rota escolhida quando o túnel está ativo.
Sockets UDP já abertos podem precisar ser recriados. O tráfego dos demais processos, o IPv6,
o DNS, as portas web e a rede local seguem pela rota normal.
Se a VPS deixar de responder, novas conexões voltam à rota normal. Uma sessão
de jogo em andamento pode precisar se reconectar.

## Windows requirements

- Windows 10/11
- **Administrator authorization required** to install, uninstall, and run the app

## Install (end users)

No Rust toolchain is required. Download a packaged build from
[GitHub Releases](https://github.com/razectp/game-optimizer/releases):

1. **Setup (recommended):** run `GameOptimizer-0.6.7-setup.exe`, approve the
   Windows administrator prompt, and follow the setup wizard (language, welcome,
   license, folder, shortcuts). The app and preferences remain in the current
   user's `%LOCALAPPDATA%`. If you enter credentials for a different Windows
   account, per-user settings and shortcuts may apply to that account instead. Over an
   existing install it shows an update page and asks to close the running app.
2. **Portable:** extract `GameOptimizer-0.6.7-windows.zip` and install:

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
window_width = 1040
window_height = 680
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
