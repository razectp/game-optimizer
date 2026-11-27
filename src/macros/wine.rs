//! Start a small Win32 bridge in an existing Wine prefix.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::macros::{build_key_lparam, find_key_spec};

const PROGRAM: &[u8] = include_bytes!("../../assets/wine-macro-bridge.exe");
static NEXT_BRIDGE: AtomicU64 = AtomicU64::new(0);

pub(super) struct WineLaunch {
    executable: PathBuf,
    environment: Vec<(OsString, OsString)>,
}

impl WineLaunch {
    pub(super) fn for_pid(pid: u32) -> Option<Self> {
        let proc = PathBuf::from(format!("/proc/{pid}"));
        let image = fs::read_link(proc.join("exe")).ok()?;
        let image_name = image.file_name()?.to_str()?;
        let runtime = image.parent()?;
        let loader = match image_name {
            "wine64" | "wine64-preloader" => "wine64",
            "wine" | "wine-preloader" => "wine",
            _ => return None,
        };
        let executable = runtime.join(loader);
        if !executable.is_file() {
            return None;
        }
        let raw = fs::read(proc.join("environ")).ok()?;
        let environment: Vec<_> = raw
            .split(|&byte| byte == 0)
            .filter_map(|entry| {
                let delimiter = entry.iter().position(|&byte| byte == b'=')?;
                let name = &entry[..delimiter];
                let value = &entry[delimiter + 1..];
                matches!(
                    name,
                    b"WINEPREFIX"
                        | b"WINEDLLPATH"
                        | b"WINELOADER"
                        | b"WINESERVER"
                        | b"WINEESYNC"
                        | b"WINEFSYNC"
                        | b"WINEARCH"
                        | b"WINELOADERNOEXEC"
                        | b"LD_LIBRARY_PATH"
                        | b"PATH"
                        | b"DISPLAY"
                        | b"XAUTHORITY"
                        | b"XDG_RUNTIME_DIR"
                )
                .then(|| {
                    (
                        OsStr::from_bytes(name).to_owned(),
                        OsStr::from_bytes(value).to_owned(),
                    )
                })
            })
            .collect();
        let prefix = environment.iter().find(|(name, _)| name == "WINEPREFIX")?;
        if !Path::new(&prefix.1).is_absolute() || !Path::new(&prefix.1).is_dir() {
            return None;
        }
        Some(Self {
            executable,
            environment,
        })
    }
}

struct BridgeFiles(PathBuf);

impl BridgeFiles {
    fn new() -> io::Result<Self> {
        let directory = std::env::temp_dir().join(format!(
            "game-optimizer-macro-{}-{}",
            std::process::id(),
            NEXT_BRIDGE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new().mode(0o700).create(&directory)?;
        let files = Self(directory);
        fs::write(files.executable(), PROGRAM)?;
        Ok(files)
    }

    fn executable(&self) -> PathBuf {
        self.0.join("wine-macro-bridge.exe")
    }
}

impl Drop for BridgeFiles {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.executable());
        let _ = fs::remove_dir(&self.0);
    }
}

struct Bridge {
    child: Option<Child>,
    input: Option<ChildStdin>,
    output: ChildStdout,
    files: Option<BridgeFiles>,
    started: Instant,
    ready: bool,
}

impl Bridge {
    fn spawn(launch: &WineLaunch) -> io::Result<Self> {
        let files = BridgeFiles::new()?;
        let mut child = Command::new(&launch.executable)
            .envs(launch.environment.iter().cloned())
            .env("WINEDEBUG", "-all")
            .env(crate::macros::WINE_BRIDGE_MARKER, "1")
            .arg(files.executable())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let input = child.stdin.take().expect("piped stdin");
        let output = child.stdout.take().expect("piped stdout");
        Ok(Self {
            child: Some(child),
            input: Some(input),
            output,
            files: Some(files),
            started: Instant::now(),
            ready: false,
        })
    }

