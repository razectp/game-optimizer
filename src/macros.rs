//! Autonomous per-window hotkey and macro automation for monitored games.
//!
//! Sends keyboard and mouse messages (`WM_KEYDOWN` / `WM_KEYUP`, button clicks)
//! directly to the target game process window (`HWND`) via `PostMessageW` so each
//! game instance operates autonomously in the background without stealing focus
//! or registering global system hotkeys.

#![allow(unsafe_code)]

use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Minimum allowed interval between macro steps or key presses (ms).
pub const MIN_MACRO_INTERVAL_MS: u64 = 10;
/// Default interval between sequential macro steps (ms).
pub const DEFAULT_MACRO_INTERVAL_MS: u64 = 50;
/// Default interval between full sequence triggers/cycles (ms) — 5 seconds.
pub const DEFAULT_CYCLE_INTERVAL_MS: u64 = 5_000;
/// Default in-window hotkey to toggle (start/pause) macros for the focused game window.
pub const DEFAULT_TOGGLE_KEY: &str = "F8";
/// Maximum allowed interval between macro steps or key presses (ms) — 1 hour.
pub const MAX_MACRO_INTERVAL_MS: u64 = 3_600_000;
/// Minimum key hold duration before sending `WM_KEYUP` (ms).
pub const MIN_MACRO_HOLD_MS: u64 = 5;
/// Default key hold duration before sending `WM_KEYUP` (ms).
pub const DEFAULT_MACRO_HOLD_MS: u64 = 15;
/// Maximum key hold duration before sending `WM_KEYUP` (ms).
pub const MAX_MACRO_HOLD_MS: u64 = 5_000;

/// Metadata describing a supported virtual key or mouse button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeySpec {
    /// Canonical identifier stored in configuration (e.g. `"F1"`, `"SPACE"`).
    pub code: &'static str,
    /// Friendly label displayed in the Portuguese GUI.
    pub label: &'static str,
    /// Category label used to group keys in the selector.
    pub category: &'static str,
    /// Win32 Virtual-Key code (`VK_*`), or `0` for mouse buttons.
    pub vk: u16,
    /// Hardware Set-1 scan code embedded in `lParam`.
    pub scan_code: u8,
    /// Whether the key uses the extended-key flag (`bit 24` in `lParam`).
    pub extended: bool,
}

