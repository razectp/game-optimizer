//! Configuration and game allowlist loading.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Runtime settings and game name allowlist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppConfig {
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
    /// Optional path the config was loaded from.
    pub source_path: Option<PathBuf>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
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
            trim_non_game_memory: true,
            memory_trim_threshold_bytes: 200 * 1024 * 1024,
            max_trim_candidates: 12,
            source_path: None,
        }
    }
}

impl AppConfig {
    /// Load `games.toml` if present; otherwise return defaults.
    pub fn load_or_default(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(path)?;
        let file: ConfigFile = toml::from_str(&raw)?;
        Ok(Self::from_file(file, Some(path.to_path_buf())))
    }

    fn from_file(file: ConfigFile, source_path: Option<PathBuf>) -> Self {
        let mut names = GameCatalog::builtin_names();
        for name in file.games.names {
            names.insert(normalize_process_name(&name));
        }
        let mut never_trim = GameCatalog::builtin_never_trim_names();
        for name in file.settings.never_trim_names.unwrap_or_default() {
            never_trim.insert(normalize_process_name(&name));
        }
        let mut reclaim = GameCatalog::builtin_reclaim_names();
        for name in file.settings.reclaim_names.unwrap_or_default() {
            reclaim.insert(normalize_process_name(&name));
        }
        let threshold_mb = file.settings.memory_trim_threshold_mb.unwrap_or(200);
        Self {
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
            trim_non_game_memory: file.settings.trim_non_game_memory.unwrap_or(true),
            memory_trim_threshold_bytes: threshold_mb.saturating_mul(1024 * 1024),
            max_trim_candidates: file.settings.max_trim_candidates.unwrap_or(12),
            source_path,
        }
    }
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
}

/// `[games]` table.
#[derive(Debug, Default, Deserialize, Serialize)]
pub struct GamesSection {
    /// Extra process names (with or without `.exe`).
    #[serde(default)]
    pub names: Vec<String>,
}

/// `[settings]` table.
#[derive(Debug, Default, Deserialize, Serialize)]
pub struct SettingsSection {
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
            },
            settings: SettingsSection::default(),
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
}
