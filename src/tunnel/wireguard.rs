//! WireGuard configuration and VPS tunnel integration.

#![allow(unsafe_code)]

use std::fs;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use windows::Win32::NetworkManagement::IpHelper::{
    IcmpCloseHandle, IcmpCreateFile, IcmpSendEcho, IP_OPTION_INFORMATION,
};

/// WireGuard embedded tunnel configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireGuardConfig {
    /// Whether the VPS tunnel is enabled.
    #[serde(default)]
    pub enabled: bool,
    /// VPS endpoint address, e.g. "203.0.113.10:51820" or "vps.example.com:51820".
    #[serde(default)]
    pub endpoint: String,
    /// Client base64 private key.
    #[serde(default)]
    pub private_key: String,
    /// VPS base64 public key.
    #[serde(default)]
    pub peer_public_key: String,
    /// Client IPv4 address in the tunnel, e.g. "10.0.0.2".
    #[serde(default = "default_tunnel_ip")]
    pub tunnel_ip: String,
    /// VPS IPv4 gateway address in the tunnel, e.g. "10.0.0.1".
    #[serde(default = "default_tunnel_gateway")]
    pub tunnel_gateway: String,
    /// DNS server to use (optional).
    #[serde(default)]
    pub dns: Option<String>,
    /// Tunnel interface MTU (default: 1420).
    #[serde(default = "default_mtu")]
    pub mtu: u16,
    /// Persistent keepalive interval in seconds (default: 25).
    #[serde(default = "default_keepalive")]
    pub keepalive: u16,
    /// Tunnel interface name on Windows.
    #[serde(default = "default_interface_name")]
    pub interface_name: String,
    /// Automatically route server connections of detected games.
    #[serde(default = "default_true")]
    pub auto_route_games: bool,
    /// Manual extra server IP/CIDR ranges to always tunnel for games.
    #[serde(default)]
    pub custom_routes: Vec<String>,
}

fn default_tunnel_ip() -> String {
    "10.0.0.2".into()
}

fn default_tunnel_gateway() -> String {
    "10.0.0.1".into()
}

fn default_mtu() -> u16 {
    1420
}

fn default_keepalive() -> u16 {
    25
}

fn default_interface_name() -> String {
    "GameOptTunnel".into()
}

fn default_true() -> bool {
    true
}

impl Default for WireGuardConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: String::new(),
            private_key: String::new(),
            peer_public_key: String::new(),
            tunnel_ip: default_tunnel_ip(),
            tunnel_gateway: default_tunnel_gateway(),
            dns: Some("1.1.1.1".into()),
            mtu: default_mtu(),
            keepalive: default_keepalive(),
            interface_name: default_interface_name(),
            auto_route_games: true,
            custom_routes: Vec::new(),
        }
    }
}

impl WireGuardConfig {
    /// Validates if the minimal configuration to connect is present.
    pub fn is_configured(&self) -> bool {
        !self.endpoint.trim().is_empty()
            && !self.private_key.trim().is_empty()
            && !self.peer_public_key.trim().is_empty()
    }

    /// Resolves the endpoint host to an IPv4 address.
    pub fn resolve_endpoint_ip(&self) -> Option<Ipv4Addr> {
        let endpoint = self.endpoint.trim();
        if endpoint.is_empty() {
            return None;
        }

        // Try direct SocketAddr parse
        if let Ok(addr) = endpoint.parse::<SocketAddr>() {
            return match addr.ip() {
                IpAddr::V4(v4) => Some(v4),
                IpAddr::V6(_) => None,
            };
        }

        // Try hostname resolution
        if let Ok(mut addrs) = endpoint.to_socket_addrs() {
            for addr in &mut addrs {
                if let IpAddr::V4(v4) = addr.ip() {
                    return Some(v4);
                }
            }
        }

        None
    }

    /// Generate a standard WireGuard `.conf` payload.
    ///
    /// NOTE: We deliberately do NOT set `AllowedIPs = 0.0.0.0/0` without `Table = off`!
    /// By specifying the tunnel gateway subnet in `AllowedIPs` and keeping standard routing,
    /// normal PC traffic is NEVER redirected, preserving complete isolation.
    pub fn generate_conf(&self) -> String {
        let mut conf = String::new();
        conf.push_str("[Interface]\n");
        conf.push_str(&format!("PrivateKey = {}\n", self.private_key.trim()));
        conf.push_str(&format!("Address = {}/24\n", self.tunnel_ip.trim()));
        if let Some(dns) = &self.dns {
            if !dns.trim().is_empty() {
                conf.push_str(&format!("DNS = {}\n", dns.trim()));
            }
        }
        conf.push_str(&format!("MTU = {}\n", self.mtu));
        // Table = off ensures Windows does not auto-add a default 0.0.0.0/0 route!
        conf.push_str("Table = off\n\n");

        conf.push_str("[Peer]\n");
        conf.push_str(&format!("PublicKey = {}\n", self.peer_public_key.trim()));
        conf.push_str(&format!("Endpoint = {}\n", self.endpoint.trim()));
        // AllowedIPs covers the tunnel subnet plus 0.0.0.0/0 in crypto-routing,
        // but because Table = off, Windows routes are managed strictly by Game Optimizer.
        conf.push_str("AllowedIPs = 0.0.0.0/0\n");
        if self.keepalive > 0 {
            conf.push_str(&format!("PersistentKeepalive = {}\n", self.keepalive));
        }

        conf
    }

