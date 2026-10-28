# Changelog

## 0.6.5 — 2026-10-28

- **Automação de Hotkeys & Macros Autônomas por Janela (Multi-client/Multi-instância)**:
  - Sistema de automação de teclas operando de forma autônoma e concorrente por processo (PID) e janela (`HWND`), perfeito para múltiplos clientes abertos do mesmo jogo sem interferência cruzada.
  - **Modos de Execução**: Suporte aos modos **Independente** (disparo contínuo por intervalos próprios de cada tecla) e **Sequencial** (execução combo passo a passo 1-a-1 com intervalo configurável de ciclo entre rodadas completas).
  - **Atalho de Alternância Rápida (Toggle Hotkey)**: Atalho configurável por jogo (padrão `F8`), com detecção global não-bloqueante via `GetAsyncKeyState` que atua exclusivamente sobre a janela do jogo que estiver em foco naquele instante.
  - **Envio Direto em Background**: Disparo de mensagens de baixo nível da API Win32 (`PostMessageW` com `WM_KEYDOWN` e `WM_KEYUP`, scancodes e lparam precisos), permitindo automação contínua mesmo quando a janela estiver em segundo plano.
  - **Detecção Inteligente de Janelas de Jogo**: Enumeração dinâmica de janelas filhas e seleção da janela ativa com dimensões reais de renderização, evitando bloqueios ou envio de teclas para launchers e telas de splash invisíveis.
  - **Persistência Atômica & Interface Gráfica**: Perfis de macros salvos em `config.toml` sob `[macros.<game>]`, com controles reativos na GUI para adicionar/remover regras, ajustar tempos de pressionamento/espera, testar teclas individualmente e acompanhar status em tempo real por PID.

## 0.6.4 — 2026-10-14

- Installer and PowerShell install/uninstall require administrator authorization while keeping files and preferences in the current Windows profile; Start-with-Windows is configured through the app CLI.
- The application requires administrator authorization on every unelevated launch. Startup requests normal UAC consent, while the post-install launch inherits Setup's elevated token; test harnesses remain unelevated.
- Distribution now includes the required `wintun.dll` in both the portable ZIP and setup; packaging fails early if the supplied DLL is missing.
- Setup version is taken from the built executable, and uninstall removes app-owned binaries while preserving user preferences and unrelated files.
- Updated Windows CI to format, lint, and test on the platform targeted by the application.
- GUI watch interval/enabled state and normal window geometry are persisted in the optional `[gui]` config table; enabled watching resumes on launch.
- Process lists use independent scrolling, including wheel containment at list boundaries; compact layouts keep controls accessible in smaller windows.
- Preferences are saved atomically by the background worker, and process enumeration no longer blocks the interface. Exiting flushes pending changes and stops the tunnel cleanly.

## 0.6.3 — 2026-10-14

- **Cluster Subnet Pre-Routing (/24) & Zero-Disconnect World Loading**:
  - Automatically pre-routes the game server cluster `/24` subnet (`A.B.C.0/24`) alongside the host route (`A.B.C.D/32`). Subsequent connections to world servers, map instances, or loading ports (e.g. MIR4 port 31000/32000) are already present in the Windows routing table from packet #0 (the very first TCP SYN packet).
  - Protected in-flight game loading sockets from violent TCP resets: restricted `SetTcpEntry` socket migration exclusively to the initial pass when migrating games that were already running before the tunnel started (`initial_migration_done`).
  - Completely eliminated the game disconnect and freeze when transitioning from character select to loading into the game world.

## 0.6.2 — 2026-10-14

