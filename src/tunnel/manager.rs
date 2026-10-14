use std::collections::{HashMap, HashSet};
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use crate::tunnel::embedded_engine::{start_embedded_tunnel, EmbeddedTunnelHandle};
use crate::tunnel::routes::RouteManager;
use crate::tunnel::sockets::{find_game_remote_ips, ProcessConnection};
use crate::tunnel::wireguard::WireGuardConfig;

/// Connection health of the WireGuard tunnel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TunnelHealth {
    /// Tunnel is not running (disabled or no games active).
    Idle,
    /// Establishing connection (awaiting handshake confirmation from VPS).
    Connecting,
    /// Active, authenticated, and passing packets cleanly.
    Healthy,
    /// Connection to VPS dropped or unreachable. Fallback bypass active to protect games.
    Degraded,
}

/// Status summary of the embedded tunnel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TunnelStatus {
    /// Whether the tunnel is currently active and healthy.
    pub active: bool,
    /// Health condition of the tunnel.
    pub health: TunnelHealth,
    /// Last measured ping to VPS in milliseconds.
    pub vps_ping_ms: Option<u32>,
    /// Number of active host routes routing game traffic through the tunnel.
    pub active_routes_count: usize,
    /// List of actively routed game server endpoints (IP and protocol).
    pub routed_destinations: Vec<String>,
    /// Friendly status message.
    pub message: String,
}

/// Result of an optimization or monitoring pass on the tunnel.
#[derive(Debug, Clone, Default)]
pub struct TunnelPassReport {
    /// Newly discovered connections routed during this pass.
    pub newly_routed: Vec<ProcessConnection>,
    /// Routes removed due to closed game processes or failover bypass.
    pub removed_routes: Vec<Ipv4Addr>,
    /// Total active routes.
    pub active_routes_count: usize,
    /// Whether failover bypass was activated this pass due to VPS connection loss.
    pub fallback_bypassed: bool,
    /// Whether the tunnel connection was restored from a previous failure.
    pub recovered: bool,
}

/// The tunnel coordinator instance.
#[derive(Debug)]
pub struct TunnelManager {
    /// Current WireGuard configuration.
    config: WireGuardConfig,
    /// Host routing manager.
    route_mgr: RouteManager,
    /// Embedded userspace WireGuard engine instance.
    embedded_engine: Option<EmbeddedTunnelHandle>,
    /// Current health status of the connection.
    health: TunnelHealth,
    /// Set of active game PIDs seen in the last pass.
    last_known_pids: HashSet<u32>,
    /// Cache of known connections per PID.
    active_connections: HashMap<u32, Vec<ProcessConnection>>,
    /// Last measured VPS ping and timestamp.
    last_ping: Option<(u32, Instant)>,
    /// Status text.
    status_text: String,
}

impl TunnelManager {
    /// Initialize a new tunnel manager with the given config.
    pub fn new(config: WireGuardConfig) -> Self {
        let mut route_mgr = RouteManager::new();
        if let Ok(gw) = config.tunnel_gateway.trim().parse::<Ipv4Addr>() {
            route_mgr.set_gateway(gw);
        }

        Self {
            config,
            route_mgr,
            embedded_engine: None,
            health: TunnelHealth::Idle,
            last_known_pids: HashSet::new(),
            active_connections: HashMap::new(),
            last_ping: None,
            status_text: "Túnel pronto.".into(),
        }
    }

    /// Update configuration (e.g. edited from GUI).
    pub fn update_config(&mut self, config: WireGuardConfig) {
        if let Ok(gw) = config.tunnel_gateway.trim().parse::<Ipv4Addr>() {
            self.route_mgr.set_gateway(gw);
        }
        if let Some(engine) = self.embedded_engine.take() {
            engine.stop();
        }
        self.health = TunnelHealth::Idle;
        self.config = config;
    }

    /// Access current configuration.
    pub fn config(&self) -> &WireGuardConfig {
        &self.config
    }

    /// Mutable access to configuration.
    pub fn config_mut(&mut self) -> &mut WireGuardConfig {
        &mut self.config
    }