/// Complete catalog of keys and mouse buttons available for game macros.
pub const SUPPORTED_KEYS: &[KeySpec] = &[
    // Function keys
    KeySpec {
        code: "F1",
        label: "F1",
        category: "Teclas de Função",
        vk: 0x70,
        scan_code: 0x3B,
        extended: false,
    },
    KeySpec {
        code: "F2",
        label: "F2",
        category: "Teclas de Função",
        vk: 0x71,
        scan_code: 0x3C,
        extended: false,
    },
    KeySpec {
        code: "F3",
        label: "F3",
        category: "Teclas de Função",
        vk: 0x72,
        scan_code: 0x3D,
        extended: false,
    },
    KeySpec {
        code: "F4",
        label: "F4",
        category: "Teclas de Função",
        vk: 0x73,
        scan_code: 0x3E,
        extended: false,
    },
    KeySpec {
        code: "F5",
        label: "F5",
        category: "Teclas de Função",
        vk: 0x74,
        scan_code: 0x3F,
        extended: false,
    },
    KeySpec {
        code: "F6",
        label: "F6",
        category: "Teclas de Função",
        vk: 0x75,
        scan_code: 0x40,
        extended: false,
    },
    KeySpec {
        code: "F7",
        label: "F7",
        category: "Teclas de Função",
        vk: 0x76,
        scan_code: 0x41,
        extended: false,
    },
    KeySpec {
        code: "F8",
        label: "F8",
        category: "Teclas de Função",
        vk: 0x77,
        scan_code: 0x42,
        extended: false,
    },
    KeySpec {
        code: "F9",
        label: "F9",
        category: "Teclas de Função",
        vk: 0x78,
        scan_code: 0x43,
        extended: false,
    },
    KeySpec {
        code: "F10",
        label: "F10",
        category: "Teclas de Função",
        vk: 0x79,
        scan_code: 0x44,
        extended: false,
    },
    KeySpec {
        code: "F11",
        label: "F11",
        category: "Teclas de Função",
        vk: 0x7A,
        scan_code: 0x57,
        extended: false,
    },
    KeySpec {
        code: "F12",
        label: "F12",
        category: "Teclas de Função",
        vk: 0x7B,
        scan_code: 0x58,
        extended: false,
    },
    // Number row
    KeySpec {
        code: "1",
        label: "Tecla 1",
        category: "Números",
        vk: 0x31,
        scan_code: 0x02,
        extended: false,
    },
    KeySpec {
        code: "2",
        label: "Tecla 2",
        category: "Números",
        vk: 0x32,
        scan_code: 0x03,
        extended: false,
    },
    KeySpec {
        code: "3",
        label: "Tecla 3",
        category: "Números",
        vk: 0x33,
        scan_code: 0x04,
        extended: false,
    },
    KeySpec {
        code: "4",
        label: "Tecla 4",
        category: "Números",
        vk: 0x34,
        scan_code: 0x05,
        extended: false,
    },
    KeySpec {
        code: "5",
        label: "Tecla 5",
        category: "Números",
        vk: 0x35,
        scan_code: 0x06,
        extended: false,
    },
    KeySpec {
        code: "6",
        label: "Tecla 6",
        category: "Números",
        vk: 0x36,
        scan_code: 0x07,
        extended: false,
    },
    KeySpec {
        code: "7",
        label: "Tecla 7",
        category: "Números",
        vk: 0x37,
        scan_code: 0x08,
        extended: false,
    },
    KeySpec {
        code: "8",
        label: "Tecla 8",
        category: "Números",
        vk: 0x38,
        scan_code: 0x09,
        extended: false,
    },
    KeySpec {
        code: "9",
        label: "Tecla 9",
        category: "Números",
        vk: 0x39,
        scan_code: 0x0A,
        extended: false,
    },
    KeySpec {
        code: "0",
        label: "Tecla 0",
        category: "Números",
        vk: 0x30,
        scan_code: 0x0B,
        extended: false,
    },
    // Common action keys
    KeySpec {
        code: "SPACE",
        label: "Espaço",
        category: "Teclas Especiais",
        vk: 0x20,
        scan_code: 0x39,
        extended: false,
    },
    KeySpec {
        code: "TAB",
        label: "Tab",
        category: "Teclas Especiais",
        vk: 0x09,
        scan_code: 0x0F,
        extended: false,
    },
    KeySpec {
        code: "ENTER",
        label: "Enter",
        category: "Teclas Especiais",
        vk: 0x0D,
        scan_code: 0x1C,
        extended: false,
    },
    KeySpec {
        code: "SHIFT",
        label: "Shift",
        category: "Teclas Especiais",
        vk: 0x10,
        scan_code: 0x2A,
        extended: false,
    },
    KeySpec {
        code: "CTRL",
        label: "Ctrl",
        category: "Teclas Especiais",
        vk: 0x11,
        scan_code: 0x1D,
        extended: false,
    },
    KeySpec {
        code: "ALT",
        label: "Alt",
        category: "Teclas Especiais",
        vk: 0x12,
        scan_code: 0x38,
        extended: false,
    },
    KeySpec {
        code: "ESC",
        label: "Esc",
        category: "Teclas Especiais",
        vk: 0x1B,
        scan_code: 0x01,
        extended: false,
    },
    KeySpec {
        code: "BACKSPACE",
        label: "Backspace",
        category: "Teclas Especiais",
        vk: 0x08,
        scan_code: 0x0E,
        extended: false,
    },
    // Letters
    KeySpec {
        code: "Q",
        label: "Tecla Q",
        category: "Letras",
        vk: 0x51,
        scan_code: 0x10,
        extended: false,
    },
    KeySpec {
        code: "W",
        label: "Tecla W",
        category: "Letras",
        vk: 0x57,
        scan_code: 0x11,
        extended: false,
    },
    KeySpec {
        code: "E",
        label: "Tecla E",
        category: "Letras",
        vk: 0x45,
        scan_code: 0x12,
        extended: false,
    },
    KeySpec {
        code: "R",
        label: "Tecla R",
        category: "Letras",
        vk: 0x52,
        scan_code: 0x13,
        extended: false,
    },
    KeySpec {
        code: "T",
        label: "Tecla T",
        category: "Letras",
        vk: 0x54,
        scan_code: 0x14,
        extended: false,
    },
    KeySpec {
        code: "Y",
        label: "Tecla Y",
        category: "Letras",
        vk: 0x59,
        scan_code: 0x15,
        extended: false,
    },
    KeySpec {
        code: "U",
        label: "Tecla U",
        category: "Letras",
        vk: 0x55,
        scan_code: 0x16,
        extended: false,
    },
    KeySpec {
        code: "I",
        label: "Tecla I",
        category: "Letras",
        vk: 0x49,
        scan_code: 0x17,
        extended: false,
    },
    KeySpec {
        code: "O",
        label: "Tecla O",
        category: "Letras",
        vk: 0x4F,
        scan_code: 0x18,
        extended: false,
    },
    KeySpec {
        code: "P",
        label: "Tecla P",
        category: "Letras",
        vk: 0x50,
        scan_code: 0x19,
        extended: false,
    },
    KeySpec {
        code: "A",
        label: "Tecla A",
        category: "Letras",
        vk: 0x41,
        scan_code: 0x1E,
        extended: false,
    },
    KeySpec {
        code: "S",
        label: "Tecla S",
        category: "Letras",
        vk: 0x53,
        scan_code: 0x1F,
        extended: false,
    },
    KeySpec {
        code: "D",
        label: "Tecla D",
        category: "Letras",
        vk: 0x44,
        scan_code: 0x20,
        extended: false,
    },
    KeySpec {
        code: "F",
        label: "Tecla F",
        category: "Letras",
        vk: 0x46,
        scan_code: 0x21,
        extended: false,
    },
    KeySpec {
        code: "G",
        label: "Tecla G",
        category: "Letras",
        vk: 0x47,
        scan_code: 0x22,
        extended: false,
    },
    KeySpec {
        code: "H",
        label: "Tecla H",
        category: "Letras",
        vk: 0x48,
        scan_code: 0x23,
        extended: false,
    },
    KeySpec {
        code: "J",
        label: "Tecla J",
        category: "Letras",
        vk: 0x4A,
        scan_code: 0x24,
        extended: false,
    },
    KeySpec {
        code: "K",
        label: "Tecla K",
        category: "Letras",
        vk: 0x4B,
        scan_code: 0x25,
        extended: false,
    },
    KeySpec {
        code: "L",
        label: "Tecla L",
        category: "Letras",
        vk: 0x4C,
        scan_code: 0x26,
        extended: false,
    },
    KeySpec {
        code: "Z",
        label: "Tecla Z",
        category: "Letras",
        vk: 0x5A,
        scan_code: 0x2C,
        extended: false,
    },
    KeySpec {
        code: "X",
        label: "Tecla X",
        category: "Letras",
        vk: 0x58,
        scan_code: 0x2D,
        extended: false,
    },
    KeySpec {
        code: "C",
        label: "Tecla C",
        category: "Letras",
        vk: 0x43,
        scan_code: 0x2E,
        extended: false,
    },
    KeySpec {
        code: "V",
        label: "Tecla V",
        category: "Letras",
        vk: 0x56,
        scan_code: 0x2F,
        extended: false,
    },
    KeySpec {
        code: "B",
        label: "Tecla B",
        category: "Letras",
        vk: 0x42,
        scan_code: 0x30,
        extended: false,
    },
    KeySpec {
        code: "N",
        label: "Tecla N",
        category: "Letras",
        vk: 0x4E,
        scan_code: 0x31,
        extended: false,
    },
    KeySpec {
        code: "M",
        label: "Tecla M",
        category: "Letras",
        vk: 0x4D,
        scan_code: 0x32,
        extended: false,
    },
    // Navigation & Arrows
    KeySpec {
        code: "UP",
        label: "Seta Cima (↑)",
        category: "Navegação & Setas",
        vk: 0x26,
        scan_code: 0x48,
        extended: true,
    },
    KeySpec {
        code: "DOWN",
        label: "Seta Baixo (↓)",
        category: "Navegação & Setas",
        vk: 0x28,
        scan_code: 0x50,
        extended: true,
    },
    KeySpec {
        code: "LEFT",
        label: "Seta Esquerda (←)",
        category: "Navegação & Setas",
        vk: 0x25,
        scan_code: 0x4B,
        extended: true,
    },
    KeySpec {
        code: "RIGHT",
        label: "Seta Direita (→)",
        category: "Navegação & Setas",
        vk: 0x27,
        scan_code: 0x4D,
        extended: true,
    },
    KeySpec {
        code: "INSERT",
        label: "Insert",
        category: "Navegação & Setas",
        vk: 0x2D,
        scan_code: 0x52,
        extended: true,
    },
    KeySpec {
        code: "DELETE",
        label: "Delete",
        category: "Navegação & Setas",
        vk: 0x2E,
        scan_code: 0x53,
        extended: true,
    },
    KeySpec {
        code: "HOME",
        label: "Home",
        category: "Navegação & Setas",
        vk: 0x24,
        scan_code: 0x47,
        extended: true,
    },
    KeySpec {
        code: "END",
        label: "End",
        category: "Navegação & Setas",
        vk: 0x23,
        scan_code: 0x4F,
        extended: true,
    },
    KeySpec {
        code: "PAGEUP",
        label: "Page Up",
        category: "Navegação & Setas",
        vk: 0x21,
        scan_code: 0x49,
        extended: true,
    },
    KeySpec {
        code: "PAGEDOWN",
        label: "Page Down",
        category: "Navegação & Setas",
        vk: 0x22,
        scan_code: 0x51,
        extended: true,
    },
    // Numpad
    KeySpec {
        code: "NUM0",
        label: "Numpad 0",
        category: "Teclado Numérico",
        vk: 0x60,
        scan_code: 0x52,
        extended: false,
    },
    KeySpec {
        code: "NUM1",
        label: "Numpad 1",
        category: "Teclado Numérico",
        vk: 0x61,
        scan_code: 0x4F,
        extended: false,
    },
    KeySpec {
        code: "NUM2",
        label: "Numpad 2",
        category: "Teclado Numérico",
        vk: 0x62,
        scan_code: 0x50,
        extended: false,
    },
    KeySpec {
        code: "NUM3",
        label: "Numpad 3",
        category: "Teclado Numérico",
        vk: 0x63,
        scan_code: 0x51,
        extended: false,
    },
    KeySpec {
        code: "NUM4",
        label: "Numpad 4",
        category: "Teclado Numérico",
        vk: 0x64,
        scan_code: 0x4B,
        extended: false,
    },
    KeySpec {
        code: "NUM5",
        label: "Numpad 5",
        category: "Teclado Numérico",
        vk: 0x65,
        scan_code: 0x4C,
        extended: false,
    },
    KeySpec {
        code: "NUM6",
        label: "Numpad 6",
        category: "Teclado Numérico",
        vk: 0x66,
        scan_code: 0x4D,
        extended: false,
    },
    KeySpec {
        code: "NUM7",
        label: "Numpad 7",
        category: "Teclado Numérico",
        vk: 0x67,
        scan_code: 0x47,
        extended: false,
    },
    KeySpec {
        code: "NUM8",
        label: "Numpad 8",
        category: "Teclado Numérico",
        vk: 0x68,
        scan_code: 0x48,
        extended: false,
    },
    KeySpec {
        code: "NUM9",
        label: "Numpad 9",
        category: "Teclado Numérico",
        vk: 0x69,
        scan_code: 0x49,
        extended: false,
    },
    // Mouse buttons
    KeySpec {
        code: "MOUSE_LEFT",
        label: "Clique Esquerdo (Mouse)",
        category: "Mouse",
        vk: 0,
        scan_code: 0,
        extended: false,
    },
    KeySpec {
        code: "MOUSE_RIGHT",
        label: "Clique Direito (Mouse)",
        category: "Mouse",
        vk: 0,
        scan_code: 0,
        extended: false,
    },
];

