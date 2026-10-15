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
    /// Routes retain the exact identity used when they were installed.
    active_routes: HashMap<(Ipv4Addr, Ipv4Addr), OwnedRoute>,
    /// Tunnel gateway address (e.g. 10.0.0.1).
    gateway: Option<Ipv4Addr>,
    /// Optional Windows interface index for explicit route binding.
    if_index: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RouteIdentity {
    mask: Ipv4Addr,
    gateway: Ipv4Addr,
    if_index: Option<u32>,
}

impl RouteIdentity {
    fn key(self, destination: Ipv4Addr) -> (Ipv4Addr, Ipv4Addr) {
        (destination, self.mask)
    }
}

#[derive(Debug)]
struct OwnedRoute {
    identity: RouteIdentity,
    pids: Vec<u32>,
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

        let identity = RouteIdentity {
            mask,
            gateway,
            if_index: self.if_index,
        };
        if self.active_routes.contains_key(&identity.key(subnet_ip))
            && self.reuse_or_remove(subnet_ip, identity, pid)?
        {
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
            return Err(format!(
                "Erro ao adicionar rota de sub-rede: {stdout} {stderr}"
            ));
        }

        self.active_routes.insert(
            identity.key(subnet_ip),
            OwnedRoute {
                identity,
                pids: vec![pid],
            },
        );
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

        let identity = RouteIdentity {
            mask: Ipv4Addr::new(255, 255, 255, 255),
            gateway,
            if_index: self.if_index,
        };
        if self.active_routes.contains_key(&identity.key(target_ip))
            && self.reuse_or_remove(target_ip, identity, pid)?
        {
            return Ok(false);
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

        self.active_routes.insert(
            identity.key(target_ip),
            OwnedRoute {
                identity,
                pids: vec![pid],
            },
        );
        Ok(true)
    }

    fn reuse_or_remove(
        &mut self,
        ip: Ipv4Addr,
        identity: RouteIdentity,
        pid: u32,
    ) -> Result<bool, String> {
        let key = identity.key(ip);
        if self
            .active_routes
            .get(&key)
            .is_some_and(|route| route.identity == identity)
        {
            let route = self
                .active_routes
                .get_mut(&key)
                .expect("route checked above");
            if !route.pids.contains(&pid) {
                route.pids.push(pid);
            }
            return Ok(true);
        }
        let old = self
            .active_routes
            .get(&key)
            .expect("route checked above")
            .identity;
        delete_host_route(ip, old)?;
        self.active_routes.remove(&key);
        Ok(false)
    }

    /// Remove routes associated with a terminated game PID.
    pub fn remove_routes_for_pid(&mut self, pid: u32) -> Vec<Ipv4Addr> {
        let mut to_delete = Vec::new();

        for (ip, route) in self.active_routes.iter_mut() {
            route.pids.retain(|&p| p != pid);
            if route.pids.is_empty() {
                to_delete.push(*ip);
            }
        }

        for key in &to_delete {
            let identity = self.active_routes[key].identity;
            if delete_host_route(key.0, identity).is_ok() {
                self.active_routes.remove(key);
            }
        }

        to_delete
            .into_iter()
            .filter(|key| !self.active_routes.contains_key(key))
            .map(|(ip, _)| ip)
            .collect()
    }

    /// Remove all managed routes (used on shutdown or when failover bypass is triggered).
    pub fn cleanup_all(&mut self) -> Vec<Ipv4Addr> {
        let routes: Vec<_> = self.active_routes.keys().copied().collect();
        let mut removed = Vec::new();
        for key in &routes {
            let identity = self.active_routes[key].identity;
            if delete_host_route(key.0, identity).is_ok() {
                self.active_routes.remove(key);
                removed.push(key.0);
            }
        }
        removed
    }

    /// List all currently routed game IPs.
    pub fn active_ips(&self) -> Vec<Ipv4Addr> {
        self.active_routes.keys().map(|(ip, _)| *ip).collect()
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

fn delete_route_args(target_ip: Ipv4Addr, identity: RouteIdentity) -> Vec<String> {
    let mut args = vec![
        "delete".into(),
        target_ip.to_string(),
        "mask".into(),
        identity.mask.to_string(),
        identity.gateway.to_string(),
    ];
    if let Some(index) = identity.if_index.filter(|i| *i > 0) {
        args.push("IF".into());
        args.push(index.to_string());
    }
    args
}

fn delete_host_route(target_ip: Ipv4Addr, identity: RouteIdentity) -> Result<(), String> {
    let args = delete_route_args(target_ip, identity);
    let output = Command::new("route")
        .args(&args)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_manager_initial_state() {
        let mgr = RouteManager::new();
        assert!(mgr.is_empty());
        assert_eq!(mgr.len(), 0);
    }

    #[test]
    fn host_and_subnet_routes_with_the_same_address_have_distinct_keys() {
        let subnet = RouteIdentity {
            mask: Ipv4Addr::new(255, 255, 255, 0),
            gateway: Ipv4Addr::new(10, 8, 0, 1),
            if_index: Some(12),
        };
        let host = RouteIdentity {
            mask: Ipv4Addr::new(255, 255, 255, 255),
            ..subnet
        };
        let destination = Ipv4Addr::new(192, 0, 2, 0);
        assert_ne!(subnet.key(destination), host.key(destination));
    }

    #[test]
    fn deletion_arguments_include_exact_route_identity() {
        let identity = RouteIdentity {
            mask: Ipv4Addr::new(255, 255, 255, 0),
            gateway: Ipv4Addr::new(10, 8, 0, 1),
            if_index: Some(12),
        };
        assert_eq!(
            delete_route_args(Ipv4Addr::new(192, 0, 2, 0), identity),
            vec![
                "delete",
                "192.0.2.0",
                "mask",
                "255.255.255.0",
                "10.8.0.1",
                "IF",
                "12"
            ]
        );
    }
}
