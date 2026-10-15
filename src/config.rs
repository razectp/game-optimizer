//! Configuration and game allowlist loading.

use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::tunnel::WireGuardConfig;

/// Runtime settings and game name allowlist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppConfig {
    /// GUI preferences persisted alongside optimizer settings.
    pub gui: GuiPreferences,
    /// Process names treated as games (lowercase, without `.exe`).
    pub game_names: BTreeSet<String>,
    /// Non-game names that must never be working-set trimmed (editors/shells/encoders).
    pub never_trim_names: BTreeSet<String>,
    /// Non-game names allowed for working-set trim (browsers only by default).
    pub reclaim_names: BTreeSet<String>,
    /// Whether to raise game process priority to High.
    pub boost_priority: bool,
    /// Whether to disable power throttling / EcoQoS on games.
    pub disable_power_throttling: bool,
    /// Whether to raise Windows memory priority for games (keeps pages resident).
    pub raise_memory_priority: bool,
    /// Whether to keep dynamic priority boosts enabled on games.
    pub enable_priority_boost: bool,
    /// Whether to run GPC CPU-set partitioning on performance cores.
    pub prefer_performance_cores: bool,
    /// Soft working-set floor as percent of current game WS (0 disables).
    pub residency_floor_percent: u8,
    /// Empty the game working set (pages cold memory out; may hitch briefly).
    pub trim_game_memory: bool,
    /// Empty large **browser** working sets (Chrome, Edge, Firefox, …) to free RAM for games.
    pub trim_non_game_memory: bool,
    /// Minimum working set (bytes) before a non-game is a trim candidate.
    pub memory_trim_threshold_bytes: u64,
    /// Cap on how many non-game processes are trimmed per pass.
    pub max_trim_candidates: usize,
    /// Embedded WireGuard VPS tunneling configuration.
    pub wireguard: WireGuardConfig,
    /// Optional path the config was loaded from.
    pub source_path: Option<PathBuf>,
}

/// User interface preferences persisted in `[gui]`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GuiPreferences {
    /// Process watcher interval in seconds (15–120).
    pub watch_interval_secs: u64,
    /// Whether process watching starts enabled.
    pub watch_enabled: bool,
    /// Window width in pixels (minimum 640).
    pub window_width: u32,
    /// Window height in pixels (minimum 420).
    pub window_height: u32,
    /// Whether the window was maximized.
    pub window_maximized: bool,
}

impl Default for GuiPreferences {
    fn default() -> Self {
        Self {
            watch_interval_secs: 45,
            watch_enabled: false,
            window_width: 880,
            window_height: 560,
            window_maximized: false,
        }
    }
}

impl GuiPreferences {
    /// Clamp persisted preferences to safe operational dimensions and interval.
    pub fn normalized(mut self) -> Self {
        self.watch_interval_secs = self.watch_interval_secs.clamp(15, 120);
        self.window_width = self.window_width.clamp(640, 16384);
        self.window_height = self.window_height.clamp(420, 16384);
        self
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            gui: GuiPreferences::default(),
            game_names: GameCatalog::builtin_names(),
            never_trim_names: GameCatalog::builtin_never_trim_names(),
            reclaim_names: GameCatalog::builtin_reclaim_names(),
            boost_priority: true,
            disable_power_throttling: true,
            raise_memory_priority: true,
            enable_priority_boost: true,
            prefer_performance_cores: true,
            residency_floor_percent: 60,
            trim_game_memory: false,
            trim_non_game_memory: false,
            memory_trim_threshold_bytes: 200 * 1024 * 1024,
            max_trim_candidates: 12,
            wireguard: WireGuardConfig::default(),
            source_path: None,
        }
    }
}

impl AppConfig {
    /// Return the standard path in user's profile `%LOCALAPPDATA%\GameOptimizer\config.toml`.
    pub fn user_config_path() -> PathBuf {
        crate::autostart::data_dir().join("config.toml")
    }

