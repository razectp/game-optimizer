//! Embedded userspace WireGuard engine using BoringTun and Wintun.
//!
//! Operates a completely independent TUN adapter without touching or relying on
//! any external WireGuard client or existing tunnels.

#![allow(unsafe_code)]

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;
use boringtun::noise::{Tunn, TunnResult};

use crate::tunnel::wireguard::WireGuardConfig;

/// Handle for a running embedded WireGuard tunnel.
pub struct EmbeddedTunnelHandle {
    stop_flag: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
    /// Windows interface index for GameOptimizerTun.
    pub if_index: u32,
    handshake_confirmed: Arc<AtomicBool>,
    last_rx_secs: Arc<AtomicU64>,
    started_at: Instant,
}

impl std::fmt::Debug for EmbeddedTunnelHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmbeddedTunnelHandle")
            .field("running", &!self.stop_flag.load(Ordering::Relaxed))
            .field("connected", &self.is_connected())
            .finish()
    }
}

impl Drop for EmbeddedTunnelHandle {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::SeqCst);
    }
}

impl EmbeddedTunnelHandle {
    /// Stop the embedded tunnel worker threads.
    pub fn stop(mut self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        let threads = std::mem::take(&mut self.threads);
        for t in threads {
            let _ = t.join();
        }
    }

    /// Whether a WireGuard handshake was successfully completed at least once.
    pub fn has_handshake(&self) -> bool {
        self.handshake_confirmed.load(Ordering::SeqCst)
    }

    /// Whether the tunnel is currently authenticated and connected with the WireGuard peer.
    pub fn is_connected(&self) -> bool {
        if !self.handshake_confirmed.load(Ordering::SeqCst) {
            return false;
        }
        if self.stop_flag.load(Ordering::Relaxed) {
            return false;
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let last = self.last_rx_secs.load(Ordering::SeqCst);
        if last == 0 {
            return true;
        }
        // WireGuard standard REKEY_ATTEMPT_TIME window (90s).
        // WireGuard is silent when idle or when host drops ICMP.
        // We only consider connection lost if no packet has arrived for over 90 seconds.
        now.saturating_sub(last) <= 90
    }

    /// Whether the tunnel was recently initiated and is waiting for initial handshake.
    pub fn is_initial_connecting(&self) -> bool {
        !self.handshake_confirmed.load(Ordering::SeqCst)
            && self.started_at.elapsed() < Duration::from_secs(25)
    }

    /// Seconds since the last packet was received from the VPS peer.
    pub fn seconds_since_last_rx(&self) -> u64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let last = self.last_rx_secs.load(Ordering::SeqCst);
        if last == 0 {
            u64::MAX
        } else {
            now.saturating_sub(last)
        }
    }
}

