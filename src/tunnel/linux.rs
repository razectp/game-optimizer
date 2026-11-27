//! Linux WireGuard policy routing for automatically detected game processes.

#![allow(unsafe_code)]

use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use wireguard_uapi::{set, DeviceInterface, WgSocket};

use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, ToSocketAddrs};
use std::process::{ChildStdin, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::linux_game_group::{self, GameGroup, GameProcess};
use super::{TunnelHealth, TunnelPassReport, TunnelStatus, WireGuardConfig};

const TABLE: &str = "gameoptimizer";
const ROUTE_TABLE: &str = "51821";
const MARK: &str = "0x474f5054";
const OUTER_MARK: u32 = 0x474f5055;
const GAME_RULE: &str = "10000";
const PROBE_RULE: &str = "10001";
const OUTER_RULE: &str = "9999";
const TICK: Duration = Duration::from_secs(2);
const RETRY: Duration = Duration::from_secs(5);
const MAX_PROFILE_BYTES: usize = 4096;
const MAX_GAME_PROCESSES: usize = 128;
static STOP: AtomicBool = AtomicBool::new(false);

#[derive(Default)]
struct GameSelection {
    tunnel: Vec<GameProcess>,
    local: Vec<GameProcess>,
}

/// Point of entry for the privileged helper. The parent owns its lifetime via stdin.
pub fn run_gui_tunnel_helper() -> anyhow::Result<()> {
    let result = serve_gui_tunnel();
    if let Err(error) = &result {
        let message = error.to_string().replace(['\r', '\n'], " ");
        let _ = writeln!(io::stdout(), "error\t{message}");
    }
    result
}

fn serve_gui_tunnel() -> anyhow::Result<()> {
    install_stop_handlers()?;
    let mut input = io::stdin().lock();
    let mut length = [0; 4];
    input.read_exact(&mut length)?;
    let length = u32::from_be_bytes(length) as usize;
    anyhow::ensure!(
        length <= MAX_PROFILE_BYTES,
        "Perfil WireGuard grande demais."
    );
    let mut profile = vec![0; length];
    input.read_exact(&mut profile)?;
    // Parse errors may include the input, so keep credentials out of the diagnostics.
    let config: WireGuardConfig = toml::from_str(std::str::from_utf8(&profile)?)
        .map_err(|_| anyhow::anyhow!("Perfil WireGuard inválido."))?;
    drop(input);
    anyhow::ensure!(
        config.enabled && config.is_configured(),
        "Perfil WireGuard incompleto."
    );

    let (monitor_tx, monitor_rx) = mpsc::sync_channel(1);
    thread::spawn(move || read_game_updates(monitor_tx));

    let mut manager = TunnelManager::new(config);
    manager.start().map_err(anyhow::Error::msg)?;
    let mut previous = None;
    let mut games = GameSelection::default();
    while !STOP.load(Ordering::Relaxed) {
        manager.process_games(&games);
        let status = manager.status();
        if previous.as_ref() != Some(&status) {
            let health = match status.health {
                TunnelHealth::Idle => "idle",
                TunnelHealth::Connecting => "connecting",
                TunnelHealth::Healthy => "healthy",
                TunnelHealth::Degraded => "degraded",
            };
            let message = status.message.replace(['\r', '\n'], " ");
            writeln!(io::stdout(), "{health}\t{message}")?;
            previous = Some(status);
        }
        match monitor_rx.recv_timeout(TICK) {
            Ok(current) => games = current,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    manager.shutdown();
    Ok(())
}

fn read_game_updates(sender: mpsc::SyncSender<GameSelection>) {
    let mut input = io::stdin().lock();
    loop {
        let mut count = [0; 2];
        if input.read_exact(&mut count).is_err() {
            return;
        }
        let count = usize::from(u16::from_be_bytes(count));
        if count > MAX_GAME_PROCESSES {
            return;
        }
        let mut games = GameSelection::default();
        for _ in 0..count {
            let mut bytes = [0; 13];
            if input.read_exact(&mut bytes).is_err() {
                return;
            }
            let game = GameProcess {
                pid: u32::from_be_bytes(bytes[..4].try_into().expect("four bytes")),
                start: u64::from_be_bytes(bytes[4..12].try_into().expect("eight bytes")),
            };
            match bytes[12] {
                0 => games.local.push(game),
                1 => games.tunnel.push(game),
                _ => return,
            }
        }
        if sender.send(games).is_err() {
            return;
        }
    }
}

enum HelperEvent {
    Status(TunnelHealth, String),
    Error(String),
    Exited(io::Result<ExitStatus>),
}

struct HelperProcess {
    input: Option<ChildStdin>,
    events: Receiver<HelperEvent>,
    accepting_games: bool,
    last_games: Vec<(GameProcess, bool)>,
}

impl HelperProcess {
    fn spawn(config: &WireGuardConfig) -> anyhow::Result<Self> {
        run("systemctl", &["--user", "start", "gameoptimizer.slice"])
            .map_err(anyhow::Error::msg)?;
        let mut config = config.clone();
        // The Linux helper controls socket routing; DNS and legacy routes stay local.
        config.dns = None;
        config.custom_routes.clear();
        let profile = toml::to_string(&config)?;
        anyhow::ensure!(
            profile.len() <= MAX_PROFILE_BYTES,
            "Perfil WireGuard grande demais."
        );
        let executable = env::current_exe()?;
        let mut elevated = if executable == std::path::Path::new(crate::linux_install::APPLICATION)
        {
            let mut pkexec = Command::new("/usr/bin/pkexec");
            pkexec
                .arg("--disable-internal-agent")
                .arg(crate::linux_install::TUNNEL_HELPER);
            pkexec
        } else {
            let mut pkexec = Command::new("pkexec");
            pkexec
                .arg("--disable-internal-agent")
                .arg(executable)
                .arg("tunnel-helper");
            pkexec
        };
        let mut child = elevated
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|err| {
                anyhow::anyhow!("Falha ao abrir a autorização gráfica do Linux: {err}")
            })?;
        let mut input = child.stdin.take().expect("stdin configured");
        let output = child.stdout.take().expect("stdout configured");
        let (sender, events) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(output).lines().map_while(Result::ok) {
                if let Some((kind, message)) = line.split_once('\t') {
                    let health = match kind {
                        "idle" => TunnelHealth::Idle,
                        "connecting" => TunnelHealth::Connecting,
                        "healthy" => TunnelHealth::Healthy,
                        "degraded" => TunnelHealth::Degraded,
                        "error" => {
                            let _ = sender.send(HelperEvent::Error(message.into()));
                            continue;
                        }
                        _ => continue,
                    };
                    let _ = sender.send(HelperEvent::Status(health, message.into()));
                }
            }
            let _ = sender.send(HelperEvent::Exited(child.wait()));
        });
        input.write_all(&(profile.len() as u32).to_be_bytes())?;
        input.write_all(profile.as_bytes())?;
        Ok(Self {
            input: Some(input),
            events,
            accepting_games: false,
            last_games: Vec::new(),
        })
    }

    fn send_games(&mut self, games: impl Iterator<Item = (GameProcess, bool)>) -> io::Result<()> {
        let Some(input) = self.input.as_mut().filter(|_| self.accepting_games) else {
            return Ok(());
        };
        let mut games: Vec<_> = games.take(MAX_GAME_PROCESSES).collect();
        games.sort_unstable_by_key(|(game, _)| game.pid);
        if games == self.last_games {
            return Ok(());
        }
        let mut bytes = Vec::with_capacity(2 + games.len() * 13);
        bytes.extend_from_slice(&(games.len() as u16).to_be_bytes());
        for (game, enabled) in &games {
            bytes.extend_from_slice(&game.pid.to_be_bytes());
            bytes.extend_from_slice(&game.start.to_be_bytes());
            bytes.push(u8::from(*enabled));
        }
        input.write_all(&bytes)?;
        self.last_games = games;
        Ok(())
    }
}