    /// Check if the tunnel is enabled and ready to route.
    pub fn is_enabled(&self) -> bool {
        self.config.enabled && self.config.is_configured()
    }

    /// Refresh, verify connection health, and route connections for active games.
    pub fn process_game_pids(&mut self, current_pids: &[u32]) -> TunnelPassReport {
        let mut report = TunnelPassReport::default();

        if !self.is_enabled() {
            if let Some(engine) = self.embedded_engine.take() {
                engine.stop();
            }
            if !self.last_known_pids.is_empty() {
                let removed = self.route_mgr.cleanup_all();
                report.removed_routes.extend(removed);
                self.last_known_pids.clear();
                self.active_connections.clear();
            }
            self.health = TunnelHealth::Idle;
            self.status_text = "Túnel desativado.".into();
            return report;
        }

        let current_set: HashSet<u32> = current_pids.iter().copied().collect();

        if current_set.is_empty() {
            if let Some(engine) = self.embedded_engine.take() {
                engine.stop();
                self.route_mgr.set_if_index(None);
            }
            if !self.last_known_pids.is_empty() {
                let removed = self.route_mgr.cleanup_all();
                report.removed_routes.extend(removed);
                self.last_known_pids.clear();
                self.active_connections.clear();
            }
            self.health = TunnelHealth::Idle;
            self.status_text = "Túnel em repouso (aguardando jogo).".into();
            return report;
        }

        // 1. Ensure embedded WireGuard tunnel is started when games are running
        if self.embedded_engine.is_none() {
            match start_embedded_tunnel(&self.config) {
                Ok(handle) => {
                    self.route_mgr.set_if_index(Some(handle.if_index));
                    self.embedded_engine = Some(handle);
                    self.health = TunnelHealth::Connecting;
                    self.status_text = "Conectando à VPS (aguardando handshake)...".into();
                }
                Err(err) => {
                    self.health = TunnelHealth::Degraded;
                    self.status_text = format!("Aviso túnel: {err}");
                }
            }
        }

        // 2. Continuous Health Check & Failure Detection
        let prev_health = self.health;
        let is_healthy = match &self.embedded_engine {
            Some(engine) => {
                if engine.is_connected() {
                    true
                } else if engine.is_initial_connecting() {
                    self.health = TunnelHealth::Connecting;
                    false
                } else {
                    self.health = TunnelHealth::Degraded;
                    false
                }
            }
            None => {
                self.health = TunnelHealth::Degraded;
                false
            }
        };

        if is_healthy {
            self.health = TunnelHealth::Healthy;
            if prev_health == TunnelHealth::Degraded {
                report.recovered = true;
            }
        }

        // 3. Automatic Failover Bypass: If tunnel fails, PURGE routes immediately!
        // This ensures game packets flow via normal default gateway without dropping the game.
        if self.health == TunnelHealth::Degraded {
            if !self.route_mgr.is_empty() {
                let removed = self.route_mgr.cleanup_all();
                report.removed_routes.extend(removed);
                report.fallback_bypassed = true;

                // Reset established TCP connections so game clients reconnect immediately
                // over the default physical gateway instead of hanging on dead tunnel sockets.
                for (pid, conns) in &self.active_connections {
                    for conn in conns {
                        if conn.protocol == "TCP" {
                            crate::tunnel::sockets::reset_tcp_connection(*pid, conn.remote_ip);
                        }
                    }
                }
            }
            self.active_connections.clear();
            self.status_text =
                "⚠️ Falha de conexão com a VPS! Fallback ativo: o jogo continua na internet direta sem cair.".into();
        } else if self.health == TunnelHealth::Connecting {
            self.status_text = "Conectando à VPS (aguardando handshake inicial)...".into();
        }

        // 4. Detect closed games and remove their routes
        let closed_pids: Vec<u32> = self
            .last_known_pids
            .difference(&current_set)
            .copied()
            .collect();

        for pid in closed_pids {
            let removed = self.route_mgr.remove_routes_for_pid(pid);
            report.removed_routes.extend(removed);
            self.active_connections.remove(&pid);
        }

        // 5. If and ONLY IF the tunnel is verified Healthy, route game connections through the tunnel!
        if self.health == TunnelHealth::Healthy {
            let connections = find_game_remote_ips(&current_set);

            for conn in connections {
                match self.route_mgr.add_game_route(conn.pid, conn.remote_ip) {
                    Ok(newly_added) => {
                        if newly_added {
                            report.newly_routed.push(conn.clone());
                            // If this was an already-established TCP connection, reset it
                            // so the game client immediately reconnects via the new tunnel route.
                            if conn.protocol == "TCP" {
                                crate::tunnel::sockets::reset_tcp_connection(conn.pid, conn.remote_ip);
                            }
                        }
                        self.active_connections
                            .entry(conn.pid)
                            .or_default()
                            .push(conn);
                    }
                    Err(err) => {
                        self.status_text = format!("Aviso de rota: {err}");
                    }
                }
            }

            // Apply any custom configured routes for games
            for custom in &self.config.custom_routes {
                if let Ok(ip) = custom.trim().parse::<Ipv4Addr>() {
                    for &pid in &current_set {
                        let _ = self.route_mgr.add_game_route(pid, ip);
                    }
                }
            }
            self.status_text = "Túnel WireGuard conectado e ativo.".into();
        }

        self.last_known_pids = current_set;
        report.active_routes_count = self.route_mgr.len();
        report
    }

