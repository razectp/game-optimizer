//! Per-process network socket inspector for Windows.
//!
//! Queries active TCP and UDP sockets owned by target game PIDs.

#![allow(unsafe_code)]

use std::collections::HashSet;
use std::net::Ipv4Addr;

use windows::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, GetExtendedUdpTable, SetTcpEntry, MIB_TCPROW_LH, TCP_TABLE_OWNER_PID_ALL,
    UDP_TABLE_OWNER_PID,
};
use windows::Win32::Networking::WinSock::AF_INET;

/// An active network connection owned by a monitored game PID.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProcessConnection {
    /// Owning OS process ID.
    pub pid: u32,
    /// Protocol used (TCP or UDP).
    pub protocol: &'static str,
    /// Local bound port.
    pub local_port: u16,
    /// Remote IPv4 address.
    pub remote_ip: Ipv4Addr,
    /// Remote port.
    pub remote_port: u16,
}

#[repr(C)]
struct MibTcpRowOwnerPid {
    state: u32,
    local_addr: u32,
    local_port: u32,
    remote_addr: u32,
    remote_port: u32,
    owning_pid: u32,
}

#[repr(C)]
struct MibUdpRowOwnerPid {
    local_addr: u32,
    local_port: u32,
    owning_pid: u32,
}

/// Reset an active game TCP connection to trigger immediate client reconnection
/// over the newly created tunnel route (ExitLag behavior).
/// Strictly ignores web/OAuth ports (443, 80, etc.) to prevent login disruptions.
pub fn reset_tcp_connection(pid: u32, remote_ip: Ipv4Addr) -> bool {
    let mut size: u32 = 0;
    let _ = unsafe {
        GetExtendedTcpTable(
            None,
            &mut size,
            false,
            AF_INET.0 as u32,
            TCP_TABLE_OWNER_PID_ALL,
            0,
        )
    };
    if size == 0 {
        return false;
    }

    let mut buffer = vec![0u8; size as usize];
    let res = unsafe {
        GetExtendedTcpTable(
            Some(buffer.as_mut_ptr() as *mut _),
            &mut size,
            false,
            AF_INET.0 as u32,
            TCP_TABLE_OWNER_PID_ALL,
            0,
        )
    };
    if res != 0 {
        return false;
    }

    let num_entries = unsafe { *(buffer.as_ptr() as *const u32) };
    let row_ptr =
        unsafe { buffer.as_ptr().add(std::mem::size_of::<u32>()) as *const MibTcpRowOwnerPid };

    for i in 0..num_entries as usize {
        let row = unsafe { &*row_ptr.add(i) };
        if row.owning_pid != pid {
            continue;
        }

        let r_ip = Ipv4Addr::from(u32::from_be(row.remote_addr));
        let r_port = u16::from_be((row.remote_port & 0xFFFF) as u16);
        // Only reset dedicated game server ports. NEVER reset web/OAuth/auth ports (443, 80, etc.)!
        if r_ip == remote_ip && is_game_port(r_port) {
            // Convert to MIB_TCPROW_LH and set State to 12 (MIB_TCP_STATE_DELETE_TCB)
            let mut close_row = MIB_TCPROW_LH::default();
            close_row.Anonymous.dwState = 12;
            close_row.dwLocalAddr = row.local_addr;
            close_row.dwLocalPort = row.local_port;
            close_row.dwRemoteAddr = row.remote_addr;
            close_row.dwRemotePort = row.remote_port;
            let ret = unsafe { SetTcpEntry(&close_row) };
            return ret == 0;
        }
    }

    false
}

/// Discover remote IPv4 destinations for the given target game PIDs.
pub fn find_game_remote_ips(target_pids: &HashSet<u32>) -> Vec<ProcessConnection> {
    if target_pids.is_empty() {
        return Vec::new();
    }

    let mut connections = Vec::new();
    collect_tcp_connections(target_pids, &mut connections);
    collect_udp_connections(target_pids, &mut connections);
    connections
}