/// Manages the privileged WireGuard child from the unprivileged Linux GUI worker.
pub(crate) struct GuiTunnel {
    config: WireGuardConfig,
    helper: Option<HelperProcess>,
    status: TunnelStatus,
    attempted: bool,
    helper_error: bool,
    game_overrides: HashMap<GameProcess, bool>,
}

impl GuiTunnel {
    pub(crate) fn new(config: WireGuardConfig) -> Self {
        Self {
            config,
            helper: None,
            status: TunnelStatus {
                active: false,
                health: TunnelHealth::Idle,
                vps_ping_ms: None,
                active_routes_count: 0,
                routed_destinations: Vec::new(),
                message: "Túnel em repouso.".into(),
            },
            attempted: false,
            helper_error: false,
            game_overrides: HashMap::new(),
        }
    }

    pub(crate) fn update_config(&mut self, config: WireGuardConfig) {
        if self.config != config {
            self.shutdown();
            self.config = config;
        }
    }

    pub(crate) fn retry(&mut self) {
        if self.helper.is_none() {
            self.attempted = false;
        }
    }

    pub(crate) fn set_game_enabled(&mut self, game: GameProcess, enabled: bool) {
        self.game_overrides.insert(game, enabled);
    }

    fn set_health(&mut self, health: TunnelHealth, message: String) {
        self.status.active = health == TunnelHealth::Healthy;
        self.status.health = health;
        self.status.active_routes_count = usize::from(self.status.active);
        self.status.message = message;
    }