    /// Load the explicitly supplied existing path first, then the user config and legacy paths.
    pub fn load_or_default(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let user_path = Self::user_config_path();
        if path == user_path {
            if user_path.exists() {
                let raw = fs::read_to_string(&user_path)?;
                let file: ConfigFile = toml::from_str(&raw)?;
                return Ok(Self::from_file(file, Some(user_path)));
            }
        } else if path.exists() {
            let raw = fs::read_to_string(path)?;
            let file: ConfigFile = toml::from_str(&raw)?;
            return Ok(Self::from_file(file, Some(path.to_path_buf())));
        } else if user_path.exists() {
            let raw = fs::read_to_string(&user_path)?;
            let file: ConfigFile = toml::from_str(&raw)?;
            return Ok(Self::from_file(file, Some(user_path)));
        }

        let local_sibling = path.with_file_name("games.local.toml");
        let target_path = if local_sibling.exists() {
            Some(local_sibling)
        } else if path.exists() {
            Some(path.to_path_buf())
        } else if let Ok(exe_path) = std::env::current_exe() {
            if let Some(parent) = exe_path.parent() {
                let local_in_exe = parent.join("games.local.toml");
                if local_in_exe.exists() {
                    Some(local_in_exe)
                } else {
                    let candidate = parent.join(path);
                    if candidate.exists() {
                        Some(candidate)
                    } else {
                        None
                    }
                }
            } else {
                None
            }
        } else {
            None
        };

        if let Some(p) = target_path {
            let raw = fs::read_to_string(&p)?;
            let file: ConfigFile = toml::from_str(&raw)?;
            return Ok(Self::from_file(file, Some(p)));
        }

        Ok(Self::default())
    }

    /// Save configuration directly to `%LOCALAPPDATA%\GameOptimizer\config.toml`.
    pub fn save_to_user_dir(&self) -> anyhow::Result<PathBuf> {
        let target = Self::user_config_path();
        self.save_to_path(&target)?;
        Ok(target)
    }