/// Start the embedded WireGuard tunnel if configured.
pub fn start_embedded_tunnel(config: &WireGuardConfig) -> Result<EmbeddedTunnelHandle, String> {
    if !config.is_configured() {
        return Err("WireGuard não configurado".into());
    }

    let endpoint_ip = config
        .resolve_endpoint_ip()
        .ok_or_else(|| "Não foi possível resolver o endpoint da VPS".to_string())?;

    let endpoint_port = config
        .endpoint
        .split(':')
        .next_back()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(51820);

    let endpoint_addr = SocketAddr::from((endpoint_ip, endpoint_port));

    // Decode keys
    let priv_bytes = base64::engine::general_purpose::STANDARD
        .decode(config.private_key.trim())
        .map_err(|e| format!("Chave privada inválida: {e}"))?;

    let pub_bytes = base64::engine::general_purpose::STANDARD
        .decode(config.peer_public_key.trim())
        .map_err(|e| format!("Chave pública da VPS inválida: {e}"))?;

    if priv_bytes.len() < 32 || pub_bytes.len() < 32 {
        return Err("Chaves do WireGuard devem ter 32 bytes (256 bits)".into());
    }

    let mut priv_arr = [0u8; 32];
    priv_arr.copy_from_slice(&priv_bytes[..32]);
    let static_private = x25519_dalek::StaticSecret::from(priv_arr);

    let mut pub_arr = [0u8; 32];
    pub_arr.copy_from_slice(&pub_bytes[..32]);
    let peer_public = x25519_dalek::PublicKey::from(pub_arr);

    // Initialize Wintun
    let wintun_path = find_wintun_dll()?;
    let wintun = unsafe { wintun::load_from_path(&wintun_path) }
        .map_err(|e| format!("Falha ao carregar {}: {e}", wintun_path.display()))?;

    let adapter_name = "GameOptimizerTun";
    let pool_name = "GameOptimizer";

    let adapter = match wintun::Adapter::open(&wintun, adapter_name) {
        Ok(a) => a,
        Err(_) => wintun::Adapter::create(&wintun, adapter_name, pool_name, None)
            .map_err(|e| format!("Falha ao criar adaptador Wintun: {e}"))?,
    };

    let if_index = adapter.get_adapter_index().unwrap_or(0);

    // Configure tunnel IP address cleanly via netsh
    let client_ip: Ipv4Addr = config
        .tunnel_ip
        .trim()
        .parse()
        .unwrap_or(Ipv4Addr::new(10, 8, 0, 2));

    // netsh interface ipv4 set address name=GameOptimizerTun source=static address=10.8.0.2 mask=255.255.255.0
    let _ = Command::new("netsh")
        .args([
            "interface",
            "ipv4",
            "set",
            "address",
            &format!("name={adapter_name}"),
            "source=static",
            &format!("address={client_ip}"),
            "mask=255.255.255.0",
        ])
        .creation_flags(0x08000000)
        .output();

    // Configure MTU
    let _ = Command::new("netsh")
        .args([
            "interface",
            "ipv4",
            "set",
            "subinterface",
            &format!("name={adapter_name}"),
            &format!("mtu={}", config.mtu),
            "store=active",
        ])
        .creation_flags(0x08000000)
        .output();

    let session = Arc::new(
        adapter
            .start_session(wintun::MAX_RING_CAPACITY)
            .map_err(|e| format!("Falha ao iniciar sessão Wintun: {e}"))?,
    );

    let socket = Arc::new(
        UdpSocket::bind("0.0.0.0:0")
            .map_err(|e| format!("Falha ao abrir socket UDP local: {e}"))?,
    );
    socket
        .set_read_timeout(Some(Duration::from_millis(100)))
        .map_err(|e| format!("Falha ao configurar timeout do socket: {e}"))?;

    // Use standard WireGuard NAT persistent keepalive (default: 25s)
    let keepalive_val = if config.keepalive == 0 {
        25
    } else {
        config.keepalive
    };
    let tunn = Arc::new(std::sync::Mutex::new(Tunn::new(
        static_private,
        peer_public,
        None,
        Some(keepalive_val),
        0,
        None,
    )));

    let stop_flag = Arc::new(AtomicBool::new(false));
    let handshake_confirmed = Arc::new(AtomicBool::new(false));
    let last_rx_secs = Arc::new(AtomicU64::new(0));
    let started_at = Instant::now();
    let mut threads = Vec::new();

    // Trigger initial handshake
    {
        let mut out_buf = vec![0u8; 65535];
        let mut t = match tunn.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let TunnResult::WriteToNetwork(packet) =
            t.format_handshake_initiation(&mut out_buf, false)
        {
            let _ = socket.send_to(packet, endpoint_addr);
        }
    }

    // Thread 1: Inbound UDP (VPS -> BoringTun -> Wintun)
    {
        let socket = Arc::clone(&socket);
        let session = Arc::clone(&session);
        let tunn = Arc::clone(&tunn);
        let stop = Arc::clone(&stop_flag);
        let hs_confirmed = Arc::clone(&handshake_confirmed);
        let rx_secs = Arc::clone(&last_rx_secs);

        threads.push(thread::spawn(move || {
            let mut udp_buf = vec![0u8; 65535];
            let mut ip_buf = vec![0u8; 65535];

            while !stop.load(Ordering::Relaxed) {
                match socket.recv_from(&mut udp_buf) {
                    Ok((n, src)) => {
                        let mut t = match tunn.lock() {
                            Ok(guard) => guard,
                            Err(poisoned) => poisoned.into_inner(),
                        };
                        match t.decapsulate(Some(src.ip()), &udp_buf[..n], &mut ip_buf) {
                            TunnResult::Done => {
                                hs_confirmed.store(true, Ordering::SeqCst);
                                let now = SystemTime::now()
                                    .duration_since(UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_secs();
                                rx_secs.store(now, Ordering::SeqCst);
                            }
                            TunnResult::Err(_) => {}
                            TunnResult::WriteToNetwork(reply) => {
                                hs_confirmed.store(true, Ordering::SeqCst);
                                let now = SystemTime::now()
                                    .duration_since(UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_secs();
                                rx_secs.store(now, Ordering::SeqCst);
                                let _ = socket.send_to(reply, endpoint_addr);
                            }
                            TunnResult::WriteToTunnelV4(ip_pkt, _)
                            | TunnResult::WriteToTunnelV6(ip_pkt, _) => {
                                hs_confirmed.store(true, Ordering::SeqCst);
                                let now = SystemTime::now()
                                    .duration_since(UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_secs();
                                rx_secs.store(now, Ordering::SeqCst);
                                if let Ok(mut send_pkt) =
                                    session.allocate_send_packet(ip_pkt.len() as u16)
                                {
                                    send_pkt.bytes_mut().copy_from_slice(ip_pkt);
                                    session.send_packet(send_pkt);
                                }
                            }
                        }
                    }
                    Err(ref e)
                        if e.kind() == std::io::ErrorKind::TimedOut
                            || e.kind() == std::io::ErrorKind::WouldBlock =>
                    {
                        // Periodic read timeout - normal
                    }
                    Err(_) => break,
                }
            }
        }));
    }

    // Thread 2: Outbound Wintun (Game -> Wintun -> BoringTun -> UDP VPS)
    {
        let socket = Arc::clone(&socket);
        let session = Arc::clone(&session);
        let tunn = Arc::clone(&tunn);
        let stop = Arc::clone(&stop_flag);

        threads.push(thread::spawn(move || {
            let mut udp_out_buf = vec![0u8; 65535];

            while !stop.load(Ordering::Relaxed) {
                match session.receive_blocking() {
                    Ok(packet) => {
                        let bytes = packet.bytes();
                        let mut t = match tunn.lock() {
                            Ok(guard) => guard,
                            Err(poisoned) => poisoned.into_inner(),
                        };
                        match t.encapsulate(bytes, &mut udp_out_buf) {
                            TunnResult::WriteToNetwork(enc_bytes) => {
                                let _ = socket.send_to(enc_bytes, endpoint_addr);
                            }
                            TunnResult::Done => {}
                            TunnResult::Err(_) => {}
                            _ => {}
                        }
                    }
                    Err(_) => {
                        if stop.load(Ordering::Relaxed) {
                            break;
                        }
                        thread::sleep(Duration::from_millis(5));
                    }
                }
            }
        }));
    }

    // Thread 3: WireGuard Timers & Keepalive Loop
    {
        let socket = Arc::clone(&socket);
        let tunn = Arc::clone(&tunn);
        let stop = Arc::clone(&stop_flag);
        let hs_confirmed = Arc::clone(&handshake_confirmed);
        let rx_secs = Arc::clone(&last_rx_secs);

        threads.push(thread::spawn(move || {
            let mut out_buf = vec![0u8; 65535];
            let mut last_handshake_retry = Instant::now();

            while !stop.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(250));

                let mut t = match tunn.lock() {
                    Ok(guard) => guard,
                    Err(poisoned) => poisoned.into_inner(),
                };
                // BoringTun manages WireGuard timers, keepalives, and standard rekeys internally
                if let TunnResult::WriteToNetwork(packet) = t.update_timers(&mut out_buf) {
                    let _ = socket.send_to(packet, endpoint_addr);
                }

                // If initial handshake hasn't confirmed yet, retry initiation every 5 seconds
                if !hs_confirmed.load(Ordering::SeqCst) {
                    if last_handshake_retry.elapsed() >= Duration::from_secs(5) {
                        if let TunnResult::WriteToNetwork(packet) =
                            t.format_handshake_initiation(&mut out_buf, false)
                        {
                            let _ = socket.send_to(packet, endpoint_addr);
                        }
                        last_handshake_retry = Instant::now();
                    }
                } else {
                    // Handshake confirmed: Only if total silence exceeds 60s, attempt a rekey initiation
                    let last = rx_secs.load(Ordering::SeqCst);
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    if last > 0
                        && now.saturating_sub(last) > 60
                        && last_handshake_retry.elapsed() >= Duration::from_secs(15)
                    {
                        if let TunnResult::WriteToNetwork(packet) =
                            t.format_handshake_initiation(&mut out_buf, false)
                        {
                            let _ = socket.send_to(packet, endpoint_addr);
                        }
                        last_handshake_retry = Instant::now();
                    }
                }
            }
        }));
    }

    Ok(EmbeddedTunnelHandle {
        stop_flag,
        threads,
        if_index,
        handshake_confirmed,
        last_rx_secs,
        started_at,
    })
}

fn find_wintun_dll() -> Result<PathBuf, String> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join("wintun.dll");
            if p.exists() {
                return Ok(p);
            }
        }
    }
    let local = Path::new("wintun.dll");
    if local.exists() {
        return Ok(local.to_path_buf());
    }
    let warp = Path::new("C:\\Program Files\\Cloudflare\\Cloudflare WARP\\wintun.dll");
    if warp.exists() {
        return Ok(warp.to_path_buf());
    }
    Err("wintun.dll não encontrada na pasta do executável nem no sistema.".into())
}