    pub(crate) fn process_game_pids(&mut self, pids: &[u32]) -> TunnelPassReport {
        let old = self.status.health;
        while let Some(event) = self
            .helper
            .as_ref()
            .and_then(|helper| helper.events.try_recv().ok())
        {
            let stopping = self
                .helper
                .as_ref()
                .is_some_and(|helper| helper.input.is_none());
            match event {
                HelperEvent::Status(health, message) if !stopping => {
                    if let Some(helper) = &mut self.helper {
                        helper.accepting_games = true;
                    }
                    self.set_health(health, message);
                }
                HelperEvent::Error(message) if !stopping => {
                    self.helper_error = true;
                    self.set_health(TunnelHealth::Degraded, message);
                }
                HelperEvent::Exited(result) => {
                    self.helper = None;
                    if !stopping && !self.helper_error {
                        let message = match result.ok().and_then(|status| status.code()) {
                            Some(126) => "Autorização cancelada. Salve o perfil para tentar novamente.",
                            Some(127) => "O Linux não autorizou o túnel. Confirme que há um agente gráfico do Polkit.",
                            _ => "O processo que gerencia o túnel foi encerrado. Salve o perfil para tentar novamente.",
                        };
                        self.set_health(TunnelHealth::Degraded, message.into());
                    }
                }
                _ => {}
            }
        }

        if self.config.enabled
            && self.config.is_configured()
            && self.helper.is_none()
            && !self.attempted
        {
            self.attempted = true;
            self.helper_error = false;
            self.set_health(
                TunnelHealth::Connecting,
                "Aguardando autorização do Linux…".into(),
            );
            match HelperProcess::spawn(&self.config) {
                Ok(helper) => self.helper = Some(helper),
                Err(err) => self.set_health(TunnelHealth::Degraded, err.to_string()),
            }
        }
        if let Some(helper) = &mut self.helper {
            self.game_overrides
                .retain(|game, _| GameProcess::read(game.pid) == Some(*game));
            let games = pids
                .iter()
                .copied()
                .filter_map(GameProcess::read)
                .map(|game| {
                    let enabled = self
                        .game_overrides
                        .get(&game)
                        .copied()
                        .unwrap_or(self.config.auto_route_games);
                    (game, enabled)
                });
            if let Err(err) = helper.send_games(games) {
                self.helper_error = true;
                helper.accepting_games = false;
                self.set_health(
                    TunnelHealth::Degraded,
                    format!("Falha ao enviar jogos ao túnel: {err}"),
                );
            }
        }
        TunnelPassReport {
            active_routes_count: self.status.active_routes_count,
            fallback_bypassed: old == TunnelHealth::Healthy
                && self.status.health == TunnelHealth::Degraded,
            recovered: old == TunnelHealth::Degraded && self.status.active,
            ..TunnelPassReport::default()
        }
    }

    pub(crate) fn test_vps_ping(&mut self) -> Result<u32, String> {
        let ms = self.config.ping_vps(Duration::from_millis(1500))?;
        self.status.vps_ping_ms = Some(ms);
        Ok(ms)
    }

    pub(crate) fn status(&self) -> TunnelStatus {
        self.status.clone()
    }

    pub(crate) fn shutdown(&mut self) {
        if let Some(helper) = &mut self.helper {
            helper.input = None;
        }
        self.attempted = false;
        self.status.vps_ping_ms = None;
        self.set_health(TunnelHealth::Idle, "Túnel em repouso.".into());
    }
}