    /// Save current configuration back to a TOML file.
    pub fn save_to_path(&self, path: impl AsRef<Path>) -> anyhow::Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        let file = ConfigFile {
            games: GamesSection {
                names: self.game_names.iter().cloned().collect(),
                snapshot: Some(true),
            },
            settings: SettingsSection {
                boost_priority: Some(self.boost_priority),
                disable_power_throttling: Some(self.disable_power_throttling),
                raise_memory_priority: Some(self.raise_memory_priority),
                enable_priority_boost: Some(self.enable_priority_boost),
                prefer_performance_cores: Some(self.prefer_performance_cores),
                residency_floor_percent: Some(self.residency_floor_percent),
                trim_game_memory: Some(self.trim_game_memory),
                trim_non_game_memory: Some(self.trim_non_game_memory),
                memory_trim_threshold_mb: Some(self.memory_trim_threshold_bytes / (1024 * 1024)),
                max_trim_candidates: Some(self.max_trim_candidates),
                never_trim_names: Some(self.never_trim_names.iter().cloned().collect()),
                reclaim_names: Some(self.reclaim_names.iter().cloned().collect()),
                lists_snapshot: Some(true),
            },
            wireguard: Some(self.wireguard.clone()),
            gui: Some(self.gui.clone().normalized()),
        };
        let raw = toml::to_string_pretty(&file)?;
        atomic_write(path, raw.as_bytes())
    }

    /// Return user-added game names (excluding the builtin catalog).
    pub fn user_custom_games(&self) -> Vec<String> {
        let builtin = GameCatalog::builtin_names();
        self.game_names
            .iter()
            .filter(|n| !builtin.contains(*n))
            .cloned()
            .collect()
    }

    /// Add a game process name (e.g. "Client-Win64-Shipping.exe" or "mir4").
    pub fn add_game_name(&mut self, name: &str) -> bool {
        let normalized = normalize_process_name(name);
        if normalized.is_empty() {
            return false;
        }
        self.game_names.insert(normalized)
    }

    /// Remove a game process name if present.
    pub fn remove_game_name(&mut self, name: &str) -> bool {
        let normalized = normalize_process_name(name);
        self.game_names.remove(&normalized)
    }

    /// Add a process to the reclaim (memory trim allowed) list.
    pub fn add_reclaim_name(&mut self, name: &str) -> bool {
        let normalized = normalize_process_name(name);
        if normalized.is_empty() {
            return false;
        }
        self.reclaim_names.insert(normalized)
    }

    /// Remove a process from the reclaim list.
    pub fn remove_reclaim_name(&mut self, name: &str) -> bool {
        let normalized = normalize_process_name(name);
        self.reclaim_names.remove(&normalized)
    }

    /// Add a process to the never-trim protection list.
    pub fn add_never_trim_name(&mut self, name: &str) -> bool {
        let normalized = normalize_process_name(name);
        if normalized.is_empty() {
            return false;
        }
        self.never_trim_names.insert(normalized)
    }

    /// Remove a process from the never-trim protection list.
    pub fn remove_never_trim_name(&mut self, name: &str) -> bool {
        let normalized = normalize_process_name(name);
        self.never_trim_names.remove(&normalized)
    }

    fn from_file(file: ConfigFile, source_path: Option<PathBuf>) -> Self {
        let mut names = if file.games.snapshot.unwrap_or(false) {
            BTreeSet::new()
        } else {
            GameCatalog::builtin_names()
        };
        for name in file.games.names {
            names.insert(normalize_process_name(&name));
        }
        let mut never_trim = if file.settings.lists_snapshot.unwrap_or(false) {
            BTreeSet::new()
        } else {
            GameCatalog::builtin_never_trim_names()
        };
        for name in file.settings.never_trim_names.unwrap_or_default() {
            never_trim.insert(normalize_process_name(&name));
        }
        let mut reclaim = if file.settings.lists_snapshot.unwrap_or(false) {
            BTreeSet::new()
        } else {
            GameCatalog::builtin_reclaim_names()
        };
        for name in file.settings.reclaim_names.unwrap_or_default() {
            reclaim.insert(normalize_process_name(&name));
        }
        let threshold_mb = file.settings.memory_trim_threshold_mb.unwrap_or(200);
        Self {
            gui: file.gui.unwrap_or_default().normalized(),
            game_names: names,
            never_trim_names: never_trim,
            reclaim_names: reclaim,
            boost_priority: file.settings.boost_priority.unwrap_or(true),
            disable_power_throttling: file.settings.disable_power_throttling.unwrap_or(true),
            raise_memory_priority: file.settings.raise_memory_priority.unwrap_or(true),
            enable_priority_boost: file.settings.enable_priority_boost.unwrap_or(true),
            prefer_performance_cores: file.settings.prefer_performance_cores.unwrap_or(true),
            residency_floor_percent: file.settings.residency_floor_percent.unwrap_or(60).min(90),
            trim_game_memory: file.settings.trim_game_memory.unwrap_or(false),
            trim_non_game_memory: file.settings.trim_non_game_memory.unwrap_or(false),
            memory_trim_threshold_bytes: threshold_mb.saturating_mul(1024 * 1024),
            max_trim_candidates: file.settings.max_trim_candidates.unwrap_or(12),
            wireguard: file.wireguard.unwrap_or_default(),
            source_path,
        }
    }
}

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

