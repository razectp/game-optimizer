//! WireGuard tunnel configuration and game traffic routing.

/// Autonomous userspace WireGuard engine (BoringTun + Wintun).
#[cfg(windows)]
pub mod embedded_engine;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
mod linux_game_group;
#[cfg(target_os = "linux")]
mod linux_reconnect;
/// Central tunnel coordinator and lifecycle manager.
pub mod manager;
/// Windows routing table controller for host routes.
#[cfg(windows)]
pub mod routes;
/// Windows TCP and UDP socket connection inspector.
#[cfg(windows)]
pub mod sockets;
/// WireGuard configuration and ICMP ping integration.
pub mod wireguard;

#[cfg(windows)]
pub use embedded_engine::{start_embedded_tunnel, EmbeddedTunnelHandle};
pub use manager::{TunnelHealth, TunnelManager, TunnelPassReport, TunnelStatus};
#[cfg(windows)]
pub use routes::RouteManager;
#[cfg(windows)]
pub use sockets::{find_game_remote_ips, is_routable_game_ip};
pub use wireguard::{ping_ipv4, WireGuardConfig};

/// An active network connection owned by a monitored game PID.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProcessConnection {
    /// Owning OS process ID.
    pub pid: u32,
    /// Transport protocol (TCP or UDP).
    pub protocol: &'static str,
    /// Local bound port.
    pub local_port: u16,
    /// Remote IPv4 address.
    pub remote_ip: std::net::Ipv4Addr,
    /// Remote port.
    pub remote_port: u16,
}

#[cfg(target_os = "linux")]
pub use linux::run_gui_tunnel_helper;
#[cfg(target_os = "linux")]
pub use linux::watch_tunnel;
#[cfg(target_os = "linux")]
pub(crate) use linux::{is_scoped_game, GuiTunnel};
#[cfg(target_os = "linux")]
pub(crate) use linux_game_group::GameProcess;