/// Monitor the configured Linux game tunnel until SIGINT or SIGTERM.
pub fn watch_tunnel(config: crate::AppConfig) -> anyhow::Result<()> {
    anyhow::ensure!(
        config.wireguard.enabled && config.wireguard.is_configured(),
        "Habilite e configure o túnel WireGuard no perfil primeiro."
    );
    install_stop_handlers()?;
    let mut manager = TunnelManager::new(config.wireguard.clone());
    manager.start().map_err(anyhow::Error::msg)?;
    println!("Monitorando jogos detectados. Ctrl+C encerra o túnel.");
    let mut previous = String::new();
    while !STOP.load(Ordering::Relaxed) {
        let pids: Vec<_> = crate::detect::snapshot_processes(&config)
            .into_iter()
            .filter(|process| process.class == crate::detect::ProcessClass::Game)
            .map(|process| process.pid)
            .collect();
        manager.process_game_pids(&pids);
        let message = manager.status_text.clone();
        if message != previous {
            println!("{message}");
            previous = message;
        }
        thread::sleep(TICK);
    }
    manager.shutdown();
    Ok(())
}

fn install_stop_handlers() -> io::Result<()> {
    // The handler only writes to a lock-free atomic; cleanup runs in the main loop.
    extern "C" fn stop(_: libc::c_int) {
        STOP.store(true, Ordering::Relaxed);
    }
    // SAFETY: Both handlers have the required C ABI and are valid for the program lifetime.
    unsafe {
        for signal in [libc::SIGINT, libc::SIGTERM] {
            if libc::signal(signal, stop as *const () as libc::sighandler_t) == libc::SIG_ERR {
                return Err(io::Error::last_os_error());
            }
        }
    }
    Ok(())
}

#[derive(Debug)]
struct NetworkManagerExclusion(PathBuf);

impl NetworkManagerExclusion {
    fn install(interface: &str) -> Result<Option<Self>, String> {
        if run(
            "systemctl",
            &["is-active", "--quiet", "NetworkManager.service"],
        )
        .is_err()
        {
            return Ok(None);
        }

        let base = Path::new("/run/NetworkManager");
        let directory = base.join("conf.d");
        let trusted_directory = |path: &Path| {
            fs::symlink_metadata(path).is_ok_and(|metadata| {
                metadata.is_dir() && metadata.uid() == 0 && metadata.mode() & 0o022 == 0
            })
        };
        if !trusted_directory(base) {
            return Err("Diretório de execução do NetworkManager indisponível.".into());
        }
        if let Err(err) = fs::create_dir(&directory) {
            if err.kind() != io::ErrorKind::AlreadyExists {
                return Err(format!("Falha ao preparar o NetworkManager: {err}"));
            }
        }
        if !trusted_directory(&directory) {
            return Err("Diretório de configuração do NetworkManager inválido.".into());
        }

        let id = std::process::id();
        let path = directory.join(format!("90-game-optimizer-{interface}-{id}.conf"));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|err| format!("Falha ao isolar a interface do NetworkManager: {err}"))?;
        let exclusion = Self(path);
        write!(
            file,
            "[device-game-optimizer-{id}]\nmatch-device=interface-name:={interface}\nmanaged=0\n"
        )
        .map_err(|err| format!("Falha ao isolar a interface do NetworkManager: {err}"))?;
        drop(file);
        run("nmcli", &["--wait", "3", "general", "reload", "conf"])?;
        Ok(Some(exclusion))
    }
}

impl Drop for NetworkManagerExclusion {
    fn drop(&mut self) {
        if fs::remove_file(&self.0).is_ok() {
            let _ = run("nmcli", &["--wait", "3", "general", "reload", "conf"]);
        }
    }
}

#[derive(Debug)]
struct LinuxTunnel {
    interface: String,
    endpoint: SocketAddrV4,
    gateway: Ipv4Addr,
    games: GameGroup,
    rebound: HashSet<GameProcess>,
    owns_table: bool,
    owns_link: bool,
    network_manager_exclusion: Option<NetworkManagerExclusion>,
    has_mark_rule: bool,
    has_outer_rule: bool,
    has_probe_rule: bool,
    started: Instant,
    last_probe: Option<Instant>,
    gateway_ping_worked: bool,
    missed_gateway_pings: u8,
    routing: bool,
}