/// Look up a [`KeySpec`] by its canonical code (case-insensitive).
pub fn find_key_spec(code: &str) -> Option<&'static KeySpec> {
    let trimmed = code.trim();
    SUPPORTED_KEYS
        .iter()
        .find(|spec| spec.code.eq_ignore_ascii_case(trimmed))
}

/// Return the friendly label for a key code, or a placeholder if unset.
pub fn key_display_label(code: &str) -> &str {
    if code.trim().is_empty() {
        return "Selecionar tecla...";
    }
    find_key_spec(code).map(|s| s.label).unwrap_or(code)
}

/// Build the Win32 `lParam` bitfield for `WM_KEYDOWN` / `WM_KEYUP`.
pub fn build_key_lparam(scan_code: u8, extended: bool, is_key_up: bool) -> isize {
    let mut lparam: isize = 1; // repeat count = 1
    lparam |= (scan_code as isize) << 16;
    if extended {
        lparam |= 1 << 24;
    }
    if is_key_up {
        lparam |= (1 << 30) | (1 << 31);
    }
    lparam
}

/// Execution mode for a game's macro list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MacroExecutionMode {
    /// Executes step 1, waits its delay, then executes step 2, waits its delay, and repeats in a loop.
    #[default]
    Sequential,
    /// Executes each enabled hotkey on its own independent timer.
    Parallel,
}

/// One repeatable hotkey / action step inside a game macro profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MacroRule {
    /// Friendly label chosen by the user (e.g. `"Trocar Alvo"`, `"Poção HP"`).
    pub label: String,
    /// Canonical key or mouse code from [`SUPPORTED_KEYS`] (e.g. `"F1"`, `"SPACE"`).
    pub key: String,
    /// Wait time after this step (in Sequential mode) or repeat interval (in Parallel mode), in ms.
    pub interval_ms: u64,
    /// Key hold duration in milliseconds before releasing (`10..=5_000`).
    pub hold_ms: u64,
    /// Whether this individual hotkey step is enabled.
    pub enabled: bool,
}

impl Default for MacroRule {
    fn default() -> Self {
        Self {
            label: String::new(),
            key: String::new(),
            interval_ms: DEFAULT_MACRO_INTERVAL_MS,
            hold_ms: DEFAULT_MACRO_HOLD_MS,
            enabled: true,
        }
    }
}

impl MacroRule {
    /// Returns true if this step is enabled and has a valid key selected.
    pub fn is_ready(&self) -> bool {
        self.enabled && find_key_spec(&self.key).is_some()
    }

    /// Clamp timing fields and canonicalize the key identifier without forcing a hardcoded key.
    pub fn normalized(mut self) -> Self {
        self.interval_ms = self
            .interval_ms
            .clamp(MIN_MACRO_INTERVAL_MS, MAX_MACRO_INTERVAL_MS);
        self.hold_ms = self
            .hold_ms
            .clamp(MIN_MACRO_HOLD_MS, MAX_MACRO_HOLD_MS)
            .min(self.interval_ms.saturating_sub(5).max(MIN_MACRO_HOLD_MS));
        if let Some(spec) = find_key_spec(&self.key) {
            self.key = spec.code.to_string();
        } else {
            self.key.clear();
        }
        self.label = self.label.trim().to_string();
        self
    }
}