    /// Test latency to the VPS endpoint and cache the result.
    pub fn test_vps_ping(&mut self) -> Result<u32, String> {
        let ms = self.config.ping_vps(Duration::from_millis(1500))?;
        self.last_ping = Some((ms, Instant::now()));
        self.status_text = format!("Ping VPS: {ms}ms");
        Ok(ms)
    }

    /// Current status snapshot for GUI or CLI display.
    pub fn status(&self) -> TunnelStatus {
        let mut destinations = Vec::new();
        for ip in self.route_mgr.active_ips() {
            destinations.push(ip.to_string());
        }

        let ping = self.last_ping.map(|(ms, _)| ms);

        TunnelStatus {
            active: self.health == TunnelHealth::Healthy,
            health: self.health,
            vps_ping_ms: ping,
            active_routes_count: self.route_mgr.len(),
            routed_destinations: destinations,
            message: self.status_text.clone(),
        }
    }

    /// Tear down all active routes cleanly.
    pub fn shutdown(&mut self) {
        if let Some(engine) = self.embedded_engine.take() {
            engine.stop();
        }
        self.route_mgr.cleanup_all();
        self.health = TunnelHealth::Idle;
        self.last_known_pids.clear();
        self.active_connections.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tunnel_manager_initial_state_is_idle() {
        let mgr = TunnelManager::new(WireGuardConfig::default());
        let status = mgr.status();
        assert_eq!(status.health, TunnelHealth::Idle);
        assert!(!status.active);
        assert_eq!(status.active_routes_count, 0);
    }

    #[test]
    fn process_game_pids_when_disabled_remains_idle() {
        let mut mgr = TunnelManager::new(WireGuardConfig::default());
        let report = mgr.process_game_pids(&[1234, 5678]);
        assert_eq!(mgr.status().health, TunnelHealth::Idle);
        assert!(!report.fallback_bypassed);
        assert_eq!(report.active_routes_count, 0);
    }

    #[test]
    fn degraded_tunnel_transitions_to_degraded_when_tunnel_cannot_start() {
        let mut cfg = WireGuardConfig::default();
        cfg.enabled = true;
        cfg.endpoint = "192.0.2.1:51820".into();
        cfg.private_key = "aGVsbG93b3JsZGFzZGFzZGFzZGFzZGFzZGFzZGFzZGE=".into();
        cfg.peer_public_key = "d29ybGRoZWxsb2FzZGFzZGFzZGFzZGFzZGFzZGFzZGE=".into();
        let mut mgr = TunnelManager::new(cfg);

        // When starting tunnel fails (no wintun.dll in test / unprivileged), health becomes Degraded
        let report = mgr.process_game_pids(&[9999]);
        assert_eq!(mgr.status().health, TunnelHealth::Degraded);
        assert!(!report.recovered);
    }
}