impl LinuxTunnel {
    fn start(config: &WireGuardConfig) -> Result<Self, String> {
        let uid = env::var("PKEXEC_UID")
            .or_else(|_| env::var("SUDO_UID"))
            .ok()
            .and_then(|value| value.parse().ok())
            .filter(|uid: &u32| *uid != 0)
            .ok_or("Não foi possível identificar a sessão de usuário que autorizou o túnel.")?;
        let interface = config.interface_name.trim();
        if interface.is_empty()
            || interface.len() > 15
            || !interface
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err("Nome de interface WireGuard inválido no Linux (até 15 letras, dígitos, '_' ou '-').".into());
        }
        let address: Ipv4Addr = config
            .tunnel_ip
            .trim()
            .parse()
            .map_err(|_| "Endereço IPv4 do cliente WireGuard inválido.")?;
        let gateway: Ipv4Addr = config
            .tunnel_gateway
            .trim()
            .parse()
            .map_err(|_| "Gateway IPv4 WireGuard inválido.")?;
        if address.is_unspecified() || gateway.is_unspecified() || address == gateway {
            return Err("Configure endereços IPv4 distintos para o cliente e o gateway.".into());
        }
        let endpoint = config
            .endpoint
            .trim()
            .to_socket_addrs()
            .map_err(|e| format!("Falha ao resolver endpoint WireGuard: {e}"))?
            .find_map(|address| match address {
                SocketAddr::V4(address) => Some(address),
                SocketAddr::V6(_) => None,
            })
            .ok_or("O endpoint WireGuard precisa de um endereço IPv4.")?;
        let private_key = decode_key(&config.private_key, "privada")?;
        let peer_public_key = decode_key(&config.peer_public_key, "pública da VPS")?;
        let preshared_key = config
            .preshared_key
            .as_deref()
            .map(|key| decode_key(key, "pré-compartilhada"))
            .transpose()?;
        let mut tunnel = Self {
            interface: interface.into(),
            endpoint,
            gateway,
            games: GameGroup::new(uid)
                .map_err(|err| format!("Falha ao preparar o grupo de jogos: {err}"))?,
            rebound: HashSet::new(),
            owns_table: false,
            owns_link: false,
            network_manager_exclusion: None,
            has_mark_rule: false,
            has_outer_rule: false,
            has_probe_rule: false,
            started: Instant::now(),
            last_probe: None,
            gateway_ping_worked: false,
            missed_gateway_pings: 0,
            routing: false,
        };

        // Create (rather than add) claims the table exclusively. The table starts with an
        // empty lease set, so setup and partial failures never divert game traffic.
        run_with_input("nft", &["-f", "-"], &tunnel.ruleset())?;
        tunnel.owns_table = true;
        if let Ok(routes) = run("ip", &["-4", "route", "show", "table", ROUTE_TABLE]) {
            if !routes.trim().is_empty() {
                return Err(format!("A tabela de rotas {ROUTE_TABLE} já está ocupada."));
            }
        }
        if run("ip", &["link", "show", "dev", interface]).is_ok() {
            return Err(format!("A interface {interface} já está em uso."));
        }
        // Exclude only our interface before it appears, so NM creates no external profile.
        tunnel.network_manager_exclusion = NetworkManagerExclusion::install(interface)?;
        run(
            "ip",
            &["link", "add", "name", interface, "type", "wireguard"],
        )?;
        tunnel.owns_link = true;