fn default_toggle_key() -> String {
    DEFAULT_TOGGLE_KEY.to_string()
}

/// Saved macro profile for a game (persisted in `config.toml`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GameMacroProfile {
    /// Whether macros start active automatically when this game is detected.
    pub auto_start: bool,
    /// Whether steps run one after another (`Sequential`) or independently (`Parallel`).
    pub mode: MacroExecutionMode,
    /// Wait time in milliseconds before repeating the full sequence from Step 1 (in `Sequential` mode).
    pub cycle_interval_ms: u64,
    /// Key code that toggles (starts/stops) the macro ONLY when pressed inside the focused game window.
    /// Empty string (`""`) disables the shortcut.
    #[serde(default = "default_toggle_key")]
    pub toggle_key: String,
    /// Configured hotkey rules for this game.
    pub rules: Vec<MacroRule>,
}

impl Default for GameMacroProfile {
    fn default() -> Self {
        Self {
            auto_start: false,
            mode: MacroExecutionMode::Sequential,
            cycle_interval_ms: DEFAULT_CYCLE_INTERVAL_MS,
            toggle_key: DEFAULT_TOGGLE_KEY.to_string(),
            rules: Vec::new(),
        }
    }
}

impl GameMacroProfile {
    /// Normalize all rules and cycle interval in this profile.
    pub fn normalized(mut self) -> Self {
        if self.cycle_interval_ms == 0 {
            self.cycle_interval_ms = DEFAULT_CYCLE_INTERVAL_MS;
        }
        self.cycle_interval_ms = self
            .cycle_interval_ms
            .clamp(MIN_MACRO_INTERVAL_MS, MAX_MACRO_INTERVAL_MS);
        let tk = self.toggle_key.trim();
        if tk.is_empty() || tk.eq_ignore_ascii_case("NONE") {
            self.toggle_key.clear();
        } else if let Some(spec) = find_key_spec(tk) {
            self.toggle_key = spec.code.to_string();
        } else {
            self.toggle_key = DEFAULT_TOGGLE_KEY.to_string();
        }
        self.rules = self.rules.into_iter().map(MacroRule::normalized).collect();
        self
    }

    /// Count how many rules are marked `enabled` and have a key configured.
    pub fn enabled_rules_count(&self) -> usize {
        self.rules.iter().filter(|r| r.is_ready()).count()
    }
}

/// Live status snapshot of macro execution for a single game window (`PID`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MacroInstanceStatus {
    /// Target process ID.
    pub pid: u32,
    /// Normalized game name.
    pub normalized_name: String,
    /// Detected Win32 window title for this PID, if found.
    pub window_title: Option<String>,
    /// Number of visible windows matched for this PID.
    pub window_count: usize,
    /// Whether autonomous macro sending is currently active for this PID.
    pub active: bool,
    /// Execution mode currently used by this instance.
    pub mode: MacroExecutionMode,
    /// Configured repeat interval for the full sequence in milliseconds.
    pub cycle_interval_ms: u64,
    /// In-window toggle key configured for this game (e.g. `"F8"`).
    pub toggle_key: String,
    /// True if this instance was just toggled (started/stopped) by pressing `toggle_key` in its window.
    pub toggled_by_hotkey: bool,
    /// True when the last step of the sequence has finished and it is waiting for the next cycle trigger.
    pub waiting_for_next_cycle: bool,
    /// Index of the step currently executing in Sequential mode.
    pub current_step_index: Option<usize>,
    /// Active rules configured on this PID.
    pub rules: Vec<MacroRule>,
    /// Per-rule trigger counters (same length as `rules`).
    pub rule_triggers: Vec<u64>,
    /// Total key/mouse actions dispatched to this window.
    pub total_triggers: u64,
}

#[derive(Debug, Clone, Default)]
struct RuleRuntimeState {
    last_fired: Option<Instant>,
    release_due: Option<Instant>,
    held_key: Option<String>,
    triggers: u64,
}

#[derive(Debug, Clone, Default)]
struct SequentialCursor {
    step_idx: usize,
    held_key: Option<String>,
    release_due: Option<Instant>,
    next_step_due: Option<Instant>,
    waiting_for_cycle: bool,
    cycle_wait_started_at: Option<Instant>,
}

#[derive(Debug, Clone)]
struct InstanceState {
    pid: u32,
    normalized_name: String,
    window_title: Option<String>,
    window_count: usize,
    active: bool,
    mode: MacroExecutionMode,
    cycle_interval_ms: u64,
    toggle_key: String,
    toggled_by_hotkey: bool,
    seq: SequentialCursor,
    rules: Vec<MacroRule>,
    runtime: Vec<RuleRuntimeState>,
    total_triggers: u64,
}

/// Manages per-window autonomous macro execution across all active game PIDs.
pub struct MacroManager {
    profiles: BTreeMap<String, GameMacroProfile>,
    instances: HashMap<u32, InstanceState>,
    prev_toggle_key_down: HashMap<String, bool>,
}

impl MacroManager {
    /// Create a new manager initialized with saved profiles from `AppConfig`.
    pub fn new(profiles: BTreeMap<String, GameMacroProfile>) -> Self {
        Self {
            profiles,
            instances: HashMap::new(),
            prev_toggle_key_down: HashMap::new(),
        }
    }

    /// Update saved profiles from configuration changes.
    pub fn update_profiles(&mut self, profiles: BTreeMap<String, GameMacroProfile>) {
        for (game_name, profile) in &profiles {
            let clean = profile.clone().normalized();
            for inst in self.instances.values_mut() {
                if &inst.normalized_name == game_name {
                    inst.mode = clean.mode;
                    inst.cycle_interval_ms = clean.cycle_interval_ms;
                    inst.toggle_key = clean.toggle_key.clone();
                    Self::sync_instance_rules(inst, clean.rules.clone());
                }
            }
        }
        self.profiles = profiles;
    }

    /// Return true if any monitored game instance currently has active macros.
    pub fn has_active_instances(&self) -> bool {
        self.instances
            .values()
            .any(|inst| inst.active && inst.rules.iter().any(MacroRule::is_ready))
    }

    /// Return true if any monitored game instance has ready rules (either active or waiting for in-window toggle key).
    pub fn has_configured_instances(&self) -> bool {
        self.instances
            .values()
            .any(|inst| inst.rules.iter().any(MacroRule::is_ready))
    }

