//! Assign detected Linux game processes to the tunnel's cgroup.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct GameProcess {
    pub(crate) pid: u32,
    pub(super) start: u64,
}

impl GameProcess {
    pub(crate) fn read(pid: u32) -> Option<Self> {
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let start = stat
            .rsplit_once(')')?
            .1
            .split_whitespace()
            .nth(19)?
            .parse()
            .ok()?;
        Some(Self { pid, start })
    }

    pub(super) fn still_owned_by(self, uid: u32) -> bool {
        if Self::read(self.pid) != Some(self) {
            return false;
        }
        fs::read_to_string(format!("/proc/{}/status", self.pid)).is_ok_and(|status| {
            status.lines().any(|line| {
                line.strip_prefix("Uid:").is_some_and(|ids| {
                    let mut ids = ids.split_whitespace();
                    ids.by_ref()
                        .take(2)
                        .map(str::parse::<u32>)
                        .eq([Ok(uid), Ok(uid)])
                })
            })
        })
    }
}

pub(super) fn scope_path(uid: u32) -> PathBuf {
    PathBuf::from(format!(
        "/sys/fs/cgroup/user.slice/user-{uid}.slice/user@{uid}.service/gameoptimizer.slice"
    ))
}

fn process_cgroup(pid: u32) -> Option<PathBuf> {
    let text = fs::read_to_string(format!("/proc/{pid}/cgroup")).ok()?;
    let path = text.lines().find_map(|line| line.strip_prefix("0::/"))?;
    let relative = Path::new(path);
    relative
        .components()
        .all(|part| matches!(part, Component::Normal(_)))
        .then(|| Path::new("/sys/fs/cgroup").join(relative))
}

pub(super) fn is_scoped_game(pid: u32, uid: u32) -> bool {
    process_cgroup(pid).is_some_and(|path| {
        path.strip_prefix(scope_path(uid))
            .ok()
            .and_then(|relative| relative.components().next())
            .is_some_and(|component| component.as_os_str().as_bytes().starts_with(b"managed-"))
    })
}

pub(super) fn same_wine_broker(pid: u32, uid: u32, games: &[GameProcess]) -> bool {
    let Some(broker) = GameProcess::read(pid).filter(|broker| broker.still_owned_by(uid)) else {
        return false;
    };
    let Ok(exe) = fs::read_link(format!("/proc/{}/exe", broker.pid)) else {
        return false;
    };
    if !matches!(
        exe.file_name().and_then(|name| name.to_str()),
        Some("wineserver" | "wineserver64" | "wineserver32")
    ) {
        return false;
    }
    let Some(prefix) = wine_prefix(broker.pid) else {
        return false;
    };
    games.iter().any(|game| {
        game.still_owned_by(uid) && wine_prefix(game.pid).is_some_and(|other| other == prefix)
    })
}

fn wine_prefix(pid: u32) -> Option<Vec<u8>> {
    let environment = fs::read(format!("/proc/{pid}/environ")).ok()?;
    let prefix = environment
        .split(|&byte| byte == 0)
        .find_map(|item| item.strip_prefix(b"WINEPREFIX="))?;
    let prefix = Path::new(std::ffi::OsStr::from_bytes(prefix));
    let canonical = prefix.is_absolute().then(|| prefix.canonicalize().ok())??;
    Some(canonical.as_os_str().as_bytes().to_vec())
}

fn wine_servers(
    uid: u32,
    prefixes: &HashSet<Vec<u8>>,
    games: &HashSet<GameProcess>,
) -> Vec<GameProcess> {
    let mut servers = Vec::new();
    let mut shared = HashSet::new();
    let installs: Vec<_> = games
        .iter()
        .filter_map(|game| Some((wine_prefix(game.pid)?, wine_command(game.pid)?)))
        .collect();
    for entry in fs::read_dir("/proc").into_iter().flatten().flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse().ok())
        else {
            continue;
        };
        let Ok(exe) = fs::read_link(entry.path().join("exe")) else {
            continue;
        };
        let Some(name) = exe.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.starts_with("wine") {
            continue;
        }
        let Some(process) = GameProcess::read(pid).filter(|process| process.still_owned_by(uid))
        else {
            continue;
        };
        let Some(prefix) = wine_prefix(pid).filter(|prefix| prefixes.contains(prefix)) else {
            continue;
        };
        if matches!(name, "wineserver" | "wineserver64" | "wineserver32") {
            servers.push((process, prefix));
        } else if !games.contains(&process)
            && wine_command(pid).is_none_or(|command| {
                is_wine_application(&command)
                    && !is_optimizer_wine_bridge(pid, &command)
                    && !installs.iter().any(|(game_prefix, game)| {
                        *game_prefix == prefix && belongs_to_game_install(&command, game)
                    })
            })
        {
            shared.insert(prefix);
        }
    }
    servers
        .into_iter()
        .filter_map(|(server, prefix)| (!shared.contains(&prefix)).then_some(server))
        .collect()
}

fn wine_command(pid: u32) -> Option<String> {
    let cmdline = fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let argv0 = cmdline.split(|&byte| byte == 0).next()?;
    Some(
        String::from_utf8_lossy(argv0)
            .to_ascii_lowercase()
            .replace('\\', "/"),
    )
}

fn is_wine_application(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or_default();
    name.ends_with(".exe")
        && !path.contains("/windows/system32/")
        && !path.contains("/windows/syswow64/")
        && !matches!(
            name,
            "winedevice.exe"
                | "wineboot.exe"
                | "services.exe"
                | "explorer.exe"
                | "rpcss.exe"
                | "winemenubuilder.exe"
                | "start.exe"
                | "plugplay.exe"
        )
}