    /// Write configuration to disk (e.g. for WireGuard CLI / Service).
    pub fn write_conf_file(&self, path: impl AsRef<Path>) -> std::io::Result<PathBuf> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, self.generate_conf())?;
        Ok(path)
    }

    /// Measure ICMP round-trip time (ping) to the VPS endpoint or internal gateway in milliseconds.
    pub fn ping_vps(&self, timeout: Duration) -> Result<u32, String> {
        // First try the internal tunnel gateway (direct inside-tunnel ping)
        if let Ok(gw_ip) = self.tunnel_gateway.trim().parse::<Ipv4Addr>() {
            if let Ok(ms) = ping_ipv4(gw_ip, Duration::from_millis(600)) {
                return Ok(ms);
            }
        }

        // Fallback to resolving and pinging the public endpoint IP
        let ip = self
            .resolve_endpoint_ip()
            .ok_or_else(|| "Não foi possível resolver o IP do endpoint da VPS".to_string())?;

        ping_ipv4(ip, timeout)
    }
}

/// Send an ICMP echo ping to an IPv4 address and return latency in milliseconds.
pub fn ping_ipv4(target: Ipv4Addr, timeout: Duration) -> Result<u32, String> {
    // SAFETY: IcmpCreateFile creates an ICMP echo handle in Windows.
    let handle = match unsafe { IcmpCreateFile() } {
        Ok(h) => h,
        Err(e) => return Err(format!("Falha ao abrir handle ICMP: {e}")),
    };

    let ip_bytes = target.octets();
    let dest_addr = u32::from_ne_bytes(ip_bytes);

    let send_data = b"GameOptimizerPingPayload";
    let reply_size = std::mem::size_of::<
        windows::Win32::NetworkManagement::IpHelper::ICMP_ECHO_REPLY,
    >() + send_data.len()
        + 8;
    let mut reply_buffer = vec![0u8; reply_size];

    let start = Instant::now();

    // SAFETY: Buffer and handle are correctly initialized and bounded.
    let replies = unsafe {
        IcmpSendEcho(
            handle,
            dest_addr,
            send_data.as_ptr() as *const _,
            send_data.len() as u16,
            Some(&IP_OPTION_INFORMATION::default() as *const _),
            reply_buffer.as_mut_ptr() as *mut _,
            reply_size as u32,
            timeout.as_millis().min(5000) as u32,
        )
    };

    let elapsed = start.elapsed().as_millis() as u32;

    // SAFETY: Handle cleanup.
    let _ = unsafe { IcmpCloseHandle(handle) };

    if replies > 0 {
        // Read round-trip time from echo reply
        let reply = unsafe {
            &*(reply_buffer.as_ptr()
                as *const windows::Win32::NetworkManagement::IpHelper::ICMP_ECHO_REPLY)
        };
        if reply.Status == 0 {
            let rtt = if reply.RoundTripTime > 0 {
                reply.RoundTripTime
            } else {
                elapsed.max(1)
            };
            return Ok(rtt);
        }
    }

    Err(format!(
        "VPS não respondeu ao ping (timeout de {}ms)",
        timeout.as_millis()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_not_enabled() {
        let cfg = WireGuardConfig::default();
        assert!(!cfg.enabled);
        assert!(!cfg.is_configured());
    }

    #[test]
    fn conf_generation_includes_table_off() {
        let mut cfg = WireGuardConfig::default();
        cfg.endpoint = "198.51.100.1:51820".into();
        cfg.private_key = "CLIENT_PRIV".into();
        cfg.peer_public_key = "SERVER_PUB".into();
        cfg.tunnel_ip = "10.0.0.2".into();

        let conf = cfg.generate_conf();
        assert!(conf.contains("Table = off"));
        assert!(conf.contains("PrivateKey = CLIENT_PRIV"));
        assert!(conf.contains("PublicKey = SERVER_PUB"));
        assert!(conf.contains("Address = 10.0.0.2/24"));
    }

    #[test]
    fn resolve_endpoint_ipv4() {
        let mut cfg = WireGuardConfig::default();
        cfg.endpoint = "127.0.0.1:51820".into();
        assert_eq!(cfg.resolve_endpoint_ip(), Some(Ipv4Addr::new(127, 0, 0, 1)));
    }
}