    /// Toggle or set whether autonomous execution is active for a specific `pid`.
    pub fn set_instance_active(
        &mut self,
        pid: u32,
        normalized_name: &str,
        active: bool,
    ) -> MacroInstanceStatus {
        let profile = self
            .profiles
            .get(normalized_name)
            .cloned()
            .unwrap_or_default()
            .normalized();
        let inst = self
            .instances
            .entry(pid)
            .or_insert_with(|| Self::new_instance(pid, normalized_name, &profile));
        if !active && inst.active {
            Self::release_all_held_keys(inst);
            inst.seq = SequentialCursor::default();
            for rt in &mut inst.runtime {
                rt.last_fired = None;
            }
        } else if active && !inst.active {
            inst.seq = SequentialCursor::default();
        }
        inst.active = active;
        Self::refresh_window_metadata(inst);
        Self::status_from_instance(inst, Instant::now())
    }

    /// Update the rules, cycle repeat interval, toggle key, and execution mode for a specific `pid`.
    pub fn set_instance_rules(
        &mut self,
        pid: u32,
        normalized_name: &str,
        mode: MacroExecutionMode,
        cycle_interval_ms: u64,
        toggle_key: String,
        rules: Vec<MacroRule>,
    ) {
        let clean: Vec<MacroRule> = rules.into_iter().map(MacroRule::normalized).collect();
        let clean_cycle = if cycle_interval_ms == 0 {
            DEFAULT_CYCLE_INTERVAL_MS
        } else {
            cycle_interval_ms.clamp(MIN_MACRO_INTERVAL_MS, MAX_MACRO_INTERVAL_MS)
        };
        let clean_toggle =
            if toggle_key.trim().is_empty() || toggle_key.trim().eq_ignore_ascii_case("NONE") {
                String::new()
            } else if let Some(spec) = find_key_spec(toggle_key.trim()) {
                spec.code.to_string()
            } else {
                DEFAULT_TOGGLE_KEY.to_string()
            };
        let profile = self
            .profiles
            .entry(normalized_name.to_string())
            .or_default();
        profile.mode = mode;
        profile.cycle_interval_ms = clean_cycle;
        profile.toggle_key = clean_toggle.clone();
        profile.rules = clean.clone();

        let auto_start = profile.auto_start;
        let inst = self.instances.entry(pid).or_insert_with(|| {
            Self::new_instance(
                pid,
                normalized_name,
                &GameMacroProfile {
                    auto_start,
                    mode,
                    cycle_interval_ms: clean_cycle,
                    toggle_key: clean_toggle.clone(),
                    rules: clean.clone(),
                },
            )
        });
        if inst.mode != mode {
            Self::release_all_held_keys(inst);
            inst.seq = SequentialCursor::default();
            inst.mode = mode;
        }
        inst.cycle_interval_ms = clean_cycle;
        inst.toggle_key = clean_toggle;
        Self::sync_instance_rules(inst, clean);
        Self::refresh_window_metadata(inst);
    }

    /// Fire a single test pulse (`WM_KEYDOWN` followed by `WM_KEYUP`) to `pid` right now.
    pub fn test_single_key(&mut self, pid: u32, normalized_name: &str, key_code: &str) -> usize {
        let profile = self
            .profiles
            .get(normalized_name)
            .cloned()
            .unwrap_or_default();
        let inst = self
            .instances
            .entry(pid)
            .or_insert_with(|| Self::new_instance(pid, normalized_name, &profile));
        Self::refresh_window_metadata(inst);
        let sent = imp::send_key_down(pid, key_code);
        if sent > 0 {
            std::thread::sleep(Duration::from_millis(35));
            let _ = imp::send_key_up(pid, key_code);
            inst.total_triggers = inst.total_triggers.saturating_add(1);
        }
        sent
    }

    /// Advance all active macro timers and dispatch due key events to each game window.
    pub fn tick(
        &mut self,
        active_games: &[(u32, String)],
        now: Instant,
    ) -> Vec<MacroInstanceStatus> {
        let live_pids: std::collections::HashSet<u32> =
            active_games.iter().map(|(pid, _)| *pid).collect();

        // Release keys and clean up instances whose game process has exited.
        let closed: Vec<u32> = self
            .instances
            .keys()
            .copied()
            .filter(|pid| !live_pids.contains(pid))
            .collect();
        for pid in closed {
            if let Some(mut inst) = self.instances.remove(&pid) {
                Self::release_all_held_keys(&mut inst);
            }
        }

        // Ensure every running game has an instance entry.
        for (pid, norm_name) in active_games {
            let profile = self
                .profiles
                .get(norm_name)
                .cloned()
                .unwrap_or_default()
                .normalized();
            self.instances
                .entry(*pid)
                .or_insert_with(|| Self::new_instance(*pid, norm_name, &profile));
        }

        // Check if the user pressed a configured toggle key (e.g. F8) while focused on a specific game window.
        // Only the focused game window (matching `foreground_window_pid()`) is toggled!
        self.poll_foreground_window_toggle();

        let mut statuses = Vec::with_capacity(active_games.len());
        for (pid, _) in active_games {
            if let Some(inst) = self.instances.get_mut(pid) {
                Self::refresh_window_metadata(inst);
                if inst.active {
                    match inst.mode {
                        MacroExecutionMode::Sequential => Self::tick_sequential(inst, now),
                        MacroExecutionMode::Parallel => Self::tick_parallel(inst, now),
                    }
                }
                statuses.push(Self::status_from_instance(inst, now));
                inst.toggled_by_hotkey = false;
            }
        }
        statuses
    }

    fn poll_foreground_window_toggle(&mut self) {
        let mut unique_keys: Vec<String> = Vec::new();
        for inst in self.instances.values() {
            if !inst.toggle_key.is_empty() && !unique_keys.contains(&inst.toggle_key) {
                unique_keys.push(inst.toggle_key.clone());
            }
        }
        if unique_keys.is_empty() {
            return;
        }

        let fg_pid = imp::foreground_window_pid();
        for key_code in unique_keys {
            let is_down = imp::is_key_physically_down(&key_code);
            let was_down = self
                .prev_toggle_key_down
                .insert(key_code.clone(), is_down)
                .unwrap_or(false);
            if is_down && !was_down {
                if let Some(pid) = fg_pid {
                    if let Some(inst) = self.instances.get_mut(&pid) {
                        if inst.toggle_key.eq_ignore_ascii_case(&key_code) {
                            if inst.active {
                                Self::release_all_held_keys(inst);
                                inst.seq = SequentialCursor::default();
                                for rt in &mut inst.runtime {
                                    rt.last_fired = None;
                                }
                                inst.active = false;
                                inst.toggled_by_hotkey = true;
                            } else if inst.rules.iter().any(MacroRule::is_ready) {
                                inst.seq = SequentialCursor::default();
                                inst.active = true;
                                inst.toggled_by_hotkey = true;
                            }
                        }
                    }
                }
            }
        }
    }

