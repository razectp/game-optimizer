//! Migrate established Linux game connections once the new socket route is ready.

use std::collections::HashSet;
use std::fs;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::Path;

use super::linux::run;
use super::linux_game_group::{same_wine_broker, GameGroup, GameProcess};

pub(super) const WEB_AND_DNS_PORTS: &[u16] = &[53, 80, 443, 8080, 8443];

struct Connection<'a> {
    local: SocketAddrV4,
    remote: SocketAddrV4,
    inode: u64,
    cgroup: &'a str,
    owners: Vec<u32>,
}

impl<'a> Connection<'a> {
    fn parse(line: &'a str) -> Option<Self> {
        let mut fields = line.split_ascii_whitespace().skip(2);
        let local = fields.next()?.parse().ok()?;
        let remote = fields.next()?.parse().ok()?;
        let inode = fields
            .find_map(|field| field.strip_prefix("ino:"))?
            .parse()
            .ok()?;
        let cgroup = fields.find_map(|field| field.strip_prefix("cgroup:"))?;
        if !Path::new(cgroup).is_absolute() {
            return None;
        }
        let owners = line
            .split(",pid=")
            .skip(1)
            .filter_map(|field| field.split_once(",fd=")?.0.parse().ok())
            .collect();
        Some(Self {
            local,
            remote,
            inode,
            cgroup,
            owners,
        })
    }
}

fn eligible_destination(addr: SocketAddrV4, endpoint: Ipv4Addr) -> bool {
    let ip = *addr.ip();
    ip != endpoint
        && ip.octets()[0] != 0
        && ip.octets()[0] < 224
        && !ip.is_loopback()
        && !ip.is_private()
        && !ip.is_link_local()
        && !WEB_AND_DNS_PORTS.contains(&addr.port())
}

fn game_inodes(games: &[GameProcess]) -> HashSet<u64> {
    games
        .iter()
        .flat_map(|game| {
            fs::read_dir(format!("/proc/{}/fd", game.pid))
                .into_iter()
                .flatten()
        })
        .filter_map(|entry| fs::read_link(entry.ok()?.path()).ok())
        .filter_map(|link| {
            link.to_str()?
                .strip_prefix("socket:[")?
                .strip_suffix(']')?
                .parse()
                .ok()
        })
        .collect()
}

#[derive(Clone, Copy)]
pub(super) enum Route {
    Tunnel,
    Local,
}

impl Route {
    fn previous_socket_is_eligible(self, cgroup: &str, group: &Path) -> bool {
        match self {
            Self::Tunnel => true,
            Self::Local => Path::new(cgroup)
                .strip_prefix("/")
                .is_ok_and(|path| path.starts_with(group)),
        }
    }
}

/// Request a fresh socket for each eligible, previously established game connection.
pub(super) fn reconnect(
    games: &[GameProcess],
    group: &GameGroup,
    endpoint: Ipv4Addr,
    route: Route,
) -> Result<usize, String> {
    let games: Vec<_> = games
        .iter()
        .copied()
        .filter(|game| game.still_owned_by(group.uid()))
        .collect();
    let inodes = game_inodes(&games);
    if inodes.is_empty() {
        return Ok(0);
    }
    let list = run(
        "ss",
        &[
            "-H",
            "-O",
            "-4",
            "-t",
            "-n",
            "-e",
            "-p",
            "--cgroup",
            "state",
            "established",
        ],
    )?;
    let targets_game = |pid| {
        games
            .iter()
            .any(|game| game.pid == pid && game.still_owned_by(group.uid()))
    };
    let mut closed = 0;
    for connection in list.lines().filter_map(Connection::parse) {
        if !inodes.contains(&connection.inode)
            || !route.previous_socket_is_eligible(connection.cgroup, group.cgroup())
            || !connection.owners.iter().copied().any(targets_game)
            || !connection.owners.iter().all(|&pid| {
                let belongs_to_route = match route {
                    Route::Tunnel => group.contains_pid(pid) && targets_game(pid),
                    Route::Local => targets_game(pid),
                };
                belongs_to_route || same_wine_broker(pid, group.uid(), &games)
            })
            || !eligible_destination(connection.remote, endpoint)
        {
            continue;
        }

        // Constrain the kernel query to this exact connection and its old cgroup.
        // ss uses the kernel's socket cookie to identify the socket it closes.
        let output = run(
            "ss",
            &[
                "-K",
                "-H",
                "-4",
                "-t",
                "-n",
                "state",
                "established",
                "src",
                &connection.local.ip().to_string(),
                "sport",
                "=",
                &format!(":{}", connection.local.port()),
                "dst",
                &connection.remote.ip().to_string(),
                "dport",
                "=",
                &format!(":{}", connection.remote.port()),
                "cgroup",
                "=",
                connection.cgroup,
            ],
        )?;
        closed += usize::from(!output.trim().is_empty());
    }
    Ok(closed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_game_socket_and_all_wine_owners() {
        let socket = Connection::parse(
            "0 0 192.168.1.2:48100 192.0.2.85:33000 users:((\"game\",pid=42,fd=7),(\"wineserver\",pid=17,fd=9)) uid:1000 ino:4200 sk:4 cgroup:/user.slice/session.scope <-> cgroup:/user.slice/session.scope",
        ).unwrap();
        assert_eq!(socket.owners, [42, 17]);
        assert_eq!(socket.inode, 4200);
        assert_eq!(socket.cgroup, "/user.slice/session.scope");
        assert!(eligible_destination(
            socket.remote,
            "192.0.2.10".parse().unwrap()
        ));
    }

    #[test]
    fn reconnects_managed_sockets_and_leaves_unrelated_local_sockets_on_opt_out() {
        let group = Path::new("example/managed");
        let managed = "/example/managed";
        let local = "/example/local";
        assert!(Route::Tunnel.previous_socket_is_eligible(managed, group));
        assert!(Route::Tunnel.previous_socket_is_eligible(local, group));
        assert!(Route::Local.previous_socket_is_eligible(managed, group));
        assert!(!Route::Local.previous_socket_is_eligible(local, group));
    }

    #[test]
    fn does_not_restart_login_local_or_endpoint_connections() {
        let endpoint = "192.0.2.10".parse().unwrap();
        for address in ["192.0.2.20:443", "192.168.1.4:33000", "192.0.2.10:33000"] {
            assert!(!eligible_destination(address.parse().unwrap(), endpoint));
        }
    }
}