fn collect_tcp_connections(target_pids: &HashSet<u32>, out: &mut Vec<ProcessConnection>) {
    let mut size: u32 = 0;
    // SAFETY: First call retrieves the required buffer size.
    let _res = unsafe {
        GetExtendedTcpTable(
            None,
            &mut size,
            false,
            AF_INET.0 as u32,
            TCP_TABLE_OWNER_PID_ALL,
            0,
        )
    };
    if size == 0 {
        return;
    }

    let mut buffer = vec![0u8; size as usize];
    // SAFETY: Buffer is appropriately sized as requested by the initial call.
    let res = unsafe {
        GetExtendedTcpTable(
            Some(buffer.as_mut_ptr() as *mut _),
            &mut size,
            false,
            AF_INET.0 as u32,
            TCP_TABLE_OWNER_PID_ALL,
            0,
        )
    };

    if res != 0 {
        return;
    }

    let num_entries = unsafe { *(buffer.as_ptr() as *const u32) };
    let row_ptr =
        unsafe { buffer.as_ptr().add(std::mem::size_of::<u32>()) as *const MibTcpRowOwnerPid };

    for i in 0..num_entries as usize {
        let row = unsafe { &*row_ptr.add(i) };
        if !target_pids.contains(&row.owning_pid) {
            continue;
        }

        // Ignore listening, closed, or time-wait sockets
        if row.state == 1 || row.state == 2 || row.state == 11 {
            continue;
        }

        let remote_ip = Ipv4Addr::from(u32::from_be(row.remote_addr));
        let remote_port = u16::from_be((row.remote_port & 0xFFFF) as u16);
        let local_port = u16::from_be((row.local_port & 0xFFFF) as u16);

        // Filter out non-routable IPs and web/auth/CDN ports (80, 443, 8080, 8443)
        // Web logins (Google OAuth, Apple ID, WeMade auth) MUST stay on the native local connection
        // to prevent login page freezes, OAuth verification failures, and "reconnect" errors.
        if is_game_port(remote_port) && is_routable_game_ip(&remote_ip) {
            out.push(ProcessConnection {
                pid: row.owning_pid,
                protocol: "TCP",
                local_port,
                remote_ip,
                remote_port,
            });
        }
    }
}

/// Check if a network port is a dedicated game server port.
/// Web ports (80, 443, 8080, 8443) and DNS (53) are excluded because they carry
/// web logins (Google OAuth, Apple ID, WeMade auth), patch CDNs, and browser assets.
/// Routing or resetting web ports causes OAuth disconnects, login screen freezes, and CDN bandwidth waste.
pub fn is_game_port(port: u16) -> bool {
    !matches!(port, 53 | 80 | 443 | 8080 | 8443)
}

fn collect_udp_connections(target_pids: &HashSet<u32>, _out: &mut Vec<ProcessConnection>) {
    let mut size: u32 = 0;
    // SAFETY: First call retrieves buffer size.
    unsafe {
        GetExtendedUdpTable(
            None,
            &mut size,
            false,
            AF_INET.0 as u32,
            UDP_TABLE_OWNER_PID,
            0,
        );
    };
    if size == 0 {
        return;
    }

    let mut buffer = vec![0u8; size as usize];
    let res = unsafe {
        GetExtendedUdpTable(
            Some(buffer.as_mut_ptr() as *mut _),
            &mut size,
            false,
            AF_INET.0 as u32,
            UDP_TABLE_OWNER_PID,
            0,
        )
    };

    if res != 0 {
        return;
    }

    let num_entries = unsafe { *(buffer.as_ptr() as *const u32) };
    let row_ptr =
        unsafe { buffer.as_ptr().add(std::mem::size_of::<u32>()) as *const MibUdpRowOwnerPid };

    for i in 0..num_entries as usize {
        let row = unsafe { &*row_ptr.add(i) };
        if !target_pids.contains(&row.owning_pid) {
            continue;
        }

        let local_port = u16::from_be((row.local_port & 0xFFFF) as u16);
        // UDP row has local_addr and local_port.
        // For connected UDP sockets, some systems provide remote addr, but for standard UDP
        // sockets local_port informs which port the game is bound to.
        let _ = local_port;
    }
}

/// Check if an IP address is a public, routable game server IP.
/// Filters out loopback (127.0.0.1), zeroes (0.0.0.0), multicast, link-local, and private LAN.
pub fn is_routable_game_ip(ip: &Ipv4Addr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() || ip.is_broadcast() {
        return false;
    }

    let octets = ip.octets();
    // 10.0.0.0/8
    if octets[0] == 10 {
        return false;
    }
    // 172.16.0.0/12
    if octets[0] == 172 && (16..=31).contains(&octets[1]) {
        return false;
    }
    // 192.168.0.0/16
    if octets[0] == 192 && octets[1] == 168 {
        return false;
    }
    // 169.254.0.0/16 (Link-local)
    if octets[0] == 169 && octets[1] == 254 {
        return false;
    }

    true
}