    /// Release all held keys on shutdown.
    pub fn shutdown(&mut self) {
        for inst in self.instances.values_mut() {
            Self::release_all_held_keys(inst);
            inst.active = false;
        }
    }

    fn new_instance(pid: u32, normalized_name: &str, profile: &GameMacroProfile) -> InstanceState {
        let rules: Vec<MacroRule> = profile
            .rules
            .iter()
            .cloned()
            .map(MacroRule::normalized)
            .collect();
        let runtime = vec![RuleRuntimeState::default(); rules.len()];
        let mut inst = InstanceState {
            pid,
            normalized_name: normalized_name.to_string(),
            window_title: None,
            window_count: 0,
            active: profile.auto_start && rules.iter().any(MacroRule::is_ready),
            mode: profile.mode,
            cycle_interval_ms: if profile.cycle_interval_ms == 0 {
                DEFAULT_CYCLE_INTERVAL_MS
            } else {
                profile
                    .cycle_interval_ms
                    .clamp(MIN_MACRO_INTERVAL_MS, MAX_MACRO_INTERVAL_MS)
            },
            toggle_key: profile.toggle_key.clone(),
            toggled_by_hotkey: false,
            seq: SequentialCursor::default(),
            rules,
            runtime,
            total_triggers: 0,
        };
        Self::refresh_window_metadata(&mut inst);
        inst
    }

    fn sync_instance_rules(inst: &mut InstanceState, new_rules: Vec<MacroRule>) {
        // Release any held keys in parallel or sequential state if rules changed
        Self::release_all_held_keys(inst);
        let mut new_runtime = Vec::with_capacity(new_rules.len());
        for (idx, rule) in new_rules.iter().enumerate() {
            if let Some(prev) = inst.runtime.get(idx) {
                if inst
                    .rules
                    .get(idx)
                    .is_some_and(|old| old.key == rule.key && old.interval_ms == rule.interval_ms)
                {
                    new_runtime.push(prev.clone());
                    continue;
                }
            }
            new_runtime.push(RuleRuntimeState::default());
        }
        inst.rules = new_rules;
        inst.runtime = new_runtime;
        if inst.seq.step_idx >= inst.rules.len() {
            inst.seq.step_idx = 0;
        }
    }

    fn tick_sequential(inst: &mut InstanceState, now: Instant) {
        // 1. If a key is currently held down in the current step, release it when hold_ms expires
        if let Some(due) = inst.seq.release_due {
            if now >= due {
                if let Some(held) = inst.seq.held_key.take() {
                    let _ = imp::send_key_up(inst.pid, &held);
                }
                inst.seq.release_due = None;
            } else {
                return;
            }
        }

        if inst.rules.is_empty() || !inst.rules.iter().any(MacroRule::is_ready) {
            return;
        }

        // 2. Wait until the current step's post-action delay or cycle cooldown (`next_step_due`) has elapsed
        if let Some(next_due) = inst.seq.next_step_due {
            if now < next_due {
                return;
            }
            inst.seq.next_step_due = None;
            inst.seq.waiting_for_cycle = false;
            inst.seq.cycle_wait_started_at = None;
            if !inst.rules.is_empty() {
                inst.seq.step_idx = (inst.seq.step_idx + 1) % inst.rules.len();
            }
        }

        // 3. Find the next ready rule starting from `seq.step_idx`
        let len = inst.rules.len();
        let mut chosen = None;
        for offset in 0..len {
            let candidate = (inst.seq.step_idx + offset) % len;
            if inst.rules[candidate].is_ready() {
                chosen = Some(candidate);
                break;
            }
        }
        let Some(idx) = chosen else {
            return;
        };
        inst.seq.step_idx = idx;

        let rule = &inst.rules[idx];
        let interval_ms = rule.interval_ms.max(MIN_MACRO_INTERVAL_MS);
        let hold_ms = rule
            .hold_ms
            .max(MIN_MACRO_HOLD_MS)
            .min(interval_ms.saturating_sub(5).max(MIN_MACRO_HOLD_MS));
        let sent = imp::send_key_down(inst.pid, &rule.key);
        if sent > 0 {
            inst.seq.held_key = Some(rule.key.clone());
            inst.seq.release_due = Some(now + Duration::from_millis(hold_ms));
            if let Some(rt) = inst.runtime.get_mut(idx) {
                rt.triggers = rt.triggers.saturating_add(1);
                rt.last_fired = Some(now);
            }
            inst.total_triggers = inst.total_triggers.saturating_add(1);
        }

        // Check if there is another ready step after `idx` in the current sequence cycle.
        // If `idx` is the last ready step of the sequence, wait `cycle_interval_ms` before
        // starting the sequence again from the first step.
        let has_more_steps_in_cycle = ((idx + 1)..len).any(|i| inst.rules[i].is_ready());
        let wait_ms = if has_more_steps_in_cycle {
            interval_ms
        } else {
            inst.cycle_interval_ms
                .max(MIN_MACRO_INTERVAL_MS)
                .max(hold_ms + 5)
        };
        inst.seq.waiting_for_cycle = !has_more_steps_in_cycle;
        inst.seq.cycle_wait_started_at = if has_more_steps_in_cycle {
            None
        } else {
            Some(now)
        };
        inst.seq.next_step_due = Some(now + Duration::from_millis(wait_ms));
    }

    fn tick_parallel(inst: &mut InstanceState, now: Instant) {
        for (rule, rt) in inst.rules.iter().zip(inst.runtime.iter_mut()) {
            // 1. Check if a held key is due for release
            if let Some(due) = rt.release_due {
                if now >= due || !rule.is_ready() {
                    if let Some(held) = rt.held_key.take() {
                        let _ = imp::send_key_up(inst.pid, &held);
                    }
                    rt.release_due = None;
                }
            }

            if !rule.is_ready() {
                continue;
            }

            // 2. Check if interval has elapsed to fire next key press
            let should_fire = match rt.last_fired {
                None => true,
                Some(last) => {
                    now.saturating_duration_since(last)
                        >= Duration::from_millis(rule.interval_ms.max(MIN_MACRO_INTERVAL_MS))
                }
            };

            if should_fire && rt.held_key.is_none() {
                let sent = imp::send_key_down(inst.pid, &rule.key);
                rt.last_fired = Some(now);
                if sent > 0 {
                    rt.held_key = Some(rule.key.clone());
                    rt.release_due =
                        Some(now + Duration::from_millis(rule.hold_ms.max(MIN_MACRO_HOLD_MS)));
                    rt.triggers = rt.triggers.saturating_add(1);
                    inst.total_triggers = inst.total_triggers.saturating_add(1);
                }
            }
        }
    }