fn atomic_write(path: &Path, contents: &[u8]) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("configuration path has no file name"))?;
    let mut temp_path = None;
    let mut file = None;
    for _ in 0..100 {
        let id = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!(
            ".{}.{}.{}.tmp",
            name.to_string_lossy(),
            std::process::id(),
            id
        ));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(created) => {
                temp_path = Some(candidate);
                file = Some(created);
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    let temp_path = temp_path
        .ok_or_else(|| anyhow::anyhow!("could not allocate unique temporary config file"))?;
    let result = (|| -> anyhow::Result<()> {
        let mut file = file.expect("temporary file created with path");
        file.write_all(contents)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp_path, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

/// Built-in known game process names.
pub struct GameCatalog;

impl GameCatalog {
    /// Default allowlist of known game process names.
    pub fn builtin_names() -> BTreeSet<String> {
        [
            "mir4g",
            "mir4",
            "mir4launcher",
            "wemade",
            "eldenring",
            "gta5",
            "gtav",
            "rdr2",
            "cs2",
            "valorant",
            "valorant-win64-shipping",
            "fortniteclient-win64-shipping",
            "league of legends",
            "leagueclient",
            "leagueclientux",
            "overwatch",
            "wow",
            "wowclassic",
            "dota2",
            "hl2",
            "csgo",
            "rocketleague",
            "apex_legends",
            "r5apex",
            "destiny2",
            "pathofexile",
            "pathofexile_x64",
            "pathofexilesteam",
            "lostark",
            "blackdesert64",
            "ffxiv_dx11",
            "genshinimpact",
            "starrail",
            "zenlesszonezero",
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    }

    /// Editors, shells, and media tools that should keep their working set.
    pub fn builtin_never_trim_names() -> BTreeSet<String> {
        [
            "cursor",
            "code",
            "code - insiders",
            "devenv",
            "idea64",
            "rider64",
            "windowsterminal",
            "powershell",
            "pwsh",
            "cmd",
            "windows terminal",
            "ffmpeg",
            "ffplay",
            "ffprobe",
            "handbrake",
            "handbrakecli",
            "obs64",
            "obs32",
            "obs",
            "afterfx",
            "blender",
            "capcut",
            "shotcut",
            "kdenlive",
            "resolve",
            "davinci resolve",
            "adobe premiere pro",
            "premiere pro",
            "adobe media encoder",
            "nvencc",
            "qsvencc",
            "vceencc",
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    }

    /// Browsers whose working set may be emptied to free RAM for games.
    pub fn builtin_reclaim_names() -> BTreeSet<String> {
        [
            "chrome",
            "chromium",
            "msedge",
            "microsoftedge",
            "firefox",
            "firefoxdeveloperedition",
            "librewolf",
            "waterfox",
            "brave",
            "opera",
            "operagx",
            "vivaldi",
            "thorium",
            "floorp",
            "palemoon",
            "seamonkey",
            "iexplore",
            "plugin-container",
            "arc",
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    }
}

/// On-disk TOML shape.
#[derive(Debug, Default, Deserialize, Serialize)]
pub struct ConfigFile {
    /// Game allowlist section.
    #[serde(default)]
    pub games: GamesSection,
    /// Optimizer toggles.
    #[serde(default)]
    pub settings: SettingsSection,
    /// WireGuard embedded tunnel section.
    #[serde(default)]
    pub wireguard: Option<WireGuardConfig>,
    /// GUI preferences.
    #[serde(default)]
    pub gui: Option<GuiPreferences>,
}

/// `[games]` table.
#[derive(Debug, Default, Deserialize, Serialize)]
pub struct GamesSection {
    /// Extra process names (with or without `.exe`).
    #[serde(default)]
    pub names: Vec<String>,
    /// When true, `names` is a complete list including removals from built-ins.
    #[serde(default)]
    pub snapshot: Option<bool>,
}

/// `[settings]` table.
#[derive(Debug, Default, Deserialize, Serialize)]
pub struct SettingsSection {
    /// When true, the two name arrays below are complete lists.
    #[serde(default)]
    pub lists_snapshot: Option<bool>,
    /// Raise game priority to High.
    pub boost_priority: Option<bool>,
    /// Disable Windows execution-speed power throttling on games.
    pub disable_power_throttling: Option<bool>,
    /// Raise memory priority so the OS prefers keeping game pages in RAM.
    pub raise_memory_priority: Option<bool>,
    /// Keep dynamic thread priority boosts enabled.
    pub enable_priority_boost: Option<bool>,
    /// Prefer performance CPU sets and partition them across same-name instances.
    pub prefer_performance_cores: Option<bool>,
    /// Soft working-set floor percent of current game WS (0–90).
    pub residency_floor_percent: Option<u8>,
    /// Empty game working sets (may cause brief hitching when pages fault back).
    pub trim_game_memory: Option<bool>,
    /// Empty large **browser** working sets (Chrome/Edge/…).
    pub trim_non_game_memory: Option<bool>,
    /// Non-game WS threshold in MiB for trim candidates.
    pub memory_trim_threshold_mb: Option<u64>,
    /// Max non-game processes trimmed per pass.
    pub max_trim_candidates: Option<usize>,
    /// Extra process names never trimmed.
    pub never_trim_names: Option<Vec<String>>,
    /// Extra process names allowed for non-game working-set trim.
    pub reclaim_names: Option<Vec<String>>,
}

/// Normalize a process name for allowlist comparison.
pub fn normalize_process_name(name: &str) -> String {
    let trimmed = name.trim().to_lowercase();
    trimmed.strip_suffix(".exe").unwrap_or(&trimmed).to_string()
}

/// Path fragments that strongly suggest a game install tree.
pub fn game_path_markers() -> &'static [&'static str] {
    &[
        r"\steam\steamapps\common\",
        r"\epic games\",
        r"\xboxgames\",
        r"\wemade\",
        r"\mir4\",
        r"\riot games\",
        r"\ubisoft\ubisoft game launcher\games\",
        r"\ea games\",
        r"\origin games\",
        r"\gog galaxy\games\",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sandbox() -> PathBuf {
        let root = std::env::temp_dir().join("opencode");
        fs::create_dir_all(&root).unwrap();
        let dir = root.join(format!(
            "config-tests-{}-{}",
            std::process::id(),
            TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        dir
    }

    #[test]
    fn normalize_strips_exe_and_case() {
        assert_eq!(normalize_process_name("SampleGame.EXE"), "samplegame");
        assert_eq!(normalize_process_name("  samplegame  "), "samplegame");
    }

    #[test]
    fn builtin_includes_common_titles() {
        let names = GameCatalog::builtin_names();
        assert!(names.contains("mir4g"));
        assert!(names.contains("cs2"));
    }

    #[test]
    fn config_merges_user_names() {
        let file = ConfigFile {
            games: GamesSection {
                names: vec!["MyGame.EXE".into()],
                ..GamesSection::default()
            },
            settings: SettingsSection::default(),
            wireguard: None,
            gui: None,
        };
        let cfg = AppConfig::from_file(file, None);
        assert!(cfg.game_names.contains("mygame"));
        assert!(cfg.game_names.contains("cs2"));
        assert!(cfg.raise_memory_priority);
        assert!(cfg.prefer_performance_cores);
        assert!(cfg.reclaim_names.contains("chrome"));
        assert!(cfg.never_trim_names.contains("ffmpeg"));
    }

    #[test]
    fn extra_reclaim_names_merge_from_toml() {
        let file = ConfigFile {
            games: GamesSection::default(),
            settings: SettingsSection {
                reclaim_names: Some(vec!["MyBrowser.EXE".into()]),
                ..SettingsSection::default()
            },
            wireguard: None,
            gui: None,
        };
        let cfg = AppConfig::from_file(file, None);
        assert!(cfg.reclaim_names.contains("mybrowser"));
        assert!(cfg.reclaim_names.contains("chrome"));
        assert!(!cfg.reclaim_names.contains("ffmpeg"));
    }

    #[test]
    fn reclaim_allowlist_is_browsers_not_encoders() {
        let reclaim = GameCatalog::builtin_reclaim_names();
        assert!(reclaim.contains("chrome"));
        assert!(reclaim.contains("msedge"));
        assert!(reclaim.contains("firefox"));
        assert!(!reclaim.contains("ffmpeg"));
        assert!(!reclaim.contains("obs64"));
    }

    #[test]
    fn config_includes_wireguard() {
        let wg = WireGuardConfig {
            enabled: true,
            endpoint: "1.2.3.4:51820".into(),
            ..WireGuardConfig::default()
        };
        let file = ConfigFile {
            wireguard: Some(wg),
            ..ConfigFile::default()
        };

        let cfg = AppConfig::from_file(file, None);
        assert!(cfg.wireguard.enabled);
        assert_eq!(cfg.wireguard.endpoint, "1.2.3.4:51820");
    }

    #[test]
    fn custom_game_and_trim_management() {
        let mut cfg = AppConfig::default();
        assert!(!cfg
            .user_custom_games()
            .contains(&"mycustomgame".to_string()));

        assert!(cfg.add_game_name("MyCustomGame.exe"));
        assert!(cfg
            .user_custom_games()
            .contains(&"mycustomgame".to_string()));
        assert!(cfg.game_names.contains("mycustomgame"));

        assert!(cfg.remove_game_name("MyCustomGame"));
        assert!(!cfg
            .user_custom_games()
            .contains(&"mycustomgame".to_string()));

        cfg.add_reclaim_name("CustomBrowser.exe");
        assert!(cfg.reclaim_names.contains("custombrowser"));
        cfg.remove_reclaim_name("custombrowser");
        assert!(!cfg.reclaim_names.contains("custombrowser"));

        cfg.add_never_trim_name("CustomProtected.exe");
        assert!(cfg.never_trim_names.contains("customprotected"));
        cfg.remove_never_trim_name("customprotected");
        assert!(!cfg.never_trim_names.contains("customprotected"));
    }

    #[test]
    fn user_config_path_ends_with_config_toml() {
        let path = AppConfig::user_config_path();
        assert!(path.ends_with("config.toml"));
    }

    #[test]
    fn gui_defaults_and_normalization() {
        assert_eq!(GuiPreferences::default().watch_interval_secs, 45);
        let normalized = GuiPreferences {
            watch_interval_secs: 1,
            window_width: 1,
            window_height: u32::MAX,
            ..Default::default()
        }
        .normalized();
        assert_eq!(normalized.watch_interval_secs, 15);
        assert_eq!(normalized.window_width, 640);
        assert_eq!(normalized.window_height, 16384);
    }

    #[test]
    fn old_file_defaults_and_roundtrip_preserves_list_removals_and_additions() {
        let dir = sandbox();
        let path = dir.join("config.toml");
        fs::write(
            &path,
            "[games]\nnames = [\"custom.exe\"]\n[settings]\nboost_priority = false\n",
        )
        .unwrap();
        let mut cfg = AppConfig::load_or_default(&path).unwrap();
        assert_eq!(cfg.gui, GuiPreferences::default());
        assert!(!cfg.boost_priority);
        cfg.remove_game_name("cs2");
        cfg.add_game_name("new-game");
        cfg.remove_reclaim_name("chrome");
        cfg.add_reclaim_name("custom-browser");
        cfg.remove_never_trim_name("ffmpeg");
        cfg.add_never_trim_name("custom-protected");
        cfg.gui.watch_enabled = true;
        cfg.gui.watch_interval_secs = 73;
        cfg.gui.window_width = 1024;
        cfg.gui.window_height = 768;
        cfg.gui.window_maximized = true;
        cfg.wireguard.enabled = true;
        cfg.wireguard.endpoint = "198.51.100.10:51820".into();
        cfg.save_to_path(&path).unwrap();
        let loaded = AppConfig::load_or_default(&path).unwrap();
        assert!(!loaded.game_names.contains("cs2"));
        assert!(loaded.game_names.contains("new-game"));
        assert!(!loaded.reclaim_names.contains("chrome"));
        assert!(loaded.reclaim_names.contains("custom-browser"));
        assert!(!loaded.never_trim_names.contains("ffmpeg"));
        assert!(loaded.never_trim_names.contains("custom-protected"));
        assert!(loaded.gui.watch_enabled);
        assert_eq!(loaded.gui.watch_interval_secs, 73);
        assert_eq!(
            (loaded.gui.window_width, loaded.gui.window_height),
            (1024, 768)
        );
        assert!(loaded.gui.window_maximized);
        assert!(loaded.wireguard.enabled);
        assert_eq!(loaded.wireguard.endpoint, "198.51.100.10:51820");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn atomic_write_failure_keeps_target_directory_and_cleans_temp() {
        let dir = sandbox();
        let path = dir.join("config.toml");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("sentinel"), "preserve").unwrap();
        assert!(atomic_write(&path, b"replacement").is_err());
        assert_eq!(
            fs::read_to_string(path.join("sentinel")).unwrap(),
            "preserve"
        );
        assert_eq!(
            fs::read_dir(&dir).unwrap().count(),
            1,
            "failed rename must remove its temporary file"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn atomic_write_replaces_existing_file() {
        let dir = sandbox();
        let path = dir.join("config.toml");
        fs::write(&path, "before").unwrap();
        atomic_write(&path, b"after").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"after");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn save_creates_file_in_existing_sandbox_directory() {
        let dir = sandbox();
        let path = dir.join("config.toml");
        AppConfig::default().save_to_path(&path).unwrap();
        assert!(path.exists());
        fs::remove_dir_all(dir).unwrap();
    }
}