        let endpoint = SocketAddr::V4(endpoint);
        let any = IpAddr::V4(Ipv4Addr::UNSPECIFIED);
        let mut peer = set::Peer::from_public_key(&peer_public_key)
            .endpoint(&endpoint)
            .persistent_keepalive_interval(config.keepalive)
            .allowed_ips(vec![set::AllowedIp {
                ipaddr: &any,
                cidr_mask: Some(0),
            }]);
        if let Some(key) = &preshared_key {
            peer = peer.preshared_key(key);
        }
        let device = set::Device::from_ifname(interface)
            .private_key(&private_key)
            .fwmark(OUTER_MARK)
            .peers(vec![peer]);
        WgSocket::connect()
            .map_err(|error| format!("Falha ao abrir WireGuard no kernel: {error}"))?
            .set_device(device)
            .map_err(|error| format!("Falha ao configurar WireGuard no kernel: {error}"))?;
        run(
            "ip",
            &[
                "-4",
                "address",
                "add",
                &format!("{address}/32"),
                "dev",
                interface,
            ],
        )?;
        run(
            "sysctl",
            &["-w", &format!("net.ipv4.conf.{interface}.rp_filter=2")],
        )?;
        run(
            "ip",
            &[
                "link",
                "set",
                "dev",
                interface,
                "mtu",
                &config.mtu.to_string(),
                "up",
            ],
        )?;
        run(
            "ip",
            &[
                "-4",
                "route",
                "add",
                "default",
                "dev",
                interface,
                "table",
                ROUTE_TABLE,
            ],
        )?;
        let outer_mark = OUTER_MARK.to_string();
        policy_rule(
            "add",
            OUTER_RULE,
            &["fwmark", &outer_mark, "lookup", "main"],
        )?;
        tunnel.has_outer_rule = true;
        policy_rule("add", GAME_RULE, &["fwmark", MARK, "lookup", ROUTE_TABLE])?;
        tunnel.has_mark_rule = true;
        policy_rule(
            "add",
            PROBE_RULE,
            &["oif", interface, "lookup", ROUTE_TABLE],
        )?;
        tunnel.has_probe_rule = true;
        Ok(tunnel)
    }

    fn ruleset(&self) -> String {
        let interface = &self.interface;
        let endpoint = self.endpoint.ip();
        let cgroup = self.games.cgroup().to_string_lossy();
        let level = cgroup.split('/').count();
        let excluded_ports = super::linux_reconnect::WEB_AND_DNS_PORTS
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "create table ip {TABLE}\n\
             add set ip {TABLE} ready {{ type mark; flags timeout; }}\n\
             add chain ip {TABLE} output {{ type route hook output priority mangle; policy accept; }}\n\
             add chain ip {TABLE} game\n\
             add rule ip {TABLE} output meta mark @ready ip daddr != {{ {endpoint}, 0.0.0.0/8, 10.0.0.0/8, 127.0.0.0/8, 169.254.0.0/16, 172.16.0.0/12, 192.168.0.0/16, 224.0.0.0/3 }} meta l4proto {{ tcp, udp }} th dport != {{ {excluded_ports} }} socket cgroupv2 level {level} \"{cgroup}\" jump game\n\
             add rule ip {TABLE} game ct mark 0 ct state new ct status & confirmed == 0 ct mark set {MARK}\n\
             add rule ip {TABLE} game ct mark {MARK} meta mark set {MARK}\n\
             add chain ip {TABLE} postrouting {{ type nat hook postrouting priority srcnat; policy accept; }}\n\
             add rule ip {TABLE} postrouting oifname \"{interface}\" meta mark {MARK} masquerade\n",
        )
    }

    fn tick(&mut self) -> Result<TunnelHealth, String> {
        let now = Instant::now();
        if self
            .last_probe
            .is_none_or(|last| now.duration_since(last) >= RETRY)
        {
            self.last_probe = Some(now);
            // Outbound traffic to the inner gateway initiates/retries a handshake.
            // Authentication is checked via WireGuard, even if ICMP replies are blocked.
            let probe = run(
                "ping",
                &[
                    "-n",
                    "-4",
                    "-I",
                    &self.interface,
                    "-c",
                    "1",
                    "-W",
                    "1",
                    &self.gateway.to_string(),
                ],
            );
            if probe.is_ok() {
                self.gateway_ping_worked = true;
                self.missed_gateway_pings = 0;
            } else {
                self.missed_gateway_pings = self.missed_gateway_pings.saturating_add(1);
            }
        }
        let device = WgSocket::connect()
            .map_err(|error| format!("Falha ao consultar WireGuard: {error}"))?
            .get_device(DeviceInterface::from_name(self.interface.as_str()))
            .map_err(|error| format!("Falha ao consultar o túnel: {error}"))?;
        let unix_now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let has_recent_handshake = device.peers.iter().any(|peer| {
            let timestamp = peer.last_handshake_time.as_secs();
            timestamp > 0 && unix_now.abs_diff(timestamp) < 180
        });
        let healthy =
            has_recent_handshake && (!self.gateway_ping_worked || self.missed_gateway_pings < 3);
        if healthy || self.routing {
            // The kernel expires the diversion after eight seconds if the app stops refreshing it.
            let mut rules = format!("flush set ip {TABLE} ready\n");
            if healthy {
                rules.push_str(&format!(
                    "add element ip {TABLE} ready {{ 0 timeout 8s }}\n"
                ));
            }
            run_with_input("nft", &["-f", "-"], &rules)?;
        }
        self.routing = healthy;
        Ok(if healthy {
            TunnelHealth::Healthy
        } else if now.duration_since(self.started) < Duration::from_secs(15) {
            TunnelHealth::Connecting
        } else {
            TunnelHealth::Degraded
        })
    }
}