    fn release_all_held_keys(inst: &mut InstanceState) {
        if let Some(held) = inst.seq.held_key.take() {
            let _ = imp::send_key_up(inst.pid, &held);
        }
        inst.seq.release_due = None;
        for rt in &mut inst.runtime {
            if let Some(held) = rt.held_key.take() {
                let _ = imp::send_key_up(inst.pid, &held);
            }
            rt.release_due = None;
        }
    }

    fn refresh_window_metadata(inst: &mut InstanceState) {
        let info = imp::inspect_process_windows(inst.pid);
        inst.window_count = info.count;
        inst.window_title = info.primary_title;
    }

    fn status_from_instance(inst: &InstanceState, now: Instant) -> MacroInstanceStatus {
        let last_step_highlight_done = inst
            .seq
            .cycle_wait_started_at
            .is_some_and(|at| now.saturating_duration_since(at) >= Duration::from_millis(120));
        let waiting_for_next_cycle = inst.active
            && inst.mode == MacroExecutionMode::Sequential
            && inst.seq.waiting_for_cycle
            && inst.seq.held_key.is_none()
            && last_step_highlight_done;
        let current_step_index = if inst.active
            && inst.mode == MacroExecutionMode::Sequential
            && !waiting_for_next_cycle
            && inst.seq.step_idx < inst.rules.len()
            && inst.rules[inst.seq.step_idx].is_ready()
        {
            Some(inst.seq.step_idx)
        } else {
            None
        };
        MacroInstanceStatus {
            pid: inst.pid,
            normalized_name: inst.normalized_name.clone(),
            window_title: inst.window_title.clone(),
            window_count: inst.window_count,
            active: inst.active,
            mode: inst.mode,
            cycle_interval_ms: inst.cycle_interval_ms,
            toggle_key: inst.toggle_key.clone(),
            toggled_by_hotkey: inst.toggled_by_hotkey,
            waiting_for_next_cycle,
            current_step_index,
            rules: inst.rules.clone(),
            rule_triggers: inst.runtime.iter().map(|rt| rt.triggers).collect(),
            total_triggers: inst.total_triggers,
        }
    }
}

#[derive(Debug, Default)]
struct WindowInspectResult {
    count: usize,
    primary_title: Option<String>,
}

#[cfg(windows)]
mod imp {
    use super::{build_key_lparam, find_key_spec, WindowInspectResult};
    use windows::core::BOOL;
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW,
        GetWindowThreadProcessId, IsWindowVisible, PostMessageW, WM_KEYDOWN, WM_KEYUP,
        WM_LBUTTONDOWN, WM_LBUTTONUP, WM_RBUTTONDOWN, WM_RBUTTONUP,
    };

    #[link(name = "user32")]
    extern "system" {
        fn GetAsyncKeyState(vkey: i32) -> i16;
    }

    pub(super) fn foreground_window_pid() -> Option<u32> {
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.0.is_null() {
            return None;
        }
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        if pid == 0 {
            None
        } else {
            Some(pid)
        }
    }

    pub(super) fn is_key_physically_down(key_code: &str) -> bool {
        let Some(spec) = find_key_spec(key_code) else {
            return false;
        };
        if spec.vk == 0 {
            return false;
        }
        let state = unsafe { GetAsyncKeyState(i32::from(spec.vk)) };
        (state as u16 & 0x8000) != 0
    }

    const MK_LBUTTON_FLAG: usize = 0x0001;
    const MK_RBUTTON_FLAG: usize = 0x0002;

    struct EnumContext {
        target_pid: u32,
        titled_windows: Vec<(HWND, String)>,
        untitled_windows: Vec<HWND>,
    }

    unsafe extern "system" fn enum_windows_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let ctx = unsafe { &mut *(lparam.0 as *mut EnumContext) };
        let mut window_pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut window_pid)) };
        if window_pid != ctx.target_pid {
            return BOOL(1);
        }
        if !unsafe { IsWindowVisible(hwnd) }.as_bool() {
            return BOOL(1);
        }
        let len = unsafe { GetWindowTextLengthW(hwnd) };
        if len > 0 {
            let mut buf = vec![0u16; (len as usize) + 1];
            let copied = unsafe { GetWindowTextW(hwnd, &mut buf) };
            if copied > 0 {
                let title = String::from_utf16_lossy(&buf[..copied as usize])
                    .trim()
                    .to_string();
                if !title.is_empty() {
                    ctx.titled_windows.push((hwnd, title));
                    return BOOL(1);
                }
            }
        }
        ctx.untitled_windows.push(hwnd);
        BOOL(1)
    }

    fn collect_target_windows(pid: u32) -> EnumContext {
        let mut ctx = EnumContext {
            target_pid: pid,
            titled_windows: Vec::new(),
            untitled_windows: Vec::new(),
        };
        if pid == 0 {
            return ctx;
        }
        // SAFETY: `ctx` lives on the stack for the synchronous duration of `EnumWindows`.
        unsafe {
            let _ = EnumWindows(
                Some(enum_windows_proc),
                LPARAM(std::ptr::from_mut(&mut ctx) as isize),
            );
        }
        ctx
    }

    pub(super) fn inspect_process_windows(pid: u32) -> WindowInspectResult {
        let ctx = collect_target_windows(pid);
        if !ctx.titled_windows.is_empty() {
            WindowInspectResult {
                count: ctx.titled_windows.len(),
                primary_title: ctx.titled_windows.first().map(|(_, t)| t.clone()),
            }
        } else {
            WindowInspectResult {
                count: ctx.untitled_windows.len(),
                primary_title: None,
            }
        }
    }

    fn target_hwnds(pid: u32) -> Vec<HWND> {
        let ctx = collect_target_windows(pid);
        if !ctx.titled_windows.is_empty() {
            ctx.titled_windows.into_iter().map(|(h, _)| h).collect()
        } else {
            ctx.untitled_windows
        }
    }

    pub(super) fn send_key_down(pid: u32, key_code: &str) -> usize {
        let Some(spec) = find_key_spec(key_code) else {
            return 0;
        };
        let hwnds = target_hwnds(pid);
        let mut sent = 0usize;
        for hwnd in hwnds {
            let res = match spec.code {
                "MOUSE_LEFT" => unsafe {
                    PostMessageW(
                        Some(hwnd),
                        WM_LBUTTONDOWN,
                        WPARAM(MK_LBUTTON_FLAG),
                        LPARAM(0),
                    )
                },
                "MOUSE_RIGHT" => unsafe {
                    PostMessageW(
                        Some(hwnd),
                        WM_RBUTTONDOWN,
                        WPARAM(MK_RBUTTON_FLAG),
                        LPARAM(0),
                    )
                },
                _ => {
                    let lparam = build_key_lparam(spec.scan_code, spec.extended, false);
                    unsafe {
                        PostMessageW(
                            Some(hwnd),
                            WM_KEYDOWN,
                            WPARAM(spec.vk as usize),
                            LPARAM(lparam),
                        )
                    }
                }
            };
            if res.is_ok() {
                sent += 1;
            }
        }
        sent
    }

    pub(super) fn send_key_up(pid: u32, key_code: &str) -> usize {
        let Some(spec) = find_key_spec(key_code) else {
            return 0;
        };
        let hwnds = target_hwnds(pid);
        let mut sent = 0usize;
        for hwnd in hwnds {
            let res = match spec.code {
                "MOUSE_LEFT" => unsafe {
                    PostMessageW(Some(hwnd), WM_LBUTTONUP, WPARAM(0), LPARAM(0))
                },
                "MOUSE_RIGHT" => unsafe {
                    PostMessageW(Some(hwnd), WM_RBUTTONUP, WPARAM(0), LPARAM(0))
                },
                _ => {
                    let lparam = build_key_lparam(spec.scan_code, spec.extended, true);
                    unsafe {
                        PostMessageW(
                            Some(hwnd),
                            WM_KEYUP,
                            WPARAM(spec.vk as usize),
                            LPARAM(lparam),
                        )
                    }
                }
            };
            if res.is_ok() {
                sent += 1;
            }
        }
        sent
    }
}

