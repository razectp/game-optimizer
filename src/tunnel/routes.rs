//! Windows routing table manager for game server tunneling.
//!
//! Adds and removes host-specific (/32) routes directing game server traffic
//! through the WireGuard tunnel gateway.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::os::windows::process::CommandExt;
use std::process::Command;

const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Route manager tracking active routes injected for running games.
#[derive(Debug, Default)]
pub struct RouteManager {
    /// Map of Remote IP -> list of PIDs using it.
    active_routes: HashMap<Ipv4Addr, Vec<u32>>,
    /// Tunnel gateway address (e.g. 10.0.0.1).
    gateway: Option<Ipv4Addr>,
    /// Optional Windows interface index for explicit route binding.
    if_index: Option<u32>,
}

impl RouteManager {
    /// Create a new route manager.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set or update the active tunnel gateway.
    pub fn set_gateway(&mut self, gateway: Ipv4Addr) {
        self.gateway = Some(gateway);
    }

    /// Set or update the active interface index.
    pub fn set_if_index(&mut self, if_index: Option<u32>) {
        self.if_index = if_index;
    }

    /// Add a subnet route (e.g. /24 cluster) for game servers.
    pub fn add_subnet_route(
        &mut self,
        pid: u32,
        subnet_ip: Ipv4Addr,
        mask: Ipv4Addr,
    ) -> Result<bool, String> {
        let gateway = match self.gateway {
            Some(gw) => gw,
            None => return Err("Gateway do túnel não configurado".into()),
        };

        if let Some(pids) = self.active_routes.get_mut(&subnet_ip) {
            if !pids.contains(&pid) {
                pids.push(pid);
            }
            return Ok(false);
        }

        let mut args = vec![
            "add".to_string(),
            subnet_ip.to_string(),
            "mask".to_string(),
            mask.to_string(),
            gateway.to_string(),
        ];
        if let Some(idx) = self.if_index {
            if idx > 0 {
                args.push("IF".to_string());
                args.push(idx.to_string());
            }
        }
        args.push("metric".to_string());
        args.push("1".to_string());

        let output = Command::new("route")
            .args(&args)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("Falha ao executar 'route add subnet': {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            return Err(format!("Erro ao adicionar rota de sub-rede: {stdout} {stderr}"));
        }

        self.active_routes.insert(subnet_ip, vec![pid]);
        Ok(true)
    }

    /// Add a host route for a game process destination IP.
    /// Also automatically adds the /24 cluster subnet route so any new game connections
    /// in the same server pool (such as world loading on port 31000) pass directly
    /// through the tunnel from packet #0.
    pub fn add_game_route(&mut self, pid: u32, target_ip: Ipv4Addr) -> Result<bool, String> {
        let gateway = match self.gateway {
            Some(gw) => gw,
            None => return Err("Gateway do túnel não configurado".into()),
        };

        // Automatically pre-route the /24 cluster subnet so subsequent game connections
        // (e.g. world loading on port 31000 or channel changes) already have a route in place!
        let octets = target_ip.octets();
        let subnet_ip = Ipv4Addr::new(octets[0], octets[1], octets[2], 0);
        let _ = self.add_subnet_route(pid, subnet_ip, Ipv4Addr::new(255, 255, 255, 0));

        if let Some(pids) = self.active_routes.get_mut(&target_ip) {
            if !pids.contains(&pid) {
                pids.push(pid);
            }
            return Ok(false); // Route already exists
        }

        // Apply host route: route add <IP> mask 255.255.255.255 <Gateway> [IF <if_index>] metric 1
        let mut args = vec![
            "add".to_string(),
            target_ip.to_string(),
            "mask".to_string(),
            "255.255.255.255".to_string(),
            gateway.to_string(),
        ];
        if let Some(idx) = self.if_index {
            if idx > 0 {
                args.push("IF".to_string());
                args.push(idx.to_string());
            }
        }
        args.push("metric".to_string());
        args.push("1".to_string());

        let output = Command::new("route")
            .args(&args)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("Falha ao executar 'route add': {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            return Err(format!("Erro ao adicionar rota: {stdout} {stderr}"));
        }

        self.active_routes.insert(target_ip, vec![pid]);
        Ok(true)
    }

    /// Remove routes associated with a terminated game PID.
    pub fn remove_routes_for_pid(&mut self, pid: u32) -> Vec<Ipv4Addr> {
        let mut to_delete = Vec::new();

        for (ip, pids) in self.active_routes.iter_mut() {
            pids.retain(|&p| p != pid);
            if pids.is_empty() {
                to_delete.push(*ip);
            }
        }

        for ip in &to_delete {
            self.active_routes.remove(ip);
            let _ = delete_host_route(*ip);
        }

        to_delete
    }

    /// Remove all managed routes (used on shutdown or when failover bypass is triggered).
    pub fn cleanup_all(&mut self) -> Vec<Ipv4Addr> {
        let ips: Vec<Ipv4Addr> = self.active_routes.keys().copied().collect();
        for ip in &ips {
            let _ = delete_host_route(*ip);
        }
        self.active_routes.clear();
        ips
    }

    /// List all currently routed game IPs.
    pub fn active_ips(&self) -> Vec<Ipv4Addr> {
        self.active_routes.keys().copied().collect()
    }

    /// Count of active routes.
    pub fn len(&self) -> usize {
        self.active_routes.len()
    }

    /// Whether there are no active routes.
    pub fn is_empty(&self) -> bool {
        self.active_routes.is_empty()
    }
}

impl Drop for RouteManager {
    fn drop(&mut self) {
        self.cleanup_all();
    }
}

fn delete_host_route(target_ip: Ipv4Addr) -> Result<(), String> {
    let output = Command::new("route")
        .args(["delete", &target_ip.to_string()])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("Falha ao executar 'route delete': {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!("Erro ao remover rota: {stdout} {stderr}"));
    }

    Ok(())
}

/// Clean up any leftover routes in Windows routing table that point to the given tunnel gateway.
/// Useful on startup and shutdown to purge any orphaned host routes left behind by previous crashes or runs.
pub fn cleanup_stale_gateway_routes(gateway: Ipv4Addr) {
    let _ = Command::new("route")
        .args(["delete", "*", &gateway.to_string()])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_manager_initial_state() {
        let mgr = RouteManager::new();
        assert!(mgr.is_empty());
        assert_eq!(mgr.len(), 0);
    }
}