impl Drop for LinuxTunnel {
    fn drop(&mut self) {
        if self.owns_table {
            let _ = run("nft", &["delete", "table", "ip", TABLE]);
        }
        if self.has_probe_rule {
            let _ = policy_rule(
                "del",
                PROBE_RULE,
                &["oif", &self.interface, "lookup", ROUTE_TABLE],
            );
        }
        if self.has_mark_rule {
            let _ = policy_rule("del", GAME_RULE, &["fwmark", MARK, "lookup", ROUTE_TABLE]);
        }
        if self.has_outer_rule {
            let _ = policy_rule(
                "del",
                OUTER_RULE,
                &["fwmark", &OUTER_MARK.to_string(), "lookup", "main"],
            );
        }
        if self.owns_link {
            let _ = run("ip", &["link", "del", "dev", &self.interface]);
        }
    }
}

pub(crate) fn is_scoped_game(pid: u32) -> bool {
    // SAFETY: getuid takes no arguments and has no memory preconditions.
    let uid = unsafe { libc::getuid() };
    linux_game_group::is_scoped_game(pid, uid)
}

fn decode_key(key: &str, label: &str) -> Result<[u8; 32], String> {
    STANDARD
        .decode(key.trim())
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| {
            format!("Chave {label} do WireGuard inválida (esperados 32 bytes em Base64).")
        })
}

fn policy_rule(action: &str, priority: &str, selector: &[&str]) -> Result<(), String> {
    let mut args = vec!["-4", "rule", action, "priority", priority];
    args.extend_from_slice(selector);
    run("ip", &args).map(|_| ())
}