#[cfg(not(windows))]
mod imp {
    use super::WindowInspectResult;

    pub(super) fn foreground_window_pid() -> Option<u32> {
        None
    }

    pub(super) fn is_key_physically_down(_key_code: &str) -> bool {
        false
    }

    pub(super) fn inspect_process_windows(_pid: u32) -> WindowInspectResult {
        WindowInspectResult::default()
    }

    pub(super) fn send_key_down(_pid: u32, _key_code: &str) -> usize {
        0
    }

    pub(super) fn send_key_up(_pid: u32, _key_code: &str) -> usize {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_lookup_is_case_insensitive_and_complete() {
        let f1 = find_key_spec("f1").expect("F1 must exist");
        assert_eq!(f1.code, "F1");
        assert_eq!(f1.vk, 0x70);

        let space = find_key_spec("space").expect("SPACE must exist");
        assert_eq!(space.label, "Espaço");
        assert_eq!(space.vk, 0x20);
    }

    #[test]
    fn lparam_encodes_scancode_extended_and_transition_bits() {
        let down = build_key_lparam(0x3B, false, false);
        assert_eq!(down & 0xFFFF, 1);
        assert_eq!((down >> 16) & 0xFF, 0x3B);
        assert_eq!((down >> 24) & 1, 0);
        assert_eq!((down >> 31) & 1, 0);

        let up_ext = build_key_lparam(0x48, true, true);
        assert_eq!((up_ext >> 16) & 0xFF, 0x48);
        assert_eq!((up_ext >> 24) & 1, 1);
        assert_eq!((up_ext >> 30) & 1, 1);
        assert_eq!((up_ext >> 31) & 1, 1);
    }

    #[test]
    fn macro_rule_normalizes_bounds_without_forcing_hardcoded_keys() {
        let rule = MacroRule {
            label: "  Passo 1  ".into(),
            key: "invalid_key".into(),
            interval_ms: 2,
            hold_ms: 99999,
            enabled: true,
        }
        .normalized();
        assert_eq!(rule.label, "Passo 1");
        assert_eq!(rule.key, "");
        assert!(!rule.is_ready());
        assert_eq!(rule.interval_ms, MIN_MACRO_INTERVAL_MS);
        assert!(rule.hold_ms < rule.interval_ms);
    }

    #[test]
    fn sequential_mode_advances_step_one_after_another_then_waits_cycle_interval() {
        let mut profiles = BTreeMap::new();
        profiles.insert(
            "mir4g".to_string(),
            GameMacroProfile {
                auto_start: true,
                mode: MacroExecutionMode::Sequential,
                cycle_interval_ms: 5000,
                toggle_key: String::new(),
                rules: vec![
                    MacroRule {
                        label: "Alvo".into(),
                        key: "TAB".into(),
                        interval_ms: 50,
                        hold_ms: 15,
                        enabled: true,
                    },
                    MacroRule {
                        label: "Desmontar".into(),
                        key: "PAGEDOWN".into(),
                        interval_ms: 50,
                        hold_ms: 15,
                        enabled: true,
                    },
                    MacroRule {
                        label: "Coletar".into(),
                        key: "F".into(),
                        interval_ms: 50,
                        hold_ms: 15,
                        enabled: true,
                    },
                ],
            },
        );

        let mut mgr = MacroManager::new(profiles);
        let t0 = Instant::now();
        let st0 = mgr.tick(&[(1001, "mir4g".into())], t0);
        assert_eq!(st0[0].current_step_index, Some(0));

        // After 55ms, step 0's 50ms delay has elapsed and it advances to step 1
        let st1 = mgr.tick(&[(1001, "mir4g".into())], t0 + Duration::from_millis(55));
        assert_eq!(st1[0].current_step_index, Some(1));

        // After another 55ms (t=110ms), step 1's 50ms delay has elapsed and it advances to step 2 (last step)
        let st2 = mgr.tick(&[(1001, "mir4g".into())], t0 + Duration::from_millis(110));
        assert_eq!(st2[0].current_step_index, Some(2));

        // At t=500ms, step 2 has finished holding, and the sequence is waiting for the 5000ms cycle_interval_ms
        let st_wait = mgr.tick(&[(1001, "mir4g".into())], t0 + Duration::from_millis(500));
        assert!(st_wait[0].waiting_for_next_cycle);
        assert_eq!(st_wait[0].current_step_index, None);

        // After 5000ms from step 2 (t=5120ms), the next cycle triggers from step 0!
        let st3 = mgr.tick(&[(1001, "mir4g".into())], t0 + Duration::from_millis(5120));
        assert!(!st3[0].waiting_for_next_cycle);
        assert_eq!(st3[0].current_step_index, Some(0));
    }
}
