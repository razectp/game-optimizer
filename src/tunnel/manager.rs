use std::collections::{HashMap, HashSet};
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use crate::tunnel::embedded_engine::{start_embedded_tunnel, EmbeddedTunnelHandle};
use crate::tunnel::routes::RouteManager;
use crate::tunnel::sockets::{find_game_remote_ips, ProcessConnection};
use crate::tunnel::wireguard::WireGuardConfig;

fn snapshot_connections(
    connections: impl IntoIterator<Item = ProcessConnection>,
) -> HashMap<u32, Vec<ProcessConnection>> {
    let mut snapshot = HashMap::new();
    for connection in connections {
        let entries: &mut Vec<ProcessConnection> = snapshot.entry(connection.pid).or_default();
        if !entries.contains(&connection) {
            entries.push(connection);
        }
    }
    snapshot
}

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
    /// Whether an initial migration of already-established game sockets was executed.
    initial_migration_done: bool,
    /// Discovered game server subnets (/24) for pre-routing future game connections.
    discovered_subnets: HashSet<Ipv4Addr>,
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
            initial_migration_done: false,
            discovered_subnets: HashSet::new(),
        }
    }

    /// Update configuration (e.g. edited from GUI).
    pub fn update_config(&mut self, config: WireGuardConfig) {
        if self.config == config {
            return;
        }
        // Remove routes while the adapter still exists. Closing Wintun first
        // may let Windows remove them before our ownership tracking is updated.
        self.route_mgr.cleanup_all();
        if let Some(engine) = self.embedded_engine.take() {
            engine.stop();
        }
        self.route_mgr.set_if_index(None);
        // Existing routes retain their original identity for safe cleanup.
        // New routes must always use the newly configured gateway.
        if let Ok(gw) = config.tunnel_gateway.trim().parse::<Ipv4Addr>() {
            self.route_mgr.set_gateway(gw);
        }
        self.health = TunnelHealth::Idle;
        self.initial_migration_done = false;
        self.last_known_pids.clear();
        self.active_connections.clear();
        self.discovered_subnets.clear();
        self.status_text = "Configuração do túnel atualizada.".into();
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
            if !self.last_known_pids.is_empty() || !self.route_mgr.is_empty() {
                let removed = self.route_mgr.cleanup_all();
                report.removed_routes.extend(removed);
                self.last_known_pids.clear();
                self.active_connections.clear();
                self.discovered_subnets.clear();
            }
            if let Some(engine) = self.embedded_engine.take() {
                engine.stop();
                self.route_mgr.set_if_index(None);
            }
            self.health = TunnelHealth::Idle;
            self.discovered_subnets.clear();
            self.status_text = "Túnel desativado.".into();
            return report;
        }

        let current_set: HashSet<u32> = current_pids.iter().copied().collect();

        if current_set.is_empty() {
            if !self.last_known_pids.is_empty() || !self.route_mgr.is_empty() {
                let removed = self.route_mgr.cleanup_all();
                report.removed_routes.extend(removed);
                self.last_known_pids.clear();
                self.active_connections.clear();
                self.discovered_subnets.clear();
            }
            if let Some(engine) = self.embedded_engine.take() {
                engine.stop();
                self.route_mgr.set_if_index(None);
            }
            self.initial_migration_done = false;
            self.discovered_subnets.clear();
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

                // NOTE: We deliberately do NOT reset TCP connections here.
                // Purging host routes instantly allows subsequent game packets to flow directly
                // through the physical default gateway. Resetting TCP connections forces games
                // (like MIR4) to display "reconnect" errors and freeze.
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
            let mut routed_snapshot = Vec::new();
            // Pre-route all previously discovered game subnets (/24) so that newly opening
            // game sockets (like port 31000 world loading) ALREADY match the tunnel route from their very first SYN packet!
            for &subnet in &self.discovered_subnets {
                for &pid in &current_set {
                    let _ = self.route_mgr.add_subnet_route(
                        pid,
                        subnet,
                        Ipv4Addr::new(255, 255, 255, 0),
                    );
                }
            }

            let connections = find_game_remote_ips(&current_set);
            let endpoint_ip = self.config.resolve_endpoint_ip();

            for conn in connections {
                if let Some(ep_ip) = endpoint_ip {
                    if conn.remote_ip == ep_ip {
                        continue;
                    }
                }

                // Remember this game server's /24 cluster subnet
                let octets = conn.remote_ip.octets();
                let subnet = Ipv4Addr::new(octets[0], octets[1], octets[2], 0);
                if self.discovered_subnets.len() < 256 {
                    self.discovered_subnets.insert(subnet);
                }

                match self.route_mgr.add_game_route(conn.pid, conn.remote_ip) {
                    Ok(newly_added) => {
                        if newly_added {
                            report.newly_routed.push(conn.clone());
                            // Only reset TCP connections during the INITIAL migration pass for a game
                            // that was ALREADY established on the physical adapter BEFORE the tunnel started.
                            // Connections created while the tunnel is running (like port 31000 world loading)
                            // are NEVER reset, preventing mid-load disconnects.
                            if !self.initial_migration_done
                                && conn.protocol == "TCP"
                                && crate::tunnel::sockets::is_game_port(conn.remote_port)
                            {
                                crate::tunnel::sockets::reset_tcp_connection(
                                    conn.pid,
                                    conn.remote_ip,
                                );
                            }
                        }
                        routed_snapshot.push(conn);
                    }
                    Err(err) => {
                        self.status_text = format!("Aviso de rota: {err}");
                    }
                }
            }

            // Mark initial migration done so future connections (e.g. world loading, map changes)
            // will NEVER be violently reset while in-flight!
            self.initial_migration_done = true;

            // Apply any custom configured routes for games
            for custom in &self.config.custom_routes {
                if let Ok(ip) = custom.trim().parse::<Ipv4Addr>() {
                    for &pid in &current_set {
                        let _ = self.route_mgr.add_game_route(pid, ip);
                    }
                }
            }
            self.status_text = "Túnel WireGuard conectado e ativo.".into();
            self.active_connections = snapshot_connections(routed_snapshot);
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
        self.route_mgr.cleanup_all();
        if let Some(engine) = self.embedded_engine.take() {
            engine.stop();
        }
        self.health = TunnelHealth::Idle;
        self.initial_migration_done = false;
        self.last_known_pids.clear();
        self.active_connections.clear();
        self.discovered_subnets.clear();
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
        let cfg = WireGuardConfig {
            enabled: true,
            endpoint: "192.0.2.1:51820".into(),
            private_key: "invalid".into(),
            peer_public_key: "invalid".into(),
            ..WireGuardConfig::default()
        };
        let mut mgr = TunnelManager::new(cfg);

        // When starting tunnel fails (no wintun.dll in test / unprivileged), health becomes Degraded
        let report = mgr.process_game_pids(&[9999]);
        assert_eq!(mgr.status().health, TunnelHealth::Degraded);
        assert!(!report.recovered);
    }

    #[test]
    fn constructor_and_equal_config_update_are_pure_noops() {
        let mut mgr = TunnelManager::new(WireGuardConfig::default());
        mgr.status_text = "preserved".into();
        mgr.update_config(WireGuardConfig::default());
        assert_eq!(mgr.status_text, "preserved");
        assert!(mgr.route_mgr.is_empty());
    }

    #[test]
    fn connection_snapshot_is_deduplicated_and_replaces_old_entries() {
        let connection = ProcessConnection {
            local_port: 40000,
            pid: 1,
            remote_ip: Ipv4Addr::new(192, 0, 2, 1),
            remote_port: 80,
            protocol: "TCP",
        };
        let mut cache = HashMap::new();
        for _ in 0..1000 {
            cache = snapshot_connections([connection.clone(), connection.clone()]);
        }
        assert_eq!(cache.get(&1).unwrap().len(), 1);
        cache = snapshot_connections(std::iter::empty());
        assert!(cache.is_empty());
    }
}