pub(super) fn run(tool: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(tool)
        .args(args)
        .output()
        .map_err(|e| format!("Falha ao executar {tool}: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "{tool}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn run_with_input(tool: &str, args: &[&str], input: &str) -> Result<(), String> {
    let mut child = Command::new(tool)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Falha ao executar {tool}: {e}"))?;
    let written = child
        .stdin
        .take()
        .expect("stdin configured")
        .write_all(input.as_bytes());
    let output = child
        .wait_with_output()
        .map_err(|e| format!("Falha ao executar {tool}: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "{tool}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    written.map_err(|e| format!("Falha ao enviar configuração para {tool}: {e}"))
}

/// Native Linux tunnel coordinator. Socket membership in the launch slice controls routing.
#[derive(Debug)]
pub struct TunnelManager {
    config: WireGuardConfig,
    tunnel: Option<LinuxTunnel>,
    health: TunnelHealth,
    status_text: String,
    last_tick: Option<Instant>,
    last_ping: Option<u32>,
}

impl TunnelManager {
    /// Create an inactive Linux tunnel manager.
    pub fn new(config: WireGuardConfig) -> Self {
        Self {
            config,
            tunnel: None,
            health: TunnelHealth::Idle,
            status_text: "Túnel pronto.".into(),
            last_tick: None,
            last_ping: None,
        }
    }

    /// Replace the tunnel settings and tear down resources owned by the old session.
    pub fn update_config(&mut self, config: WireGuardConfig) {
        if self.config != config {
            self.shutdown();
            self.config = config;
            self.last_ping = None;
        }
    }

    fn start(&mut self) -> Result<(), String> {
        self.tunnel = Some(LinuxTunnel::start(&self.config)?);
        Ok(())
    }

    /// Enroll the currently detected game processes and refresh the routing lease.
    pub fn process_game_pids(&mut self, pids: &[u32]) -> TunnelPassReport {
        let detected = pids.iter().copied().filter_map(GameProcess::read).collect();
        let mut games = GameSelection::default();
        if self.config.auto_route_games {
            games.tunnel = detected;
        } else {
            games.local = detected;
        }
        self.process_games(&games)
    }

    fn process_games(&mut self, selection: &GameSelection) -> TunnelPassReport {
        let mut report = TunnelPassReport::default();
        if !self.config.enabled || !self.config.is_configured() {
            self.shutdown();
            self.status_text = "Túnel desativado ou configuração incompleta.".into();
            return report;
        }
        let now = Instant::now();
        let interval = if self.tunnel.is_some() { TICK } else { RETRY };
        let was_healthy = self.health == TunnelHealth::Healthy;
        if self
            .last_tick
            .is_none_or(|last| now.duration_since(last) >= interval)
        {
            self.last_tick = Some(now);
            let result = if let Some(tunnel) = &mut self.tunnel {
                tunnel.tick()
            } else {
                self.start().map(|_| TunnelHealth::Connecting)
            };
            match result {
                Ok(health) => {
                    self.health = health;
                    self.status_text = match health {
                        TunnelHealth::Healthy => "WireGuard ativo para jogos detectados.",
                        TunnelHealth::Connecting => "Conectando à VPS (aguardando handshake)…",
                        TunnelHealth::Degraded => {
                            "Túnel sem resposta da VPS. Novas conexões seguem pela rota normal."
                        }
                        TunnelHealth::Idle => "Túnel em repouso.",
                    }
                    .into();
                }
                Err(error) => {
                    self.tunnel = None;
                    self.health = TunnelHealth::Degraded;
                    self.status_text = error;
                }
            }
        }
        if let Some(tunnel) = &mut self.tunnel {
            use super::linux_reconnect::{reconnect, Route};

            let games = &selection.tunnel;
            let (released, group_result) = tunnel.games.synchronize(games, &selection.local);
            tunnel
                .rebound
                .retain(|game| tunnel.routing && games.contains(game));
            if let Err(err) = group_result {
                self.status_text = format!("Falha ao alterar o vínculo do jogo com o túnel: {err}");
            }

            let endpoint = *tunnel.endpoint.ip();
            match reconnect(&released, &tunnel.games, endpoint, Route::Local) {
                Ok(0) => {}
                Ok(count) => {
                    self.status_text = format!(
                        "Reconexão pela rota local solicitada para {count} conexão(ões) TCP."
                    )
                }
                Err(err) => {
                    self.status_text =
                        format!("Falha ao solicitar reconexão pela rota local: {err}")
                }
            }
            if tunnel.routing {
                let pending: Vec<_> = games
                    .iter()
                    .copied()
                    .filter(|game| {
                        !tunnel.rebound.contains(game) && tunnel.games.contains_game(*game)
                    })
                    .collect();
                if !pending.is_empty() {
                    // Request fresh sockets once per game instance and after a tunnel recovery.
                    // Sockets opened during fallback can already belong to the managed cgroup.
                    tunnel.rebound.extend(pending.iter().copied());
                    match reconnect(&pending, &tunnel.games, endpoint, Route::Tunnel) {
                        Ok(0) => {}
                        Ok(count) => {
                            self.status_text = format!(
                                "Reconexão pelo túnel solicitada para {count} conexão(ões) TCP."
                            )
                        }
                        Err(err) => {
                            self.status_text =
                                format!("Túnel ativo; reconexão automática indisponível: {err}")
                        }
                    }
                }
            }
        }
        report.fallback_bypassed = was_healthy && self.health == TunnelHealth::Degraded;
        report.recovered = self.health == TunnelHealth::Healthy && !was_healthy;
        report.active_routes_count = usize::from(self.health == TunnelHealth::Healthy);
        report
    }

    /// Measure latency to the configured VPS and cache the result.
    pub fn test_vps_ping(&mut self) -> Result<u32, String> {
        let ms = self.config.ping_vps(Duration::from_millis(1500))?;
        self.last_ping = Some(ms);
        Ok(ms)
    }

    /// Return the current Linux tunnel status.
    pub fn status(&self) -> TunnelStatus {
        let active = self.health == TunnelHealth::Healthy;
        TunnelStatus {
            active,
            health: self.health,
            vps_ping_ms: self.last_ping,
            active_routes_count: usize::from(active),
            routed_destinations: Vec::new(),
            message: self.status_text.clone(),
        }
    }

    /// Stop routing and release resources owned by this session.
    pub fn shutdown(&mut self) {
        self.tunnel = None;
        self.health = TunnelHealth::Idle;
        self.last_tick = None;
        self.status_text = "Túnel em repouso.".into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_config_stays_idle_without_creating_an_interface() {
        let mut manager = TunnelManager::new(WireGuardConfig::default());
        assert_eq!(
            manager
                .process_games(&GameSelection::default())
                .active_routes_count,
            0
        );
        assert_eq!(manager.status().health, TunnelHealth::Idle);
        assert!(manager.tunnel.is_none());
    }

    #[test]
    fn wireguard_keys_cannot_inject_configuration_lines() {
        assert!(decode_key(&format!("{}=", "A".repeat(43)), "privada").is_ok());
        assert!(decode_key(&format!("{}\n=", "A".repeat(42)), "privada").is_err());
    }
}