- **Instant Game Socket Routing & Anti-Freeze Webview/OAuth Protection**:
  - Excluded web authentication, OAuth, and CDN ports (`80`, `443`, `8080`, `8443`, `53`) from tunnel interception. Embedded login webviews (Google OAuth, WeMade Auth, Steam, Epic) now establish directly and smoothly without interruptions or "Reconnect" errors.
  - Eliminated disruptive mid-flight TCP socket resets (`SetTcpEntry`) on newly added routes. Connections transition cleanly without aborting active HTTP/TLS handshakes.
  - Accelerated active game socket sync from 2000ms down to 150ms polling intervals. Real game multiplayer and world server ports (e.g., MIR4 port 33000/33001) are routed virtually instantaneously on connection start.
  - Automatic cleanup of stale or orphan gateway routes on application launch and shutdown, preventing routing conflicts from previous abnormal exits.
  - Explicitly excluded the WireGuard VPS endpoint IP from tunnel routing to guarantee rock-solid connectivity.
- **Ping-Independent Connection Validation & Elimination of Reconnect Cycles**:
  - Eliminated aggressive 8-second silence timeout that caused constant false-positive "connection failure" cycles on hosts that block ICMP ping or during quiet game periods.
  - Standardized connection health to WireGuard's RFC-standard 90s `REKEY_ATTEMPT_TIME` window and expanded initial handshake window to 25s.
  - Normalized NAT persistent keepalive to 25s without 2..4s over-clamping, eliminating handshake renegotiation loops.
  - Completely eliminated `reset_tcp_connection` from fallback: routing tables are cleanly cleared without killing existing game TCP sockets.
  - Reassured users in GUI ping diagnostics that ICMP drops by host firewalls are normal and WireGuard UDP operates completely independently.
- **Silent Background Execution & Total Elimination of Flashing Console Prompts**:
  - Applied `CREATE_NO_WINDOW` (`0x08000000`) flags to all `reg.exe` calls in autostart duplicate removal and helper processes.
  - Completely eliminated the burst of black command prompt windows ("um monte de prompt") that briefly flashed on screen when opening the application.
  - Enforced `CREATE_NO_WINDOW` across all updater `curl.exe` and shell routines.

## 0.6.1 — 2026-10-14

- **VPN Failure Detection & Automatic Zero-Drop Failover Bypass**:
  - Implemented continuous, non-blocking health monitoring of the WireGuard tunnel (`TunnelHealth`: `Idle`, `Connecting`, `Healthy`, `Degraded`).
  - Active keepalive heartbeat (3s) with BoringTun ensures packet dropouts or VPS unreachability are detected within 6–8 seconds.
  - Automatic Instant Fallback: If connection to the VPS fails or drops, all `/32` host routes are immediately purged from the Windows routing table, allowing game packets to instantly fall back to the default ISP gateway without dropping the game client.
  - Proactive TCP Connection Reset: Active TCP connections are immediately reset via `SetTcpEntry` (`MIB_TCP_STATE_DELETE_TCB`) upon failover, forcing game clients to instantly reconnect through the direct network rather than hanging or freezing indefinitely.
  - Seamless Recovery: When the VPS reconnects and completes a WireGuard handshake, optimized routes are automatically re-engaged.
  - Real-Time GUI Health Badges: Top bar, game cards, and tunnel modal display health status (`🟢 Ativo`, `🟡 Conectando`, `⚠️ Falha VPS (Fallback Direto)`), reassuring the user that their game is actively protected against disconnects.
- **Zero External Configuration Dependency**: Compiles and runs out of the box with safe
  built-in defaults (`AppConfig::default()`). No external `games.toml` file is required on disk.
- **Interactive Running Process Picker & Root Process Tree Detection**: Users can select
  directly from active running processes in the GUI without navigating file explorer directories.
  The optimizer automatically resolves parent process chains to detect the root game process
  and root PID (e.g. recognizing the root launcher and shipping binaries), allowing 1-click
  monitoring, memory trim, or protection.
- **Direct WireGuard Tunnel Import & Re-import (.conf) Everywhere**: Users can import
  or re-import standard WireGuard `.conf` configuration files directly via 1-click native Windows file
  dialogs prominently placed in the top KPI bar, in the right sidebar ("TÚNEL WIREGUARD (VPS)"),
  and inside the tunnel modal. Also supports pasting raw `.conf` text and a new "🔄 Recarregar do Perfil"
  button to reload saved configurations from disk.
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