fn is_optimizer_wine_bridge(pid: u32, command: &str) -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        // The bridge shares the Wine prefix for window messages only.
        if !command.ends_with("/wine-macro-bridge.exe")
            || !command.contains("/game-optimizer-macro-")
        {
            return false;
        }
        let marker = format!("{}=1", crate::macros::WINE_BRIDGE_MARKER);
        fs::read(format!("/proc/{pid}/environ")).is_ok_and(|environment| {
            environment
                .split(|&byte| byte == 0)
                .any(|entry| entry == marker.as_bytes())
        })
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = (pid, command);
        false
    }
}

fn belongs_to_game_install(helper: &str, game: &str) -> bool {
    if let Some((directory, _)) = game.rsplit_once('/') {
        if helper
            .strip_prefix(directory)
            .is_some_and(|suffix| suffix.starts_with('/'))
        {
            return true;
        }
    }
    // Unreal places shared engine helpers and the game binary in sibling trees.
    let Some((engine_root, _)) = helper.split_once("/engine/binaries/") else {
        return false;
    };
    let Some((game_module, _)) = game.split_once("/binaries/") else {
        return false;
    };
    game_module
        .rsplit_once('/')
        .is_some_and(|(root, _)| root == engine_root)
}

#[derive(Debug)]
pub(super) struct GameGroup {
    uid: u32,
    path: PathBuf,
    original: HashMap<GameProcess, PathBuf>,
}

impl GameGroup {
    pub(super) fn new(uid: u32) -> io::Result<Self> {
        let path = scope_path(uid).join(format!("managed-{}", std::process::id()));
        fs::create_dir(&path)?;
        Ok(Self {
            uid,
            path,
            original: HashMap::new(),
        })
    }

    pub(super) fn cgroup(&self) -> &Path {
        self.path
            .strip_prefix("/sys/fs/cgroup")
            .expect("inside cgroupfs")
    }

    pub(super) fn uid(&self) -> u32 {
        self.uid
    }

    pub(super) fn contains_pid(&self, pid: u32) -> bool {
        process_cgroup(pid).as_deref() == Some(self.path.as_path())
    }

    pub(super) fn contains_game(&self, game: GameProcess) -> bool {
        game.still_owned_by(self.uid) && self.contains_pid(game.pid)
    }

    pub(super) fn synchronize(
        &mut self,
        detected: &[GameProcess],
        local: &[GameProcess],
    ) -> (Vec<GameProcess>, io::Result<()>) {
        let games: HashSet<_> = detected
            .iter()
            .copied()
            .filter(|game| game.still_owned_by(self.uid))
            .collect();
        let local_prefixes: HashSet<_> = local
            .iter()
            .filter(|game| game.still_owned_by(self.uid))
            .filter_map(|game| wine_prefix(game.pid))
            .collect();
        let prefixes: HashSet<_> = games
            .iter()
            .filter_map(|game| wine_prefix(game.pid))
            .filter(|prefix| !local_prefixes.contains(prefix))
            .collect();
        let servers = if prefixes.is_empty() {
            Vec::new()
        } else {
            wine_servers(self.uid, &prefixes, &games)
        };
        let mut selected = games;
        selected.extend(servers);

        let mut released = Vec::new();
        let mut error = None;
        self.original.retain(|game, original| {
            if selected.contains(game) {
                return true;
            }
            if !game.still_owned_by(self.uid)
                || process_cgroup(game.pid).as_deref() != Some(self.path.as_path())
            {
                return false;
            }
            if restore(*game, self.uid, &self.path, original) {
                released.push(*game);
                return false;
            }
            error = Some(io::Error::other(
                "não foi possível retirar um processo do grupo",
            ));
            true
        });
        for game in selected {
            let Some(current) = process_cgroup(game.pid) else {
                continue;
            };
            if current == self.path || !game.still_owned_by(self.uid) {
                continue;
            }
            match move_process(game.pid, &self.path) {
                Ok(()) => {
                    self.original.insert(game, current);
                }
                Err(err) if game.still_owned_by(self.uid) => error = Some(err),
                Err(_) => {}
            }
        }
        (released, error.map_or(Ok(()), Err))
    }
}

impl Drop for GameGroup {
    fn drop(&mut self) {
        for (&game, original) in &self.original {
            restore(game, self.uid, &self.path, original);
        }
        let _ = fs::remove_dir(&self.path);
    }
}

fn move_process(pid: u32, to: &Path) -> io::Result<()> {
    use std::io::Write;
    let mut procs = fs::OpenOptions::new()
        .write(true)
        .open(to.join("cgroup.procs"))?;
    procs.write_all(pid.to_string().as_bytes())
}

fn restore(game: GameProcess, uid: u32, managed: &Path, original: &Path) -> bool {
    if !game.still_owned_by(uid) || process_cgroup(game.pid).as_deref() != Some(managed) {
        return true;
    }
    // A transient source scope may disappear once its last process is moved.
    // Use an unmarked sibling if the original is unavailable.
    if move_process(game.pid, original).is_ok() {
        return true;
    }
    let Some(slice) = managed.parent() else {
        return false;
    };
    let released = slice.join("released");
    if let Err(err) = fs::create_dir(&released) {
        if err.kind() != io::ErrorKind::AlreadyExists {
            return false;
        }
    }
    game.still_owned_by(uid) && move_process(game.pid, &released).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_helpers_share_their_games_installation() {
        let game = "c:/games/example/client/module/binaries/win64/game.exe";
        assert!(belongs_to_game_install(
            "c:/games/example/client/engine/binaries/win64/webhelper.exe",
            game
        ));
        assert!(!belongs_to_game_install(
            "c:/games/other/client/engine/binaries/win64/webhelper.exe",
            game
        ));
    }
}
