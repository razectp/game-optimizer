//! Embedded WireGuard and per-PID game connection tunneling.
//!
//! Provides process-isolated network routing so only games use the VPS tunnel.

/// Autonomous userspace WireGuard engine (BoringTun + Wintun).
pub mod embedded_engine;
/// Central tunnel coordinator and lifecycle manager.
pub mod manager;
/// Windows routing table controller for host routes.
pub mod routes;
/// Windows TCP and UDP socket connection inspector.
pub mod sockets;
/// WireGuard configuration and ICMP ping integration.
pub mod wireguard;

pub use embedded_engine::{start_embedded_tunnel, EmbeddedTunnelHandle};
pub use manager::{TunnelHealth, TunnelManager, TunnelPassReport, TunnelStatus};
pub use routes::RouteManager;
pub use sockets::{find_game_remote_ips, is_routable_game_ip, ProcessConnection};
pub use wireguard::{ping_ipv4, WireGuardConfig};