    fn receive(&mut self, timeout_ms: libc::c_int) -> io::Result<Option<u8>> {
        let mut fd = libc::pollfd {
            fd: self.output.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: fd refers to the open child stdout and poll receives one valid pollfd.
        let result = unsafe { libc::poll(&mut fd, 1, timeout_ms) };
        if result == 0 {
            return Ok(None);
        }
        if result < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut value = [0];
        self.output.read_exact(&mut value)?;
        Ok(Some(value[0]))
    }

    fn is_ready(&mut self) -> io::Result<bool> {
        if !self.ready {
            match self.receive(0)? {
                Some(b'R') => self.ready = true,
                Some(_) => return Err(io::ErrorKind::InvalidData.into()),
                None if self.started.elapsed() >= Duration::from_secs(6) => {
                    return Err(io::ErrorKind::TimedOut.into());
                }
                None => {}
            }
        }
        Ok(self.ready)
    }

    fn post(&mut self, request: [u32; 4]) -> io::Result<bool> {
        let mut bytes = [0; 16];
        for (part, value) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(request) {
            part.copy_from_slice(&value.to_le_bytes());
        }
        self.input
            .as_mut()
            .expect("bridge stdin")
            .write_all(&bytes)?;
        match self.receive(250)? {
            Some(b'1') => Ok(true),
            Some(b'0') => Ok(false),
            _ => Err(io::ErrorKind::TimedOut.into()),
        }
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        self.input.take();
        if let Some(mut child) = self.child.take() {
            let files = self.files.take();
            std::thread::spawn(move || {
                let _ = child.wait();
                drop(files);
            });
        }
    }
}

#[derive(Default)]
pub(super) struct WineBridges {
    active: HashMap<u32, Bridge>,
    retry: HashMap<u32, Instant>,
}

impl WineBridges {
    pub(super) fn ready(&mut self, pid: u32, launch: &WineLaunch) -> bool {
        if self
            .retry
            .get(&pid)
            .is_some_and(|due| Instant::now() < *due)
        {
            return false;
        }
        if let std::collections::hash_map::Entry::Vacant(entry) = self.active.entry(pid) {
            match Bridge::spawn(launch) {
                Ok(bridge) => {
                    entry.insert(bridge);
                }
                Err(_) => {
                    self.retry
                        .insert(pid, Instant::now() + Duration::from_secs(2));
                    return false;
                }
            }
        }
        match self
            .active
            .get_mut(&pid)
            .expect("bridge prepared")
            .is_ready()
        {
            Ok(ready) => ready,
            Err(_) => {
                self.retire(pid);
                false
            }
        }
    }

    pub(super) fn send(&mut self, pid: u32, window: u32, code: &str, down: bool) -> bool {
        let Some(spec) = find_key_spec(code) else {
            return false;
        };
        let (message, wparam, lparam) = match (spec.code, down) {
            ("MOUSE_LEFT", true) => (0x0201, 1, 0),
            ("MOUSE_LEFT", false) => (0x0202, 0, 0),
            ("MOUSE_RIGHT", true) => (0x0204, 2, 0),
            ("MOUSE_RIGHT", false) => (0x0205, 0, 0),
            (_, down) => (
                if down { 0x0100 } else { 0x0101 },
                u32::from(spec.vk),
                build_key_lparam(spec.scan_code, spec.extended, !down) as u32,
            ),
        };
        let Some(bridge) = self.active.get_mut(&pid).filter(|bridge| bridge.ready) else {
            return false;
        };
        match bridge.post([window, message, wparam, lparam]) {
            Ok(sent) => sent,
            Err(_) => {
                self.retire(pid);
                false
            }
        }
    }

    pub(super) fn retain(&mut self, mut live: impl FnMut(u32) -> bool) {
        self.active.retain(|&pid, _| live(pid));
        self.retry.retain(|&pid, _| live(pid));
    }

    fn retire(&mut self, pid: u32) {
        self.active.remove(&pid);
        self.retry
            .insert(pid, Instant::now() + Duration::from_secs(2));
    }
}
