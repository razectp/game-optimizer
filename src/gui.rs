//! End-user GUI (Portuguese) with system-tray minimize.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, RichText, Sense,
    Stroke, Vec2,
};
use eframe::{App, Frame, NativeOptions};
#[cfg(windows)]
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
#[cfg(windows)]
use tray_icon::{Icon, TrayIcon, TrayIconBuilder, TrayIconEvent};

#[cfg(target_os = "linux")]
#[path = "gui/linux_tray.rs"]
mod linux_tray;

use crate::autostart;
use crate::config::AppConfig;
use crate::detect::RunningProcessInfo;
use crate::macros::{
    key_display_label, GameMacroProfile, MacroExecutionMode, MacroInstanceStatus, MacroManager,
    MacroRule, DEFAULT_CYCLE_INTERVAL_MS, DEFAULT_MACRO_HOLD_MS, DEFAULT_MACRO_INTERVAL_MS,
    MAX_MACRO_INTERVAL_MS, MIN_MACRO_INTERVAL_MS, SUPPORTED_KEYS,
};
use crate::optimize::{optimize_system, OptimizeReport, OptimizeRequest};
use crate::report::format_mib;
#[cfg(windows)]
use crate::tunnel::TunnelManager as UiTunnelManager;
#[cfg(target_os = "linux")]
use crate::tunnel::{is_scoped_game, GameProcess, GuiTunnel as UiTunnelManager};
use crate::tunnel::{TunnelHealth, TunnelStatus, WireGuardConfig};

/// Warm amber — primary actions / brand energy.
const ACCENT: Color32 = Color32::from_rgb(245, 158, 11);
const ACCENT_SOFT: Color32 = Color32::from_rgb(58, 42, 18);
/// Calm emerald for “all good” status.
const OK: Color32 = Color32::from_rgb(16, 185, 129);
const BG: Color32 = Color32::from_rgb(15, 17, 22);
const BG_RAISED: Color32 = Color32::from_rgb(22, 26, 34);
const BG_ROW: Color32 = Color32::from_rgb(30, 35, 46);
const STROKE: Color32 = Color32::from_rgb(45, 52, 66);
const TEXT: Color32 = Color32::from_rgb(243, 244, 246);
const MUTED: Color32 = Color32::from_rgb(156, 163, 175);
const WARN: Color32 = Color32::from_rgb(249, 115, 22);
const TUNNEL_COLOR: Color32 = Color32::from_rgb(56, 189, 248);
const BG_MODAL: Color32 = Color32::from_rgb(18, 21, 28);
const STROKE_STRONG: Color32 = Color32::from_rgb(62, 72, 92);
const DANGER: Color32 = Color32::from_rgb(239, 68, 68);
const DANGER_BG: Color32 = Color32::from_rgb(88, 28, 28);
const DANGER_TEXT: Color32 = Color32::from_rgb(254, 202, 202);
const ACCENT_TEXT: Color32 = Color32::from_rgb(20, 15, 8);
const SCROLL_AREA_IDS: &[&str] = &[
    "responsive-body-scroll",
    "main-games-scroll",
    "main-logs-scroll",
    "sidebar-settings-scroll",
    "tunnel-modal-scroll",
    "process-modal-content-scroll",
    "process-list-scroll",
    "custom-games-scroll",
    "builtin-games-scroll",
    "reclaim-rules-scroll",
    "protected-rules-scroll",
    "macro-modal-scroll",
    "macro-rules-scroll",
];

fn debounce_elapsed(changed_at: Instant, now: Instant, delay: Duration) -> bool {
    now.saturating_duration_since(changed_at) >= delay
}

fn narrow_layout(width: f32) -> bool {
    width < 760.0
}

fn macro_profile_key(normalized_name: &str, window_title: Option<&str>, pid: u32) -> String {
    let norm = normalized_name.trim().to_ascii_lowercase();
    if let Some(t) = window_title {
        let trimmed = t.trim();
        if !trimmed.is_empty()
            && !trimmed.eq_ignore_ascii_case(normalized_name)
            && !trimmed.eq_ignore_ascii_case(&norm)
            && !trimmed.eq_ignore_ascii_case(&format!("{normalized_name}.exe"))
            && !trimmed.eq_ignore_ascii_case(&format!("{norm}.exe"))
        {
            return format!("{norm}::{trimmed}");
        }
    }
    format!("{norm}#pid:{pid}")
}

fn contain_list_scroll(ui: &egui::Ui, viewport: egui::Rect) {
    // At the list boundary egui normally passes the remaining wheel delta to
    // its parent. Keep the surrounding dialog/body still while hovering a list.
    let hover_rect = viewport.expand2(Vec2::new(ui.spacing().scroll.bar_width, 0.0));
    if ui.rect_contains_pointer(hover_rect) {
        ui.input_mut(|input| input.smooth_scroll_delta.y = 0.0);
    }
}

fn send_worker(ctx: &egui::Context, tx: &Sender<WorkerMsg>, msg: WorkerMsg) {
    let repaint = !matches!(&msg, WorkerMsg::Tunnel(_));
    if tx.send(msg).is_ok() && repaint {
        ctx.request_repaint();
    }
}

enum WorkerMsg {
    Report(OptimizeReport),
    Log(String),
    Watching(bool),
    Tunnel(TunnelStatus),
    Macros(Vec<MacroInstanceStatus>),
    Processes(Vec<RunningProcessInfo>),
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProcessTab {
    RunningProcesses,
    Games,
    MemoryTrim,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveModal {
    Tunnel,
    Process,
    Macro,
}

enum UiCmd {
    Optimize {
        trim_game: bool,
    },
    Scan,
    RefreshProcesses(Box<AppConfig>),
    StartWatch {
        interval: u64,
        trim_game: bool,
    },
    StopWatch,
    UpdateConfig(Box<AppConfig>),
    ReplaceConfig(Box<AppConfig>),
    CommitConfig(Box<AppConfig>),
    TestVpsPing,
    #[cfg(target_os = "linux")]
    SetGameTunnel(GameProcess, bool),
    SetMacroActive {
        pid: u32,
        normalized_name: String,
        active: bool,
    },
    SetMacroRules {
        pid: u32,
        normalized_name: String,
        mode: MacroExecutionMode,
        cycle_interval_ms: u64,
        toggle_key: String,
        rules: Vec<MacroRule>,
    },
    TestMacroKey {
        pid: u32,
        normalized_name: String,
        key: String,
    },
    Shutdown,
}

#[cfg(windows)]
enum AppTrayEvent {
    Menu(MenuEvent),
    Tray(TrayIconEvent),
}

/// Launch the native GUI (blocks until quit).
pub fn run(config_path: PathBuf) -> eframe::Result<()> {
    run_with_tray_start(config_path, false)
}

/// Launch the GUI, optionally starting hidden when a Linux tray is available.
pub fn run_with_tray_start(config_path: PathBuf, start_in_tray: bool) -> eframe::Result<()> {
    #[cfg(not(target_os = "linux"))]
    let _ = start_in_tray;
    #[cfg(windows)]
    let icon = tray_rgba_icon();
    let saved_gui = AppConfig::load_or_default(&config_path)
        .map(|config| config.gui.normalized())
        .unwrap_or_default();
    let options = NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([
                saved_gui.window_width as f32,
                saved_gui.window_height as f32,
            ])
            .with_min_inner_size([640.0, 420.0])
            .with_maximized(saved_gui.window_maximized)
            .with_title("Game Optimizer")
            .with_icon(eframe_icon()),
        ..Default::default()
    };

    #[cfg(windows)]
    let (tray_tx, tray_rx) = mpsc::channel::<AppTrayEvent>();

    eframe::run_native(
        "Game Optimizer",
        options,
        Box::new(move |cc| {
            #[cfg(windows)]
            {
                let ctx_tray = cc.egui_ctx.clone();
                let tx_tray = tray_tx.clone();
                TrayIconEvent::set_event_handler(Some(move |event| {
                    let _ = tx_tray.send(AppTrayEvent::Tray(event));
                    ctx_tray.request_repaint();
                }));

                let ctx_menu = cc.egui_ctx.clone();
                let tx_menu = tray_tx;
                MenuEvent::set_event_handler(Some(move |event| {
                    let _ = tx_menu.send(AppTrayEvent::Menu(event));
                    ctx_menu.request_repaint();
                }));
            }

            install_fonts(&cc.egui_ctx);
            style_visuals(&cc.egui_ctx);
            Ok(Box::new(GuiApp::new(
                config_path,
                #[cfg(windows)]
                icon,
                #[cfg(windows)]
                tray_rx,
                #[cfg(target_os = "linux")]
                start_in_tray,
                cc.egui_ctx.clone(),
            )))
        }),
    )
}

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    #[cfg(windows)]
    let candidates = [
        r"C:\Windows\Fonts\segoeui.ttf",
        r"C:\Windows\Fonts\calibri.ttf",
    ];
    #[cfg(not(windows))]
    let candidates = [
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/usr/share/fonts/noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
        "/usr/share/fonts/truetype/ubuntu/Ubuntu-R.ttf",
        "/usr/share/fonts/cantarell/Cantarell-Regular.otf",
    ];
    for path in candidates {
        if let Ok(bytes) = std::fs::read(path) {
            fonts
                .font_data
                .insert("ui".into(), FontData::from_owned(bytes).into());
            if let Some(family) = fonts.families.get_mut(&FontFamily::Proportional) {
                family.insert(0, "ui".into());
            }
            break;
        }
    }
    ctx.set_fonts(fonts);
}

fn style_visuals(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BG;
    visuals.window_fill = BG_MODAL;
    visuals.override_text_color = Some(TEXT);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, STROKE);
    visuals.widgets.inactive.bg_fill = BG_ROW;
    visuals.widgets.inactive.weak_bg_fill = BG_RAISED;
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(45, 52, 66);
    visuals.widgets.active.bg_fill = ACCENT_SOFT;
    visuals.selection.bg_fill = ACCENT_SOFT;
    visuals.extreme_bg_color = Color32::from_rgb(12, 14, 18);
    visuals.widgets.inactive.corner_radius = CornerRadius::same(8);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(8);
    visuals.widgets.active.corner_radius = CornerRadius::same(8);
    ctx.set_visuals(visuals);

    ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.spacing.item_spacing = Vec2::new(8.0, 8.0);
        style.spacing.button_padding = Vec2::new(12.0, 7.0);
        style.spacing.window_margin = egui::Margin::same(12);
        style.spacing.indent = 12.0;
    });
}

fn application_icon(size: u32) -> image::RgbaImage {
    image::load_from_memory_with_format(
        include_bytes!("../assets/game-optimizer-icon.png"),
        image::ImageFormat::Png,
    )
    .expect("bundled Game Optimizer icon is a valid PNG")
    .resize_exact(size, size, image::imageops::FilterType::Lanczos3)
    .into_rgba8()
}

#[cfg(windows)]
fn tray_rgba_icon() -> Icon {
    let image = application_icon(32);
    Icon::from_rgba(image.into_raw(), 32, 32).expect("tray icon")
}

fn eframe_icon() -> egui::IconData {
    let image = application_icon(64);
    egui::IconData {
        rgba: image.into_raw(),
        width: 64,
        height: 64,
    }
}

fn paint_bg(ui: &egui::Ui) {
    let rect = ui.max_rect();
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, BG);
    let accent_line = egui::Rect::from_min_size(rect.min, Vec2::new(rect.width(), 2.0));
    painter.rect_filled(accent_line, 0.0, ACCENT.gamma_multiply(0.45));
}

struct GuiApp {
    #[allow(dead_code)]
    config_path: PathBuf,
    app_config: AppConfig,
    config_valid: bool,
    report: Option<OptimizeReport>,
    logs: Vec<String>,
    watching: bool,
    watch_interval: u64,
    trim_game_memory: bool,
    autostart: bool,
    #[cfg(windows)]
    start_minimized_hint: bool,
    wireguard_cfg: WireGuardConfig,
    tunnel_status: Option<TunnelStatus>,
    #[cfg(target_os = "linux")]
    game_tunnel_overrides: std::collections::HashMap<GameProcess, bool>,
    active_modal: Option<ActiveModal>,
    process_tab: ProcessTab,
    running_processes: Vec<RunningProcessInfo>,
    process_search_filter: String,
    new_game_input: String,
    new_reclaim_input: String,
    new_never_trim_input: String,
    show_paste_conf: bool,
    paste_conf_text: String,
    show_wg_private_key: bool,
    tunnel_modal_notice: Option<(Color32, String)>,
    macro_statuses: Vec<MacroInstanceStatus>,
    macro_target_pid: Option<u32>,
    macro_target_name: String,
    macro_target_normalized: String,
    capturing_macro_key_for: Option<Option<usize>>,
    recording_macro_sequence: bool,
    confirm_clear_macros: bool,
    default_macro_delay_ms: u64,
    /// Must stay owned so the tray icon is not destroyed.
    #[allow(dead_code)]
    #[cfg(windows)]
    tray: Option<TrayIcon>,
    #[cfg(windows)]
    tray_show: MenuItem,
    #[cfg(windows)]
    tray_optimize: MenuItem,
    #[cfg(windows)]
    tray_quit: MenuItem,
    #[cfg(target_os = "linux")]
    linux_tray: Option<linux_tray::LinuxTray>,
    #[cfg(target_os = "linux")]
    was_minimized: bool,
    hidden_to_tray: bool,
    ui_tx: Sender<UiCmd>,
    worker_rx: Receiver<WorkerMsg>,
    #[cfg(windows)]
    tray_rx: Receiver<AppTrayEvent>,
    geometry_dirty_at: Option<Instant>,
    exiting: bool,
    shutdown_sent: bool,
    worker_stopped: bool,
}

impl GuiApp {
    fn new(
        config_path: PathBuf,
        #[cfg(windows)] icon: Icon,
        #[cfg(windows)] tray_rx: Receiver<AppTrayEvent>,
        #[cfg(target_os = "linux")] start_in_tray: bool,
        ctx: egui::Context,
    ) -> Self {
        let autostart =
            autostart::ensure_default_enabled().unwrap_or_else(|_| autostart::is_enabled());

        let (mut initial_cfg, config_valid, config_error) =
            match AppConfig::load_or_default(&config_path) {
                Ok(config) => (config, true, None),
                Err(err) => (AppConfig::default(), false, Some(err.to_string())),
            };
        initial_cfg.gui = initial_cfg.gui.clone().normalized();
        let wireguard_cfg = initial_cfg.wireguard.clone();
        let trim_game_memory = initial_cfg.trim_game_memory;
        let watch_interval = initial_cfg.gui.watch_interval_secs;
        let watch_enabled = initial_cfg.gui.watch_enabled;

        #[cfg(windows)]
        let (tray, tray_show, tray_optimize, tray_quit) = {
            let menu = Menu::new();
            let tray_show = MenuItem::new("Abrir Game Optimizer", true, None);
            let tray_optimize = MenuItem::new("Otimizar agora", true, None);
            let tray_quit = MenuItem::new("Sair", true, None);
            let _ = menu.append(&tray_show);
            let _ = menu.append(&PredefinedMenuItem::separator());
            let _ = menu.append(&tray_optimize);
            let _ = menu.append(&PredefinedMenuItem::separator());
            let _ = menu.append(&tray_quit);

            let tray = TrayIconBuilder::new()
                .with_menu(Box::new(menu))
                .with_menu_on_left_click(false)
                .with_menu_on_right_click(true)
                .with_tooltip("Game Optimizer")
                .with_icon(icon)
                .build()
                .ok();

            (tray, tray_show, tray_optimize, tray_quit)
        };

        #[cfg(target_os = "linux")]
        let linux_tray = linux_tray::LinuxTray::start(ctx.clone()).ok();
        #[cfg(target_os = "linux")]
        let hidden_to_tray = start_in_tray && linux_tray.is_some();
        #[cfg(not(target_os = "linux"))]
        let hidden_to_tray = false;
        if hidden_to_tray {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }

        let (ui_tx, ui_rx) = mpsc::channel::<UiCmd>();
        let (worker_tx, worker_rx) = mpsc::channel::<WorkerMsg>();
        let watch_flag = Arc::new(AtomicBool::new(false));
        spawn_worker(
            config_path.clone(),
            initial_cfg.clone(),
            config_valid,
            ui_rx,
            worker_tx,
            Arc::clone(&watch_flag),
            ctx,
        );

        let mut app = Self {
            config_path,
            app_config: initial_cfg,
            config_valid,
            report: None,
            logs: vec!["Pronto.".into()],
            watching: false,
            watch_interval,
            trim_game_memory,
            autostart,
            #[cfg(windows)]
            start_minimized_hint: false,
            wireguard_cfg,
            tunnel_status: None,
            #[cfg(target_os = "linux")]
            game_tunnel_overrides: std::collections::HashMap::new(),
            active_modal: None,
            process_tab: ProcessTab::RunningProcesses,
            running_processes: Vec::new(),
            process_search_filter: String::new(),
            new_game_input: String::new(),
            new_reclaim_input: String::new(),
            new_never_trim_input: String::new(),
            show_paste_conf: false,
            paste_conf_text: String::new(),
            show_wg_private_key: false,
            tunnel_modal_notice: None,
            macro_statuses: Vec::new(),
            macro_target_pid: None,
            macro_target_name: String::new(),
            macro_target_normalized: String::new(),
            capturing_macro_key_for: None,
            recording_macro_sequence: false,
            confirm_clear_macros: false,
            default_macro_delay_ms: DEFAULT_MACRO_INTERVAL_MS,
            #[cfg(windows)]
            tray,
            #[cfg(windows)]
            tray_show,
            #[cfg(windows)]
            tray_optimize,
            #[cfg(windows)]
            tray_quit,
            #[cfg(target_os = "linux")]
            linux_tray,
            #[cfg(target_os = "linux")]
            was_minimized: false,
            hidden_to_tray,
            ui_tx,
            worker_rx,
            #[cfg(windows)]
            tray_rx,
            geometry_dirty_at: None,
            exiting: false,
            shutdown_sent: false,
            worker_stopped: false,
        };
        if let Some(error) = config_error {
            app.push_log(format!("Configuração inválida: {error}. Corrija ou recarregue o perfil; otimizações estão bloqueadas."));
        }
        if config_valid {
            app.refresh_running_processes();
            let _ = app.ui_tx.send(UiCmd::Scan);
        }
        if config_valid && watch_enabled {
            app.app_config.gui.watch_enabled = true;
            let _ = app.ui_tx.send(UiCmd::StartWatch {
                interval: app.watch_interval,
                trim_game: app.trim_game_memory,
            });
        }
        app
    }

    fn refresh_running_processes(&mut self) {
        let _ = self
            .ui_tx
            .send(UiCmd::RefreshProcesses(Box::new(self.app_config.clone())));
    }

    fn add_process_as_game(&mut self, proc: &RunningProcessInfo) {
        let mut added = false;
        if self.app_config.add_game_name(&proc.normalized_name) {
            added = true;
        }
        if cfg!(windows)
            && proc.root_name != proc.normalized_name
            && self.app_config.add_game_name(&proc.root_name)
        {
            added = true;
        }
        if added {
            if cfg!(windows) && proc.root_name != proc.normalized_name {
                self.push_log(format!(
                    "Jogo adicionado: {} (Processo raiz detectado: {}, PID: {})",
                    proc.name, proc.root_name, proc.root_pid
                ));
            } else {
                self.push_log(format!(
                    "Jogo adicionado: {} (PID: {})",
                    proc.name, proc.pid
                ));
            }
            self.save_app_config();
            let _ = self.ui_tx.send(UiCmd::Scan);
            self.refresh_running_processes();
        }
    }

    fn remove_process_as_game(&mut self, proc: &RunningProcessInfo) {
        let mut removed = false;
        if self.app_config.remove_game_name(&proc.normalized_name) {
            removed = true;
        }
        if cfg!(windows)
            && proc.root_name != proc.normalized_name
            && self.app_config.remove_game_name(&proc.root_name)
        {
            removed = true;
        }
        if removed {
            self.push_log(format!("Jogo removido do monitoramento: {}", proc.name));
            self.save_app_config();
            let _ = self.ui_tx.send(UiCmd::Scan);
            self.refresh_running_processes();
        }
    }

    fn save_app_config(&mut self) {
        if !self.config_valid {
            self.push_log("Configuração inválida: alterações não foram gravadas para preservar o perfil existente.");
            return;
        }
        self.app_config.trim_game_memory = self.trim_game_memory;
        self.app_config.gui.watch_interval_secs = self.watch_interval;
        let _ = self
            .ui_tx
            .send(UiCmd::UpdateConfig(Box::new(self.app_config.clone())));
    }

    fn queue_config_snapshot(&mut self) {
        if !self.config_valid {
            return;
        }
        self.app_config.trim_game_memory = self.trim_game_memory;
        self.app_config.gui.watch_interval_secs = self.watch_interval;
        let _ = self
            .ui_tx
            .send(UiCmd::UpdateConfig(Box::new(self.app_config.clone())));
    }

    fn commit_wireguard_config(&mut self) {
        if !self.config_valid {
            self.push_log("Configuração inválida: não é possível salvar o túnel antes de recarregar um perfil válido.");
            return;
        }
        self.app_config.wireguard = self.wireguard_cfg.clone();
        self.app_config.trim_game_memory = self.trim_game_memory;
        self.app_config.gui.watch_interval_secs = self.watch_interval;
        let _ = self
            .ui_tx
            .send(UiCmd::CommitConfig(Box::new(self.app_config.clone())));
    }

    fn import_conf_file(&mut self) {
        if let Some(file_path) = rfd::FileDialog::new()
            .add_filter("Configuração WireGuard (*.conf)", &["conf"])
            .set_title("Selecione o arquivo .conf do WireGuard para Importar / Reimportar")
            .pick_file()
        {
            match std::fs::read_to_string(&file_path) {
                Ok(content) => match WireGuardConfig::parse_conf(&content) {
                    Ok(mut parsed) => {
                        parsed.enabled = true;
                        self.wireguard_cfg = parsed;
                        let file_name = file_path
                            .file_name()
                            .map(|s| s.to_string_lossy())
                            .unwrap_or_default();
                        let msg = format!(
                            "Configuração WireGuard reimportada com sucesso de '{file_name}'."
                        );
                        self.tunnel_modal_notice = Some((OK, msg.clone()));
                        self.push_log(msg);
                        self.commit_wireguard_config();
                        let _ = self.ui_tx.send(UiCmd::Scan);
                    }
                    Err(err) => {
                        let msg = format!("Erro ao processar arquivo .conf: {err}");
                        self.tunnel_modal_notice = Some((WARN, msg.clone()));
                        self.push_log(msg);
                    }
                },
                Err(err) => {
                    let msg = format!("Falha ao ler arquivo: {err}");
                    self.tunnel_modal_notice = Some((WARN, msg.clone()));
                    self.push_log(msg);
                }
            }
        }
    }

    fn reload_config_from_profile(&mut self) {
        let loaded = match AppConfig::load_or_default(&self.config_path) {
            Ok(config) => {
                self.config_valid = true;
                config
            }
            Err(err) => {
                let msg = format!(
                    "Falha ao recarregar configuração: {err}. A última configuração válida foi preservada; nada foi sobrescrito."
                );
                self.tunnel_modal_notice = Some((WARN, msg.clone()));
                self.push_log(msg);
                return;
            }
        };
        self.app_config = loaded;
        self.app_config.gui = self.app_config.gui.clone().normalized();
        self.wireguard_cfg = self.app_config.wireguard.clone();
        self.trim_game_memory = self.app_config.trim_game_memory;
        self.watch_interval = self.app_config.gui.watch_interval_secs;
        self.watching = self.app_config.gui.watch_enabled;
        let msg = format!(
            "Configurações recarregadas do perfil ({}).",
            self.config_path.display()
        );
        self.tunnel_modal_notice = Some((OK, msg.clone()));
        self.push_log(msg);
        let _ = self
            .ui_tx
            .send(UiCmd::ReplaceConfig(Box::new(self.app_config.clone())));
        let _ = self.ui_tx.send(UiCmd::Scan);
    }

    fn pick_game_exe(&mut self) {
        let dialog = rfd::FileDialog::new().set_title("Selecione o executável do Jogo");
        #[cfg(windows)]
        let dialog = dialog.add_filter("Executáveis (*.exe)", &["exe"]);
        if let Some(exe_path) = dialog.pick_file() {
            if let Some(file_name) = exe_path.file_name() {
                let name = file_name.to_string_lossy();
                if self.app_config.add_game_name(&name) {
                    self.push_log(format!("Jogo adicionado: {name}"));
                    self.save_app_config();
                    let _ = self.ui_tx.send(UiCmd::Scan);
                }
            }
        }
    }

    fn push_log(&mut self, line: impl AsRef<str>) {
        self.logs.push(line.as_ref().to_owned());
        if self.logs.len() > 80 {
            let drain = self.logs.len() - 80;
            self.logs.drain(0..drain);
        }
    }

    fn poll_worker(&mut self) {
        loop {
            let msg = match self.worker_rx.try_recv() {
                Ok(msg) => msg,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.worker_stopped = true;
                    break;
                }
            };
            match msg {
                WorkerMsg::Report(report) => {
                    #[cfg(target_os = "linux")]
                    self.game_tunnel_overrides
                        .retain(|game, _| GameProcess::read(game.pid) == Some(*game));
                    self.report = Some(report);
                }
                WorkerMsg::Log(line) => {
                    if line.starts_with("Ping VPS:")
                        || line.starts_with("Configuração salva no perfil.")
                    {
                        self.tunnel_modal_notice = Some((OK, line.clone()));
                    } else if line.starts_with("Aviso ping VPS:") {
                        self.tunnel_modal_notice = Some((ACCENT, line.clone()));
                    } else if line.starts_with("Erro ao salvar configuração:") {
                        self.tunnel_modal_notice = Some((WARN, line.clone()));
                    }
                    self.push_log(line);
                }
                WorkerMsg::Watching(active) => {
                    self.watching = active;
                }
                WorkerMsg::Tunnel(status) => {
                    #[cfg(target_os = "linux")]
                    if status
                        .message
                        .starts_with("Reconexão pelo túnel solicitada")
                    {
                        self.push_log(&status.message);
                    }
                    self.tunnel_status = Some(status);
                }
                WorkerMsg::Macros(statuses) => {
                    self.macro_statuses = statuses;
                }
                WorkerMsg::Processes(processes) => self.running_processes = processes,
                WorkerMsg::Stopped => self.worker_stopped = true,
            }
        }
    }

    #[cfg(windows)]
    fn poll_tray(&mut self, ctx: &egui::Context) {
        while let Ok(event) = self.tray_rx.try_recv() {
            match event {
                AppTrayEvent::Menu(ev) => {
                    if ev.id == self.tray_show.id() {
                        self.restore_window(ctx);
                    } else if ev.id == self.tray_optimize.id() {
                        if self.config_valid {
                            let _ = self.ui_tx.send(UiCmd::Optimize {
                                trim_game: self.trim_game_memory,
                            });
                        } else {
                            self.push_log(
                                "Otimização bloqueada: recarregue uma configuração válida.",
                            );
                        }
                    } else if ev.id == self.tray_quit.id() {
                        self.exiting = true;
                    }
                }
                AppTrayEvent::Tray(ev) => match ev {
                    TrayIconEvent::Click {
                        button: tray_icon::MouseButton::Left,
                        ..
                    }
                    | TrayIconEvent::DoubleClick {
                        button: tray_icon::MouseButton::Left,
                        ..
                    } => {
                        self.restore_window(ctx);
                    }
                    _ => {}
                },
            }
        }
    }

    #[cfg(target_os = "linux")]
    fn poll_linux_tray(&mut self, ctx: &egui::Context) -> bool {
        use linux_tray::Action;
        let mut restored = false;
        while let Some(action) = self
            .linux_tray
            .as_ref()
            .and_then(|tray| tray.events.try_recv().ok())
        {
            match action {
                Action::Open if !self.exiting => {
                    self.restore_window(ctx);
                    restored = true;
                }
                Action::Scan if self.config_valid && !self.exiting => {
                    let _ = self.ui_tx.send(UiCmd::Scan);
                }
                Action::Quit => self.exiting = true,
                Action::Online | Action::Offline => {
                    if let Some(tray) = &mut self.linux_tray {
                        tray.online = matches!(action, Action::Online);
                    }
                }
                _ => {}
            }
        }
        if self.hidden_to_tray && !self.exiting && !self.has_tray() {
            self.restore_window(ctx);
            restored = true;
        }
        restored
    }

    fn has_tray(&self) -> bool {
        #[cfg(windows)]
        return self.tray.is_some();
        #[cfg(target_os = "linux")]
        return self
            .linux_tray
            .as_ref()
            .is_some_and(linux_tray::LinuxTray::available);
        #[cfg(not(any(windows, target_os = "linux")))]
        false
    }

    fn hide_to_tray(&mut self, ctx: &egui::Context) {
        self.hidden_to_tray = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
    }

    #[cfg(any(windows, target_os = "linux"))]
    fn restore_window(&mut self, ctx: &egui::Context) {
        self.hidden_to_tray = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        ctx.request_repaint();
    }

    fn game_count(&self) -> usize {
        self.report.as_ref().map(|r| r.games.len()).unwrap_or(0)
    }
}

impl App for GuiApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        self.poll_worker();
        #[cfg(windows)]
        self.poll_tray(ctx);
        #[cfg(target_os = "linux")]
        let restored = self.poll_linux_tray(ctx);
        if self.exiting && self.worker_stopped {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            ctx.request_repaint();
            return;
        }

        let viewport = ctx.input(|i| i.viewport().clone());
        #[cfg(target_os = "linux")]
        {
            let minimized = viewport.minimized == Some(true);
            if minimized && !self.was_minimized && !restored && !self.exiting && self.has_tray() {
                self.hide_to_tray(ctx);
            }
            self.was_minimized = minimized;
        }
        if let (Some(rect), Some(maximized)) = (viewport.inner_rect, viewport.maximized) {
            let size = rect.size();
            let (width, height) = (
                size.x.round().max(0.0) as u32,
                size.y.round().max(0.0) as u32,
            );
            if (maximized || (width >= 640 && height >= 420))
                && ((!maximized
                    && (self.app_config.gui.window_width != width
                        || self.app_config.gui.window_height != height))
                    || self.app_config.gui.window_maximized != maximized)
            {
                if !maximized {
                    self.app_config.gui.window_width = width;
                    self.app_config.gui.window_height = height;
                }
                self.app_config.gui.window_maximized = maximized;
                self.geometry_dirty_at = Some(Instant::now());
            }
        }
        if self
            .geometry_dirty_at
            .is_some_and(|at| debounce_elapsed(at, Instant::now(), Duration::from_millis(450)))
        {
            self.geometry_dirty_at = None;
            if self.config_valid {
                self.queue_config_snapshot();
            }
        }

        let close_requested = ctx.input(|i| i.viewport().close_requested());
        if close_requested {
            if self.exiting || !self.has_tray() {
                if !self.exiting {
                    self.exiting = true;
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.hide_to_tray(ctx);
            }
            #[cfg(windows)]
            if !self.start_minimized_hint && !self.exiting {
                self.push_log("Na bandeja do sistema. Clique no ícone para reabrir.");
                self.start_minimized_hint = true;
            }
        }

        if self.exiting && !self.shutdown_sent {
            self.queue_config_snapshot();
            if self.ui_tx.send(UiCmd::Shutdown).is_ok() {
                self.shutdown_sent = true;
            } else {
                self.worker_stopped = true;
                ctx.request_repaint();
            }
        }

        let repaint_ms = if self.active_modal == Some(ActiveModal::Macro)
            && self.macro_statuses.iter().any(|s| s.active)
        {
            80
        } else {
            500
        };
        ctx.request_repaint_after(Duration::from_millis(repaint_ms));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut Frame) {
        let ctx = ui.ctx().clone();
        paint_bg(ui);

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(Color32::TRANSPARENT)
                    .inner_margin(egui::Margin::symmetric(16, 12)),
            )
            .show(ui, |ui| {
                self.draw_header(ui);
                ui.add_space(10.0);
                self.draw_body(ui, &ctx);
            });

        match self.active_modal {
            Some(ActiveModal::Tunnel) => self.draw_tunnel_modal(&ctx),
            Some(ActiveModal::Process) => self.draw_process_modal(&ctx),
            Some(ActiveModal::Macro) => self.draw_macro_modal(&ctx),
            None => {}
        }
    }
}

impl GuiApp {
    fn draw_header_actions(&mut self, ui: &mut egui::Ui, rtl: bool) {
        let mut draw_buttons = |ui: &mut egui::Ui| {
            let primary_label = if cfg!(target_os = "linux") {
                "Buscar Jogos"
            } else {
                "Otimizar Agora"
            };
            let primary = ui
                .add_enabled_ui(self.config_valid, |ui| {
                    primary_button(ui, primary_label, Some(Vec2::new(150.0, 36.0)), 13.0)
                })
                .inner;
            if primary.clicked() {
                let command = if cfg!(target_os = "linux") {
                    UiCmd::Scan
                } else {
                    UiCmd::Optimize {
                        trim_game: self.trim_game_memory,
                    }
                };
                let _ = self.ui_tx.send(command);
            }

            if self.has_tray()
                && secondary_button(
                    ui,
                    RichText::new("Bandeja").color(MUTED).size(12.0),
                    Some(Vec2::new(84.0, 36.0)),
                )
                .on_hover_text("Minimizar para a bandeja do sistema")
                .clicked()
            {
                self.hide_to_tray(ui.ctx());
            }
        };

        if rtl {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                draw_buttons(ui);
            });
        } else {
            ui.horizontal_wrapped(|ui| {
                draw_buttons(ui);
            });
        }
    }

    fn draw_header(&mut self, ui: &mut egui::Ui) {
        #[cfg(target_os = "linux")]
        let subtitle = "Tunelamento VPS WireGuard & Macros de Jogos";
        #[cfg(not(target_os = "linux"))]
        let subtitle = "Tunelamento VPS Autônomo & Acelerador de Jogos";

        let watching = self.watching;
        let draw_brand = |ui: &mut egui::Ui| {
            ui.horizontal(|ui| {
                let mark = ui.allocate_response(Vec2::splat(32.0), Sense::hover());
                ui.painter().circle_filled(
                    mark.rect.center(),
                    15.0,
                    Color32::from_rgba_unmultiplied(ACCENT.r(), ACCENT.g(), ACCENT.b(), 36),
                );
                ui.painter().circle_filled(mark.rect.center(), 9.0, ACCENT);

                ui.add_space(6.0);
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Game Optimizer")
                                .font(FontId::new(21.0, FontFamily::Proportional))
                                .color(TEXT)
                                .strong(),
                        );
                        ui.add_space(6.0);
                        status_chip(ui, watching);
                    });
                    ui.label(RichText::new(subtitle).color(MUTED).size(12.0));
                });
            });
        };

        if ui.available_width() >= 660.0 {
            ui.horizontal(|ui| {
                draw_brand(ui);
                self.draw_header_actions(ui, true);
            });
        } else {
            draw_brand(ui);
            ui.add_space(6.0);
            self.draw_header_actions(ui, false);
        }

        ui.add_space(10.0);
        // Metric KPI strip — uniform 28px pills
        ui.horizontal_wrapped(|ui| {
            if kpi_button(
                ui,
                "Jogos:",
                &self.game_count().to_string(),
                None,
                ACCENT,
                "Clique para gerenciar jogos monitorados",
            )
            .clicked()
            {
                self.process_tab = ProcessTab::Games;
                self.active_modal = Some(ActiveModal::Process);
            }

            #[cfg(windows)]
            {
                let (trim_label, trim_color) = if self.trim_game_memory {
                    ("Ativo (Jogos)", OK)
                } else if self.app_config.trim_non_game_memory {
                    ("Ativo (Segundo Plano)", OK)
                } else {
                    ("Desativado", MUTED)
                };
                if kpi_button(
                    ui,
                    "Trim:",
                    trim_label,
                    Some(trim_color),
                    trim_color,
                    "Clique para gerenciar regras de trim de memória",
                )
                .clicked()
                {
                    self.process_tab = ProcessTab::MemoryTrim;
                    self.active_modal = Some(ActiveModal::Process);
                }
            }

            kpi_badge(
                ui,
                "Intervalo:",
                &format!("{}s", self.watch_interval),
                None,
                TEXT,
            );

            let (tunnel_label, tunnel_col) = match &self.tunnel_status {
                Some(s) if s.health == TunnelHealth::Healthy => {
                    if let Some(ms) = s.vps_ping_ms {
                        (format!("Ativo ({ms}ms)"), OK)
                    } else if cfg!(windows) {
                        (format!("Ativo ({} rota(s))", s.active_routes_count), OK)
                    } else {
                        ("Ativo".into(), OK)
                    }
                }
                Some(s) if s.health == TunnelHealth::Connecting => ("Conectando…".into(), ACCENT),
                Some(s) if s.health == TunnelHealth::Degraded => ("Ver detalhes".into(), WARN),
                _ if self.wireguard_cfg.enabled => ("Pronto".into(), ACCENT),
                _ => ("Desativado".into(), MUTED),
            };

            if kpi_button(
                ui,
                "Túnel VPS:",
                &tunnel_label,
                Some(tunnel_col),
                tunnel_col,
                "Clique para abrir configurações e opções do túnel WireGuard",
            )
            .clicked()
            {
                self.active_modal = Some(ActiveModal::Tunnel);
            }
        });
    }

    fn draw_body(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) {
        if !self.config_valid {
            egui::Frame::new()
                .fill(Color32::from_rgb(55, 25, 22))
                .stroke(Stroke::new(1.0, WARN))
                .corner_radius(8.0)
                .inner_margin(8.0)
                .show(ui, |ui| {
                    ui.label(
                        RichText::new("Configuração inválida. Otimizações e gravações estão bloqueadas para preservar o perfil. Use ‘Recarregar do Perfil’ após corrigir o arquivo.")
                            .color(TEXT)
                            .strong(),
                    );
                });
            ui.add_space(6.0);
        }
        let available = ui.available_height().max(1.0);
        let width = ui.available_width();
        if narrow_layout(width) || available < 340.0 {
            let content_w = (width - ui.spacing().scroll.bar_width - 4.0).max(280.0);
            egui::ScrollArea::vertical()
                .id_salt(SCROLL_AREA_IDS[0])
                .max_height(available)
                .show(ui, |ui| {
                    ui.set_width(content_w);
                    self.draw_game_panels(ui, available, false);
                    ui.add_space(12.0);
                    self.draw_controls(ui);
                });
        } else {
            let gap = ui.spacing().item_spacing.x;
            let sidebar_width = (width * 0.34).clamp(260.0, 330.0);
            let main_width = (width - sidebar_width - gap).max(280.0);
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    Vec2::new(main_width, available),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| self.draw_game_panels(ui, available, true),
                );
                ui.allocate_ui_with_layout(
                    Vec2::new(sidebar_width, available),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt(SCROLL_AREA_IDS[3])
                            .max_height(available)
                            .auto_shrink([false; 2])
                            .show(ui, |ui| self.draw_controls(ui));
                    },
                );
            });
        }
    }

    fn draw_game_panels(&mut self, ui: &mut egui::Ui, available: f32, contain_scroll: bool) {
        let (list_h, log_h) = if contain_scroll {
            let usable = (available - 82.0).max(220.0);
            let list_h = (usable * 0.68).max(160.0);
            let log_h = (usable - list_h).max(88.0);
            (list_h, log_h)
        } else {
            (260.0, 150.0)
        };

        section_label(ui, "JOGOS EM EXECUÇÃO");
        egui::Frame::new()
            .fill(BG_RAISED)
            .stroke(Stroke::new(1.0, STROKE))
            .corner_radius(10.0)
            .inner_margin(12.0)
            .show(ui, |ui| {
                let list = egui::ScrollArea::vertical()
                    .id_salt(SCROLL_AREA_IDS[1])
                    .max_height(list_h)
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        self.draw_games(ui);
                    });
                if contain_scroll {
                    contain_list_scroll(ui, list.inner_rect);
                }
            });

        ui.add_space(10.0);
        section_label(ui, "LOG DE ATIVIDADES DO TÚNEL & SISTEMA");
        egui::Frame::new()
            .fill(Color32::from_rgb(11, 13, 17))
            .stroke(Stroke::new(1.0, STROKE))
            .corner_radius(10.0)
            .inner_margin(10.0)
            .show(ui, |ui| {
                let rest = if contain_scroll {
                    ui.available_height().min(log_h).max(75.0)
                } else {
                    log_h
                };
                let log = egui::ScrollArea::vertical()
                    .id_salt(SCROLL_AREA_IDS[2])
                    .max_height(rest)
                    .auto_shrink([false; 2])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        for line in self.logs.iter().rev().take(50).rev() {
                            let col = if line.contains("Túnel VPS") {
                                TUNNEL_COLOR
                            } else if line.contains("Otimização") || line.contains("ok") {
                                OK
                            } else if line.contains("Erro") || line.contains("Falha") {
                                WARN
                            } else {
                                MUTED
                            };
                            ui.label(RichText::new(line).color(col).size(12.0).monospace());
                        }
                    });
                if contain_scroll {
                    contain_list_scroll(ui, log.inner_rect);
                }
            });
    }

    fn draw_controls(&mut self, ui: &mut egui::Ui) {
        section_label(ui, "AÇÕES");
        egui::Frame::new()
            .fill(BG_RAISED)
            .stroke(Stroke::new(1.0, STROKE))
            .corner_radius(10.0)
            .inner_margin(10.0)
            .show(ui, |ui| {
                let full_w = ui.available_width();
                if secondary_button(
                    ui,
                    RichText::new("Atualizar Detecção").color(TEXT).strong(),
                    Some(Vec2::new(full_w, 34.0)),
                )
                .clicked()
                {
                    let _ = self.ui_tx.send(UiCmd::Scan);
                }
                ui.add_space(6.0);
                if self.watching {
                    if danger_button(ui, "Parar Vigilância", Some(Vec2::new(full_w, 34.0)), 13.0)
                        .clicked()
                    {
                        self.watching = false;
                        self.app_config.gui.watch_enabled = false;
                        let _ = self.ui_tx.send(UiCmd::StopWatch);
                        self.save_app_config();
                    }
                } else if self.config_valid
                    && secondary_button(
                        ui,
                        RichText::new("Vigiar Automaticamente").color(TEXT).strong(),
                        Some(Vec2::new(full_w, 34.0)),
                    )
                    .clicked()
                {
                    self.watching = true;
                    self.app_config.gui.watch_enabled = true;
                    let _ = self.ui_tx.send(UiCmd::StartWatch {
                        interval: self.watch_interval,
                        trim_game: self.trim_game_memory,
                    });
                    self.save_app_config();
                }
            });

        ui.add_space(10.0);
        section_label(ui, "AJUSTES");
        egui::Frame::new()
            .fill(BG_RAISED)
            .stroke(Stroke::new(1.0, STROKE))
            .corner_radius(10.0)
            .inner_margin(10.0)
            .show(ui, |ui| {
                let full_w = ui.available_width();
                if ui
                    .checkbox(
                        &mut self.autostart,
                        RichText::new("Iniciar ao entrar na sessão")
                            .color(TEXT)
                            .size(13.0),
                    )
                    .on_hover_text(if cfg!(windows) {
                        "O Windows pedirá autorização de administrador (UAC) ao entrar."
                    } else {
                        "Abre o Game Optimizer ao entrar na sessão gráfica."
                    })
                    .changed()
                {
                    let requested = self.autostart;
                    match autostart::set_enabled(requested) {
                        Ok(()) => self.push_log(if self.autostart {
                            "Inicialização automática ativada."
                        } else {
                            "Inicialização automática desativada."
                        }),
                        Err(err) => {
                            self.autostart = autostart::is_enabled();
                            self.push_log(format!(
                                "Falha ao alterar inicialização automática: {err}"
                            ));
                        }
                    }
                }

                #[cfg(windows)]
                {
                    ui.add_space(4.0);
                    if ui
                        .checkbox(
                            &mut self.trim_game_memory,
                            RichText::new("Trim de memória nos Jogos")
                                .color(TEXT)
                                .size(13.0),
                        )
                        .changed()
                    {
                        self.push_log(if self.trim_game_memory {
                            "Trim de memória ativado EXCLUSIVAMENTE para jogos."
                        } else {
                            "Trim de memória de jogos desativado."
                        });
                        self.app_config.trim_game_memory = self.trim_game_memory;
                        if self.watching {
                            let _ = self.ui_tx.send(UiCmd::StartWatch {
                                interval: self.watch_interval,
                                trim_game: self.trim_game_memory,
                            });
                        }
                        self.save_app_config();
                    }
                    ui.label(
                        RichText::new(
                            "Focado apenas nos jogos ativos. Apps em 2º plano nunca sofrem trim.",
                        )
                        .color(MUTED)
                        .size(11.0),
                    );
                }

                ui.add_space(8.0);
                ui.label(
                    RichText::new("Intervalo de vigilância (s)")
                        .color(MUTED)
                        .size(12.0),
                );
                ui.spacing_mut().slider_width = (full_w - 75.0).max(100.0);
                let interval_changed = ui
                    .add(
                        egui::Slider::new(&mut self.watch_interval, 15..=120)
                            .clamping(egui::SliderClamping::Always),
                    )
                    .changed();
                if interval_changed {
                    self.app_config.gui.watch_interval_secs = self.watch_interval;
                    if self.watching {
                        let _ = self.ui_tx.send(UiCmd::StartWatch {
                            interval: self.watch_interval,
                            trim_game: self.trim_game_memory,
                        });
                    }
                    self.save_app_config();
                }

                ui.add_space(6.0);
                if secondary_button(
                    ui,
                    RichText::new("Gerenciar Jogos & Processos")
                        .color(TEXT)
                        .strong(),
                    Some(Vec2::new(full_w, 34.0)),
                )
                .clicked()
                {
                    self.active_modal = Some(ActiveModal::Process);
                    self.process_tab = ProcessTab::RunningProcesses;
                    self.refresh_running_processes();
                }

                ui.add_space(10.0);
                section_label(ui, "TÚNEL WIREGUARD (VPS)");

                if styled_button(
                    ui,
                    RichText::new("Importar / Reimportar (.conf)")
                        .color(ACCENT)
                        .strong(),
                    Some(Vec2::new(full_w, 34.0)),
                    ACCENT_SOFT,
                    Color32::from_rgb(78, 56, 24),
                    Color32::from_rgb(96, 68, 28),
                    Stroke::new(1.0, ACCENT.gamma_multiply(0.7)),
                    7,
                )
                .on_hover_text(
                    "Selecione um arquivo .conf do WireGuard para importar ou reimportar suas credenciais imediatamente",
                )
                .clicked()
                {
                    self.import_conf_file();
                }

                ui.add_space(4.0);
                if secondary_button(
                    ui,
                    RichText::new("Configurações do Túnel")
                        .color(TEXT)
                        .strong(),
                    Some(Vec2::new(full_w, 32.0)),
                )
                .on_hover_text(
                    "Abra a janela de opções do túnel WireGuard, ping da VPS e rotas ativas",
                )
                .clicked()
                {
                    self.active_modal = Some(ActiveModal::Tunnel);
                }

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Perfil:").color(MUTED).size(11.0));
                    let path_str = self.config_path.display().to_string();
                    ui.add(
                        egui::Label::new(
                            RichText::new(&path_str)
                                .color(ACCENT)
                                .size(11.0)
                                .monospace(),
                        )
                        .truncate(),
                    )
                    .on_hover_text(path_str);
                });
            });
    }

    fn draw_games(&mut self, ui: &mut egui::Ui) {
        let games = match &self.report {
            Some(report) => report.games.clone(),
            None => {
                ui.label(RichText::new("Carregando processos…").color(MUTED));
                return;
            }
        };

        if games.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(12.0);
                ui.label(
                    RichText::new("Nenhum jogo detectado no momento.")
                        .color(TEXT)
                        .strong()
                        .size(13.5),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "Inicie seu jogo ou escolha diretamente dos processos em execução.",
                    )
                    .color(MUTED)
                    .size(12.0),
                );
                ui.add_space(10.0);
                if styled_button(
                    ui,
                    RichText::new("Escolher dos Processos em Execução")
                        .color(ACCENT)
                        .strong()
                        .size(12.5),
                    Some(Vec2::new(260.0, 34.0)),
                    ACCENT_SOFT,
                    Color32::from_rgb(78, 56, 24),
                    Color32::from_rgb(96, 68, 28),
                    Stroke::new(1.0, ACCENT.gamma_multiply(0.7)),
                    7,
                )
                .on_hover_text(
                    "Escolha diretamente da lista de processos abertos sem navegar pastas",
                )
                .clicked()
                {
                    self.active_modal = Some(ActiveModal::Process);
                    self.process_tab = ProcessTab::RunningProcesses;
                    self.refresh_running_processes();
                }
                ui.add_space(12.0);
            });
        } else {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("JOGOS ATIVOS")
                        .color(MUTED)
                        .size(11.0)
                        .strong(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if secondary_button(
                        ui,
                        RichText::new("+ Adicionar processo").color(TEXT).size(11.5),
                        None,
                    )
                    .on_hover_text("Escolha outro processo aberto para monitorar")
                    .clicked()
                    {
                        self.active_modal = Some(ActiveModal::Process);
                        self.process_tab = ProcessTab::RunningProcesses;
                        self.refresh_running_processes();
                    }
                });
            });
            ui.add_space(6.0);

            let total_w = ui.available_width();
            let (cols, card_w) = game_card_layout(total_w);
            let macro_supported = crate::macros::input_supported();
            let card_min_h = if macro_supported { 148.0 } else { 88.0 };

            for row in games.chunks(cols) {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(10.0, 10.0);
                    for game in row {
                        let inst_status = self
                            .macro_statuses
                            .iter()
                            .find(|s| s.pid == game.pid)
                            .cloned();
                        let profile_key = self.instance_profile_key(
                            &game.normalized_name,
                            inst_status.as_ref().and_then(|s| s.window_title.as_deref()),
                            game.pid,
                        );
                        let fallback_profile = self
                            .app_config
                            .macros
                            .get(&profile_key)
                            .cloned()
                            .unwrap_or_default()
                            .normalized();
                        let rules_count = inst_status
                            .as_ref()
                            .map(|s| s.rules.len())
                            .unwrap_or(fallback_profile.rules.len());
                        let enabled_rules_count = inst_status
                            .as_ref()
                            .map(|s| s.rules.iter().filter(|r| r.enabled).count())
                            .unwrap_or_else(|| fallback_profile.enabled_rules_count());
                        let is_macro_active = inst_status.as_ref().is_some_and(|s| s.active);
                        let total_sent = inst_status.as_ref().map(|s| s.total_triggers).unwrap_or(0);
                        let win_title = inst_status.as_ref().and_then(|s| s.window_title.clone());
                        let toggle_key = inst_status
                            .as_ref()
                            .map(|s| s.toggle_key.clone())
                            .unwrap_or(fallback_profile.toggle_key);
                        let toggle_label = if toggle_key.trim().is_empty() {
                            "Atalho"
                        } else {
                            key_display_label(&toggle_key)
                        };

                        let card_stroke = if is_macro_active {
                            Stroke::new(1.4, OK.gamma_multiply(0.8))
                        } else {
                            Stroke::new(1.0, STROKE)
                        };

                        ui.allocate_ui_with_layout(
                            Vec2::new(card_w, 0.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                egui::Frame::new()
                                    .fill(BG_ROW)
                                    .stroke(card_stroke)
                                    .corner_radius(CornerRadius::same(12))
                                    .inner_margin(14.0)
                                    .show(ui, |ui| {
                                        let inner_w = (card_w - 28.0).max(180.0);
                                        ui.set_width(inner_w);
                                        ui.set_min_height(card_min_h - 28.0);

                                        // 1. Card Header: Reserved title space + right-aligned RAM badge
                                        ui.horizontal(|ui| {
                                            let ram_str = format_mib(game.memory_bytes);
                                            let title_w = (inner_w - 88.0).max(80.0);
                                            ui.allocate_ui_with_layout(
                                                Vec2::new(title_w, 22.0),
                                                egui::Layout::left_to_right(egui::Align::Center),
                                                |ui| {
                                                    ui.set_max_width(title_w);
                                                    if macro_supported {
                                                        let title_btn = ui
                                                            .add(
                                                                egui::Button::new(
                                                                    RichText::new(&game.name)
                                                                        .color(TEXT)
                                                                        .strong()
                                                                        .size(14.5),
                                                                )
                                                                .frame(false)
                                                                .truncate(),
                                                            )
                                                            .on_hover_cursor(
                                                                egui::CursorIcon::PointingHand,
                                                            )
                                                            .on_hover_text(
                                                                "Clique para abrir a configuração de Hotkeys / Macros autônomas desta janela",
                                                            );
                                                        if title_btn.clicked() {
                                                            self.open_macro_modal_for_game(
                                                                game.pid,
                                                                &game.name,
                                                                &game.normalized_name,
                                                            );
                                                        }
                                                    } else {
                                                        ui.add(
                                                            egui::Label::new(
                                                                RichText::new(&game.name)
                                                                    .color(TEXT)
                                                                    .strong()
                                                                    .size(14.5),
                                                            )
                                                            .truncate(),
                                                        );
                                                    }
                                                },
                                            );

                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    egui::Frame::new()
                                                        .fill(BG_RAISED)
                                                        .stroke(Stroke::new(1.0, STROKE))
                                                        .corner_radius(6.0)
                                                        .inner_margin(egui::Margin::symmetric(6, 2))
                                                        .show(ui, |ui| {
                                                            ui.label(
                                                                RichText::new(ram_str)
                                                                    .color(ACCENT)
                                                                    .strong()
                                                                    .size(11.5),
                                                            );
                                                        });
                                                },
                                            );
                                        });

                                        // Single clean metadata line (window + PID)
                                        let win_pill_text = match win_title
                                            .as_deref()
                                            .map(str::trim)
                                            .filter(|t| {
                                                !t.is_empty() && !t.eq_ignore_ascii_case(&game.name)
                                            }) {
                                            Some(t) => format!("Janela: {t} • PID {}", game.pid),
                                            None => format!("PID {}", game.pid),
                                        };
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(win_pill_text)
                                                    .color(MUTED)
                                                    .size(11.5)
                                                    .monospace(),
                                            )
                                            .truncate(),
                                        );

                                        #[cfg(target_os = "linux")]
                                        {
                                            if let Some(instance) = GameProcess::read(game.pid) {
                                                let enabled = self
                                                    .game_tunnel_overrides
                                                    .get(&instance)
                                                    .copied()
                                                    .unwrap_or(self.wireguard_cfg.auto_route_games);
                                                let scoped = is_scoped_game(game.pid);
                                                let ready = self
                                                    .tunnel_status
                                                    .as_ref()
                                                    .is_some_and(|status| status.active);
                                                let configured = self.wireguard_cfg.enabled
                                                    && self.wireguard_cfg.is_configured();
                                                let (label, color) = if !configured {
                                                    ("Conexão local (túnel desativado)", MUTED)
                                                } else if !enabled && scoped {
                                                    ("Mudando para a rota local…", MUTED)
                                                } else if !enabled {
                                                    ("Conexão local", MUTED)
                                                } else if !ready {
                                                    ("Aguardando o túnel da VPS", ACCENT)
                                                } else if !scoped {
                                                    ("Vinculando jogo ao túnel…", ACCENT)
                                                } else {
                                                    ("Túnel ativo para novas conexões", OK)
                                                };
                                                ui.add_space(4.0);
                                                ui.horizontal_wrapped(|ui| {
                                                    ui.spacing_mut().button_padding =
                                                        Vec2::new(8.0, 4.0);
                                                    if ui
                                                        .add_enabled(
                                                            self.config_valid && configured,
                                                            egui::Button::new(
                                                                RichText::new(if enabled {
                                                                    "Desativar túnel"
                                                                } else {
                                                                    "Ativar túnel"
                                                                })
                                                                .size(11.0),
                                                            )
                                                            .corner_radius(6),
                                                        )
                                                        .on_hover_text(
                                                            "Troca a rota desta instância e solicita a reconexão TCP do jogo.",
                                                        )
                                                        .clicked()
                                                        && self
                                                            .ui_tx
                                                            .send(UiCmd::SetGameTunnel(
                                                                instance, !enabled,
                                                            ))
                                                            .is_ok()
                                                    {
                                                        self.game_tunnel_overrides
                                                            .insert(instance, !enabled);
                                                    }
                                                    status_dot(ui, color, 3.5);
                                                    ui.add(
                                                        egui::Label::new(
                                                            RichText::new(label)
                                                                .color(color)
                                                                .size(11.0),
                                                        )
                                                        .truncate(),
                                                    );
                                                });
                                            }
                                            if !macro_supported {
                                                return;
                                            }
                                        }

                                        ui.add_space(6.0);
                                        ui.separator();
                                        ui.add_space(6.0);

                                        // 2. Status Pills Section
                                        #[cfg(windows)]
                                        {
                                            let (tunnel_text, tunnel_dot_col) = match &self.tunnel_status {
                                                Some(status) if status.health == TunnelHealth::Healthy => {
                                                    let route_suffix = status
                                                        .routed_destinations
                                                        .first()
                                                        .map(|d| format!(" • {d}"))
                                                        .unwrap_or_default();
                                                    (format!("Tunelado via VPS{route_suffix}"), OK)
                                                }
                                                Some(status)
                                                    if status.health == TunnelHealth::Connecting =>
                                                {
                                                    ("Conectando VPS…".to_string(), ACCENT)
                                                }
                                                Some(status)
                                                    if status.health == TunnelHealth::Degraded =>
                                                {
                                                    ("Fallback direto ativo".to_string(), WARN)
                                                }
                                                _ => ("Otimizado (Local)".to_string(), MUTED),
                                            };
                                            ui.horizontal(|ui| {
                                                status_dot(ui, tunnel_dot_col, 3.5);
                                                ui.add(
                                                    egui::Label::new(
                                                        RichText::new(tunnel_text)
                                                            .color(tunnel_dot_col)
                                                            .size(11.5)
                                                            .strong(),
                                                    )
                                                    .truncate(),
                                                );
                                            });
                                            ui.add_space(3.0);
                                        }

                                        let (macro_text, macro_col) = if is_macro_active {
                                            (
                                                format!(
                                                    "Macro Ativo • {enabled_rules_count} tecla(s) ({total_sent}x) • [{toggle_label}]"
                                                ),
                                                OK,
                                            )
                                        } else if rules_count > 0 {
                                            (
                                                format!(
                                                    "Pausado • {rules_count} tecla(s) • [{toggle_label}]"
                                                ),
                                                ACCENT,
                                            )
                                        } else {
                                            ("Sem hotkeys configuradas".to_string(), MUTED)
                                        };
                                        ui.horizontal(|ui| {
                                            status_dot(ui, macro_col, 3.5);
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(macro_text)
                                                        .color(macro_col)
                                                        .size(11.5),
                                                )
                                                .truncate(),
                                            );
                                        });

                                        ui.add_space(8.0);

                                        // 3. Card Footer Action Buttons
                                        ui.scope(|ui| {
                                            ui.spacing_mut().button_padding = Vec2::new(8.0, 5.0);
                                            if rules_count > 0 {
                                                let btn_w = ((inner_w - 6.0) * 0.5).max(80.0);
                                                ui.horizontal(|ui| {
                                                    ui.spacing_mut().item_spacing.x = 6.0;
                                                    if is_macro_active {
                                                        if danger_button(
                                                            ui,
                                                            &format!("⏸ Pausar ({toggle_label})"),
                                                            Some(Vec2::new(btn_w, 28.0)),
                                                            11.0,
                                                        )
                                                        .on_hover_text(
                                                            "Pausar o envio autônomo de hotkeys apenas para esta janela",
                                                        )
                                                        .clicked()
                                                        {
                                                            self.set_game_macro_active(
                                                                game.pid,
                                                                &game.normalized_name,
                                                                false,
                                                            );
                                                        }
                                                    } else {
                                                        let can_start = enabled_rules_count > 0;
                                                        let start_btn = ui
                                                            .add_enabled_ui(can_start, |ui| {
                                                                primary_button(
                                                                    ui,
                                                                    &format!(
                                                                        "▶ Iniciar ({toggle_label})"
                                                                    ),
                                                                    Some(Vec2::new(btn_w, 28.0)),
                                                                    11.0,
                                                                )
                                                            })
                                                            .inner;
                                                        if start_btn
                                                            .on_hover_text(
                                                                "Iniciar o envio autônomo de hotkeys apenas para esta janela",
                                                            )
                                                            .clicked()
                                                        {
                                                            self.set_game_macro_active(
                                                                game.pid,
                                                                &game.normalized_name,
                                                                true,
                                                            );
                                                        }
                                                    }

                                                    if secondary_button(
                                                        ui,
                                                        RichText::new(format!(
                                                            "Hotkeys ({rules_count})"
                                                        ))
                                                        .color(TEXT)
                                                        .size(11.0)
                                                        .strong(),
                                                        Some(Vec2::new(btn_w, 28.0)),
                                                    )
                                                    .on_hover_text(
                                                        "Configurar hotkeys exclusivas para esta janela do jogo",
                                                    )
                                                    .clicked()
                                                    {
                                                        self.open_macro_modal_for_game(
                                                            game.pid,
                                                            &game.name,
                                                            &game.normalized_name,
                                                        );
                                                    }
                                                });
                                            } else if secondary_button(
                                                ui,
                                                RichText::new("Configurar Hotkeys")
                                                    .color(TEXT)
                                                    .size(11.5)
                                                    .strong(),
                                                Some(Vec2::new(inner_w, 28.0)),
                                            )
                                            .on_hover_text(
                                                "Configurar envio autônomo de teclas exclusivo para esta janela do jogo",
                                            )
                                            .clicked()
                                            {
                                                self.open_macro_modal_for_game(
                                                    game.pid,
                                                    &game.name,
                                                    &game.normalized_name,
                                                );
                                            }
                                        });
                                    });
                            },
                        );
                    }
                });
                ui.add_space(10.0);
            }
        }
    }

    fn draw_tunnel_modal(&mut self, ctx: &egui::Context) {
        let status_subtitle = match &self.tunnel_status {
            Some(s) if s.health == TunnelHealth::Healthy => match s.vps_ping_ms {
                Some(ms) => format!("Conectado • Latência VPS: {ms} ms"),
                None => "Conectado e saudável".to_string(),
            },
            Some(s) if s.health == TunnelHealth::Connecting => {
                "Aguardando handshake com a VPS…".to_string()
            }
            Some(s) if s.health == TunnelHealth::Degraded => {
                "Falha VPS • Fallback direto ativo".to_string()
            }
            _ if self.wireguard_cfg.enabled => "Configurado no perfil".to_string(),
            _ => "Túnel desativado no perfil".to_string(),
        };

        let (should_close, ()) = show_centered_modal(
            ctx,
            "tunnel_modal",
            "Configuração do Túnel WireGuard (VPS)",
            Some(&status_subtitle),
            Vec2::new(660.0, 560.0),
            false,
            true,
            |ui| {
                let body_h = (ui.available_height() - 52.0).max(120.0);
                egui::ScrollArea::vertical()
                    .id_salt(SCROLL_AREA_IDS[4])
                    .max_height(body_h)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let content_w = (ui.available_width() - 6.0).max(240.0);
                        ui.set_width(content_w);

                        if let Some((notice_col, notice_msg)) = self.tunnel_modal_notice.clone() {
                            let mut clear_notice = false;
                            egui::Frame::new()
                                .fill(BG_RAISED)
                                .stroke(Stroke::new(1.0, notice_col.gamma_multiply(0.7)))
                                .corner_radius(8.0)
                                .inner_margin(egui::Margin::symmetric(10, 7))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        status_dot(ui, notice_col, 3.5);
                                        let text_w = (ui.available_width() - 32.0).max(120.0);
                                        ui.allocate_ui_with_layout(
                                            Vec2::new(text_w, 0.0),
                                            egui::Layout::top_down(egui::Align::Min),
                                            |ui| {
                                                ui.label(
                                                    RichText::new(&notice_msg)
                                                        .color(TEXT)
                                                        .size(11.5),
                                                );
                                            },
                                        );
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if ui
                                                    .small_button(
                                                        RichText::new("✕").color(MUTED).size(11.0),
                                                    )
                                                    .clicked()
                                                {
                                                    clear_notice = true;
                                                }
                                            },
                                        );
                                    });
                                });
                            if clear_notice {
                                self.tunnel_modal_notice = None;
                            }
                            ui.add_space(8.0);
                        }

                        ui.label(
                            RichText::new(if cfg!(target_os = "linux") {
                                "O Game Optimizer vincula os jogos detectados ao túnel automaticamente. O Linux pode solicitar autorização para configurar a rede."
                            } else {
                                "Isolamento por PID: as conexões do jogo são direcionadas para a VPS."
                            })
                            .color(MUTED)
                            .size(12.0),
                        );
                        ui.add_space(8.0);

                        #[cfg(target_os = "linux")]
                        if let Some(status) = &self.tunnel_status {
                            ui.label(RichText::new(&status.message).color(TEXT).size(12.0));
                            ui.add_space(8.0);
                        }

                        ui.horizontal_wrapped(|ui| {
                            if ui
                                .checkbox(
                                    &mut self.wireguard_cfg.enabled,
                                    RichText::new("Habilitar túnel no perfil")
                                        .strong()
                                        .color(TEXT),
                                )
                                .changed()
                                && cfg!(target_os = "linux")
                            {
                                self.commit_wireguard_config();
                            }
                            #[cfg(windows)]
                            if let Some(status) = &self.tunnel_status {
                                let (lbl, col) = match status.health {
                                    TunnelHealth::Healthy => match status.vps_ping_ms {
                                        Some(ms) => (format!("Latência VPS: {ms}ms"), OK),
                                        None => ("Conectado e saudável".to_string(), OK),
                                    },
                                    TunnelHealth::Connecting => {
                                        ("Aguardando handshake…".to_string(), ACCENT)
                                    }
                                    TunnelHealth::Degraded => {
                                        ("Falha VPS — Fallback ativo".to_string(), WARN)
                                    }
                                    TunnelHealth::Idle => (String::new(), MUTED),
                                };
                                if !lbl.is_empty() {
                                    ui.add_space(8.0);
                                    status_dot(ui, col, 3.5);
                                    ui.label(RichText::new(lbl).color(col).strong().size(12.0));
                                }
                            }
                        });

                        #[cfg(windows)]
                        if self.wireguard_cfg.enabled
                            && self
                                .tunnel_status
                                .as_ref()
                                .is_some_and(|s| s.health == TunnelHealth::Degraded)
                        {
                            ui.add_space(6.0);
                            egui::Frame::new()
                                .fill(Color32::from_rgb(38, 22, 18))
                                .stroke(Stroke::new(1.0, Color32::from_rgb(180, 80, 60)))
                                .corner_radius(7.0)
                                .inner_margin(8.0)
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new("Proteção Ativa contra Quedas: A VPS não respondeu ou a conexão falhou. As rotas dos jogos foram desviadas automaticamente para a sua internet local. Seu jogo continua funcionando diretamente sem cair! Assim que a VPS voltar a responder, o tunelamento otimizado será reativado.")
                                            .color(Color32::from_rgb(255, 200, 180))
                                            .size(11.0),
                                    );
                                });
                        }

                        ui.add_space(10.0);
                        ui.horizontal_wrapped(|ui| {
                            if styled_button(
                                ui,
                                RichText::new("Importar / Reimportar (.conf)")
                                    .color(ACCENT)
                                    .strong()
                                    .size(12.0),
                                None,
                                ACCENT_SOFT,
                                Color32::from_rgb(78, 56, 24),
                                Color32::from_rgb(96, 68, 28),
                                Stroke::new(1.0, ACCENT.gamma_multiply(0.7)),
                                7,
                            )
                            .on_hover_text(
                                "Selecione um arquivo .conf do WireGuard para substituir as credenciais atuais",
                            )
                            .clicked()
                            {
                                self.import_conf_file();
                            }

                            if secondary_button(
                                ui,
                                RichText::new(if self.show_paste_conf {
                                    "Fechar Editor"
                                } else {
                                    "Colar Configuração (.conf)"
                                })
                                .color(TEXT)
                                .size(12.0),
                                None,
                            )
                            .on_hover_text("Abra um campo para colar o texto do arquivo .conf")
                            .clicked()
                            {
                                self.show_paste_conf = !self.show_paste_conf;
                            }

                            if secondary_button(
                                ui,
                                RichText::new("Recarregar do Perfil")
                                    .color(MUTED)
                                    .size(12.0),
                                None,
                            )
                            .on_hover_text("Recarregar configurações salvas no perfil")
                            .clicked()
                            {
                                self.reload_config_from_profile();
                            }
                        });

                        if self.show_paste_conf {
                            ui.add_space(8.0);
                            egui::Frame::new()
                                .fill(BG_RAISED)
                                .stroke(Stroke::new(1.0, STROKE))
                                .corner_radius(8.0)
                                .inner_margin(10.0)
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new(
                                            "Cole o conteúdo do seu arquivo WireGuard .conf abaixo:",
                                        )
                                        .color(MUTED)
                                        .size(11.5),
                                    );
                                    ui.add_space(4.0);
                                    ui.add(
                                        egui::TextEdit::multiline(&mut self.paste_conf_text)
                                            .hint_text("[Interface]\nPrivateKey = ...\nAddress = 10.8.0.2/24\n\n[Peer]\nPublicKey = ...\nEndpoint = 198.51.100.1:51820")
                                            .desired_rows(6)
                                            .desired_width(ui.available_width()),
                                    );
                                    ui.add_space(6.0);
                                    ui.horizontal(|ui| {
                                        if primary_button(
                                            ui,
                                            "Aplicar Texto Colado",
                                            None,
                                            12.0,
                                        )
                                        .clicked()
                                        {
                                            match WireGuardConfig::parse_conf(&self.paste_conf_text)
                                            {
                                                Ok(parsed) => {
                                                    self.wireguard_cfg = parsed;
                                                    let msg = "Configuração WireGuard colada e aplicada com sucesso.";
                                                    self.tunnel_modal_notice =
                                                        Some((OK, msg.to_string()));
                                                    self.push_log(msg);
                                                    self.commit_wireguard_config();
                                                    self.show_paste_conf = false;
                                                    self.paste_conf_text.clear();
                                                }
                                                Err(err) => {
                                                    let msg = format!(
                                                        "Erro no formato colado: {err}"
                                                    );
                                                    self.tunnel_modal_notice =
                                                        Some((WARN, msg.clone()));
                                                    self.push_log(msg);
                                                }
                                            }
                                        }
                                        if secondary_button(
                                            ui,
                                            RichText::new("Cancelar").color(MUTED).size(12.0),
                                            None,
                                        )
                                        .clicked()
                                        {
                                            self.show_paste_conf = false;
                                        }
                                    });
                                });
                        }

                        ui.add_space(10.0);
                        let field_w = (content_w - 190.0).max(180.0);
                        egui::Grid::new("wg_grid")
                            .num_columns(2)
                            .spacing([12.0, 8.0])
                            .show(ui, |ui| {
                                ui.label("Endpoint VPS (IP:Porta):");
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.wireguard_cfg.endpoint)
                                        .hint_text("ex: 198.51.100.1:51820")
                                        .desired_width(field_w),
                                );
                                ui.end_row();

                                ui.label("Chave Privada (Cliente):");
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 6.0;
                                    let key_w = (field_w - 78.0).max(100.0);
                                    ui.add(
                                        egui::TextEdit::singleline(
                                            &mut self.wireguard_cfg.private_key,
                                        )
                                        .password(!self.show_wg_private_key)
                                        .hint_text("Chave privada base64")
                                        .desired_width(key_w),
                                    );
                                    if secondary_button(
                                        ui,
                                        RichText::new(if self.show_wg_private_key {
                                            "Ocultar"
                                        } else {
                                            "Mostrar"
                                        })
                                        .color(MUTED)
                                        .size(11.0),
                                        Some(Vec2::new(72.0, 26.0)),
                                    )
                                    .clicked()
                                    {
                                        self.show_wg_private_key = !self.show_wg_private_key;
                                    }
                                });
                                ui.end_row();

                                ui.label("Chave Pública (VPS):");
                                ui.add(
                                    egui::TextEdit::singleline(
                                        &mut self.wireguard_cfg.peer_public_key,
                                    )
                                    .hint_text("Chave pública base64 da VPS")
                                    .desired_width(field_w),
                                );
                                ui.end_row();

                                ui.label("IP do Cliente no Túnel:");
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.wireguard_cfg.tunnel_ip)
                                        .hint_text("10.0.0.2")
                                        .desired_width(field_w),
                                );
                                ui.end_row();

                                ui.label("Gateway da VPN (VPS):");
                                ui.add(
                                    egui::TextEdit::singleline(
                                        &mut self.wireguard_cfg.tunnel_gateway,
                                    )
                                    .hint_text("10.0.0.1")
                                    .desired_width(field_w),
                                );
                                ui.end_row();

                                ui.label("DNS Opcional:");
                                let mut dns_str = self.wireguard_cfg.dns.clone().unwrap_or_default();
                                if ui
                                    .add(
                                        egui::TextEdit::singleline(&mut dns_str)
                                            .hint_text("1.1.1.1")
                                            .desired_width(field_w),
                                    )
                                    .changed()
                                {
                                    self.wireguard_cfg.dns = if dns_str.trim().is_empty() {
                                        None
                                    } else {
                                        Some(dns_str)
                                    };
                                }
                                ui.end_row();
                            });

                        if let Some(status) = &self.tunnel_status {
                            if !status.routed_destinations.is_empty() {
                                ui.add_space(10.0);
                                ui.label(
                                    RichText::new(format!(
                                        "Rotas de servidores ativas no momento ({}):",
                                        status.routed_destinations.len()
                                    ))
                                    .color(OK)
                                    .size(12.0)
                                    .strong(),
                                );
                                for dest in &status.routed_destinations {
                                    ui.label(
                                        RichText::new(format!("  • {dest} -> VPS Gateway"))
                                            .color(TEXT)
                                            .monospace()
                                            .size(11.0),
                                    );
                                }
                            }
                        }

                        ui.add_space(10.0);
                        ui.collapsing("Comandos necessários na VPS Linux", |ui| {
                            ui.label(
                                RichText::new(
                                    "Para que a VPS encaminhe os pacotes dos jogos para os servidores de jogo, garanta na VPS:\n\
                                    1. sysctl -w net.ipv4.ip_forward=1\n\
                                    2. iptables -t nat -A POSTROUTING -o eth0 -j MASQUERADE\n\
                                    (Substitua eth0 pelo nome da interface pública da sua VPS se for diferente).",
                                )
                                .color(MUTED)
                                .size(11.0),
                            );
                        });
                    });

                ui.add_space(6.0);
                ui.separator();
                ui.add_space(6.0);

                ui.horizontal(|ui| {
                    if secondary_button(
                        ui,
                        RichText::new("Testar Ping VPS").color(TEXT).size(12.0),
                        Some(Vec2::new(130.0, 32.0)),
                    )
                    .clicked()
                    {
                        let _ = self.ui_tx.send(UiCmd::TestVpsPing);
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if primary_button(
                            ui,
                            "Salvar no perfil",
                            Some(Vec2::new(140.0, 32.0)),
                            12.5,
                        )
                        .clicked()
                        {
                            self.commit_wireguard_config();
                        }

                        if self.wireguard_cfg != self.app_config.wireguard {
                            egui::Frame::new()
                                .fill(ACCENT_SOFT)
                                .stroke(Stroke::new(1.0, ACCENT.gamma_multiply(0.6)))
                                .corner_radius(6.0)
                                .inner_margin(egui::Margin::symmetric(8, 4))
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new("Alterações não salvas")
                                            .color(ACCENT)
                                            .size(11.0)
                                            .strong(),
                                    );
                                });
                        }
                    });
                });
            },
        );

        if should_close {
            self.active_modal = None;
            self.show_wg_private_key = false;
        }
    }

    fn submit_new_game_input(&mut self) {
        let name = self.new_game_input.trim().to_string();
        if !name.is_empty() && self.app_config.add_game_name(&name) {
            self.push_log(format!("Jogo adicionado: {name}"));
            self.save_app_config();
            self.new_game_input.clear();
            let _ = self.ui_tx.send(UiCmd::Scan);
        }
    }

    fn draw_process_row_actions(&mut self, ui: &mut egui::Ui, proc: &RunningProcessInfo) {
        if proc.is_game {
            egui::Frame::new()
                .fill(Color32::from_rgba_unmultiplied(16, 185, 129, 28))
                .stroke(Stroke::new(1.0, OK.gamma_multiply(0.45)))
                .corner_radius(5.0)
                .inner_margin(egui::Margin::symmetric(8, 3))
                .show(ui, |ui| {
                    ui.label(RichText::new("Jogo ativo").color(OK).size(11.0).strong());
                });

            if danger_button(ui, "Remover", None, 11.0)
                .on_hover_text("Remover jogo do monitoramento")
                .clicked()
            {
                self.remove_process_as_game(proc);
            }
        } else if primary_button(ui, "Monitorar Jogo", None, 11.0)
            .on_hover_text("Monitora o processo e sua raiz como parte do jogo")
            .clicked()
        {
            self.add_process_as_game(proc);
        }

        #[cfg(windows)]
        if !proc.is_game {
            let is_reclaim = self
                .app_config
                .reclaim_names
                .contains(&proc.normalized_name);
            if is_reclaim {
                if secondary_button(ui, RichText::new("Trim ✓").color(OK).size(11.0), None)
                    .on_hover_text("Clique para remover do Trim")
                    .clicked()
                {
                    self.app_config.remove_reclaim_name(&proc.normalized_name);
                    self.save_app_config();
                    self.refresh_running_processes();
                }
            } else if secondary_button(ui, RichText::new("Trim").color(MUTED).size(11.0), None)
                .on_hover_text("Adicionar ao trim de memória de segundo plano")
                .clicked()
            {
                self.app_config.add_reclaim_name(&proc.normalized_name);
                self.save_app_config();
                self.refresh_running_processes();
            }

            let is_protected = self
                .app_config
                .never_trim_names
                .contains(&proc.normalized_name);
            if is_protected {
                if secondary_button(ui, RichText::new("Protegido ✓").color(OK).size(11.0), None)
                    .on_hover_text("Clique para desproteger")
                    .clicked()
                {
                    self.app_config
                        .remove_never_trim_name(&proc.normalized_name);
                    self.save_app_config();
                    self.refresh_running_processes();
                }
            } else if secondary_button(ui, RichText::new("Proteger").color(MUTED).size(11.0), None)
                .on_hover_text("Proteger este processo contra trim")
                .clicked()
            {
                self.app_config.add_never_trim_name(&proc.normalized_name);
                self.save_app_config();
                self.refresh_running_processes();
            }
        }
    }

    fn draw_process_modal(&mut self, ctx: &egui::Context) {
        let modal_title = if cfg!(windows) {
            "Gerenciador de Processos, Jogos & Trim"
        } else {
            "Gerenciador de Processos & Jogos"
        };
        let (should_close, ()) = show_centered_modal(
            ctx,
            "process_modal",
            modal_title,
            Some("Escolha processos abertos para monitorar ou gerencie jogos cadastrados"),
            Vec2::new(780.0, 580.0),
            false,
            true,
            |ui| {
                ui.horizontal_wrapped(|ui| {
                    let tab_btn = |ui: &mut egui::Ui, active: bool, label: &str| {
                        styled_button(
                            ui,
                            RichText::new(label)
                                .color(if active { ACCENT } else { TEXT })
                                .strong()
                                .size(12.5),
                            None,
                            if active { ACCENT_SOFT } else { BG_RAISED },
                            if active {
                                Color32::from_rgb(78, 56, 24)
                            } else {
                                BG_ROW
                            },
                            ACCENT_SOFT,
                            Stroke::new(1.0, if active { ACCENT } else { STROKE }),
                            7,
                        )
                    };

                    if tab_btn(
                        ui,
                        self.process_tab == ProcessTab::RunningProcesses,
                        "Processos em Execução",
                    )
                    .clicked()
                    {
                        self.process_tab = ProcessTab::RunningProcesses;
                        self.refresh_running_processes();
                    }
                    if tab_btn(
                        ui,
                        self.process_tab == ProcessTab::Games,
                        "Jogos Cadastrados",
                    )
                    .clicked()
                    {
                        self.process_tab = ProcessTab::Games;
                    }
                    if cfg!(windows)
                        && tab_btn(
                            ui,
                            self.process_tab == ProcessTab::MemoryTrim,
                            "Memória & Regras de Trim",
                        )
                        .clicked()
                    {
                        self.process_tab = ProcessTab::MemoryTrim;
                    }
                });
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(6.0);

                match self.process_tab {
                    ProcessTab::RunningProcesses => {
                        ui.label(
                            RichText::new(
                                "Escolha diretamente qualquer processo aberto no seu sistema. O Game Optimizer detecta automaticamente o processo e a raiz para otimização e tunelamento.",
                            )
                            .color(MUTED)
                            .size(12.0),
                        );
                        ui.add_space(8.0);

                        ui.horizontal_wrapped(|ui| {
                            let filter_w = (ui.available_width() - 250.0).clamp(160.0, 320.0);
                            ui.add(
                                egui::TextEdit::singleline(&mut self.process_search_filter)
                                    .hint_text("Filtrar por nome ou PID...")
                                    .desired_width(filter_w),
                            );
                            if secondary_button(
                                ui,
                                RichText::new("Atualizar Lista").color(TEXT).size(12.0),
                                None,
                            )
                            .on_hover_text("Atualiza a lista de processos ativos")
                            .clicked()
                            {
                                self.refresh_running_processes();
                            }

                            ui.label(
                                RichText::new(format!(
                                    "{} processos ativos",
                                    self.running_processes.len()
                                ))
                                .color(MUTED)
                                .size(11.5),
                            );
                        });

                        ui.add_space(6.0);

                        let filter = self.process_search_filter.trim().to_lowercase();
                        let filtered_procs: Vec<RunningProcessInfo> = self
                            .running_processes
                            .iter()
                            .filter(|p| {
                                if filter.is_empty() {
                                    true
                                } else {
                                    p.normalized_name.contains(&filter)
                                        || p.name.to_lowercase().contains(&filter)
                                        || p.root_name.contains(&filter)
                                        || p.pid.to_string().contains(&filter)
                                        || p.root_pid.to_string().contains(&filter)
                                }
                            })
                            .cloned()
                            .collect();

                        if filtered_procs.is_empty() {
                            ui.add_space(24.0);
                            ui.vertical_centered(|ui| {
                                ui.label(
                                    RichText::new("Nenhum processo encontrado com este filtro.")
                                        .color(MUTED),
                                );
                            });
                        } else {
                            let list_h = ui.available_height().max(140.0);
                            let list = egui::ScrollArea::vertical()
                                .id_salt(SCROLL_AREA_IDS[6])
                                .max_height(list_h)
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    let row_w = (ui.available_width() - 6.0).max(220.0);
                                    for proc in &filtered_procs {
                                        egui::Frame::new()
                                            .fill(BG_ROW)
                                            .stroke(Stroke::new(1.0, STROKE))
                                            .corner_radius(7.0)
                                            .inner_margin(egui::Margin::symmetric(10, 8))
                                            .show(ui, |ui| {
                                                let inner_w = (row_w - 20.0).max(180.0);
                                                ui.set_width(inner_w);
                                                ui.scope(|ui| {
                                                    ui.spacing_mut().button_padding =
                                                        Vec2::new(8.0, 4.0);
                                                    let right_reserve = if cfg!(windows) {
                                                        350.0
                                                    } else {
                                                        205.0
                                                    };
                                                    if inner_w >= 540.0 {
                                                        let left_w =
                                                            (inner_w - right_reserve).max(120.0);
                                                        ui.horizontal(|ui| {
                                                            ui.allocate_ui_with_layout(
                                                                Vec2::new(left_w, 0.0),
                                                                egui::Layout::top_down(
                                                                    egui::Align::Min,
                                                                ),
                                                                |ui| {
                                                                    ui.set_max_width(left_w);
                                                                    ui.horizontal(|ui| {
                                                                        status_dot(
                                                                            ui,
                                                                            if proc.is_game {
                                                                                OK
                                                                            } else {
                                                                                MUTED
                                                                            },
                                                                            3.5,
                                                                        );
                                                                        ui.add(
                                                                            egui::Label::new(
                                                                                RichText::new(
                                                                                    &proc.name,
                                                                                )
                                                                                .color(TEXT)
                                                                                .strong()
                                                                                .size(13.0),
                                                                            )
                                                                            .truncate(),
                                                                        );
                                                                        ui.label(
                                                                            RichText::new(format!(
                                                                                "PID {}",
                                                                                proc.pid
                                                                            ))
                                                                            .color(MUTED)
                                                                            .size(11.0)
                                                                            .monospace(),
                                                                        );
                                                                    });

                                                                    if proc.root_pid != proc.pid
                                                                        || proc.root_name
                                                                            != proc.normalized_name
                                                                    {
                                                                        ui.add(
                                                                            egui::Label::new(
                                                                                RichText::new(format!(
                                                                                    "↳ Raiz detectada: {} (PID {})",
                                                                                    display_exec_name(&proc.root_name),
                                                                                    proc.root_pid
                                                                                ))
                                                                                .color(ACCENT)
                                                                                .size(11.0),
                                                                            )
                                                                            .truncate(),
                                                                        );
                                                                    }
                                                                },
                                                            );

                                                            ui.with_layout(
                                                                egui::Layout::right_to_left(
                                                                    egui::Align::Center,
                                                                ),
                                                                |ui| {
                                                                    ui.label(
                                                                        RichText::new(format_mib(
                                                                            proc.memory_bytes,
                                                                        ))
                                                                        .color(ACCENT)
                                                                        .strong()
                                                                        .size(12.0),
                                                                    );
                                                                    ui.add_space(6.0);
                                                                    self.draw_process_row_actions(
                                                                        ui, proc,
                                                                    );
                                                                },
                                                            );
                                                        });
                                                    } else {
                                                        ui.horizontal(|ui| {
                                                            status_dot(
                                                                ui,
                                                                if proc.is_game { OK } else { MUTED },
                                                                3.5,
                                                            );
                                                            ui.add(
                                                                egui::Label::new(
                                                                    RichText::new(&proc.name)
                                                                        .color(TEXT)
                                                                        .strong()
                                                                        .size(13.0),
                                                                )
                                                                .truncate(),
                                                            );
                                                            ui.with_layout(
                                                                egui::Layout::right_to_left(
                                                                    egui::Align::Center,
                                                                ),
                                                                |ui| {
                                                                    ui.label(
                                                                        RichText::new(format_mib(
                                                                            proc.memory_bytes,
                                                                        ))
                                                                        .color(ACCENT)
                                                                        .strong()
                                                                        .size(11.5),
                                                                    );
                                                                    ui.label(
                                                                        RichText::new(format!(
                                                                            "PID {}",
                                                                            proc.pid
                                                                        ))
                                                                        .color(MUTED)
                                                                        .size(11.0)
                                                                        .monospace(),
                                                                    );
                                                                },
                                                            );
                                                        });
                                                        if proc.root_pid != proc.pid
                                                            || proc.root_name
                                                                != proc.normalized_name
                                                        {
                                                            ui.add(
                                                                egui::Label::new(
                                                                    RichText::new(format!(
                                                                        "↳ Raiz detectada: {} (PID {})",
                                                                        display_exec_name(&proc.root_name),
                                                                        proc.root_pid
                                                                    ))
                                                                    .color(ACCENT)
                                                                    .size(11.0),
                                                                )
                                                                .truncate(),
                                                            );
                                                        }
                                                        ui.add_space(4.0);
                                                        ui.horizontal_wrapped(|ui| {
                                                            self.draw_process_row_actions(ui, proc);
                                                        });
                                                    }
                                                });
                                            });
                                        ui.add_space(4.0);
                                    }
                                });
                            contain_list_scroll(ui, list.inner_rect);
                        }
                    }

                    ProcessTab::Games => {
                        ui.label(
                            RichText::new(if cfg!(target_os = "linux") {
                                "Jogos detectados usam o túnel WireGuard quando ele está ativo."
                            } else {
                                "Processos identificados como jogos recebem prioridade alta, P-Cores exclusivos e tunelamento WireGuard automático."
                            })
                            .color(MUTED)
                            .size(12.0),
                        );
                        ui.add_space(8.0);

                        ui.horizontal_wrapped(|ui| {
                            if primary_button(ui, "Escolher dos Processos", None, 12.0)
                                .on_hover_text(
                                    "Escolha diretamente da lista de processos abertos no sistema",
                                )
                                .clicked()
                            {
                                self.process_tab = ProcessTab::RunningProcesses;
                                self.refresh_running_processes();
                            }

                            let pick_label = if cfg!(windows) {
                                "Procurar .exe"
                            } else {
                                "Procurar executável"
                            };
                            if secondary_button(
                                ui,
                                RichText::new(pick_label).strong().color(TEXT).size(12.0),
                                None,
                            )
                            .on_hover_text("Selecione o executável do jogo no sistema")
                            .clicked()
                            {
                                self.pick_game_exe();
                            }

                            ui.label(RichText::new("ou").color(MUTED).size(12.0));

                            let input_resp = ui.add(
                                egui::TextEdit::singleline(&mut self.new_game_input)
                                    .hint_text("Nome (ex: mir4)")
                                    .desired_width(150.0),
                            );
                            let enter_pressed = input_resp.lost_focus()
                                && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            if enter_pressed
                                || secondary_button(
                                    ui,
                                    RichText::new("+ Adicionar").color(TEXT).size(12.0),
                                    None,
                                )
                                .clicked()
                            {
                                self.submit_new_game_input();
                            }
                        });

                        ui.add_space(10.0);

                        let body_h = ui.available_height().max(120.0);
                        let games_scroll = egui::ScrollArea::vertical()
                            .id_salt(SCROLL_AREA_IDS[7])
                            .max_height(body_h)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                section_label(ui, "JOGOS PERSONALIZADOS DO USUÁRIO");

                                let custom_games = self.app_config.user_custom_games();
                                if custom_games.is_empty() {
                                    ui.label(
                                        RichText::new(
                                            "Nenhum jogo personalizado cadastrado ainda. Use a aba 'Processos em Execução' acima para selecionar jogos abertos.",
                                        )
                                        .color(MUTED)
                                        .size(11.5),
                                    );
                                } else {
                                    for game in &custom_games {
                                        egui::Frame::new()
                                            .fill(BG_ROW)
                                            .stroke(Stroke::new(1.0, STROKE))
                                            .corner_radius(6.0)
                                            .inner_margin(egui::Margin::symmetric(10, 6))
                                            .show(ui, |ui| {
                                                ui.horizontal(|ui| {
                                                    status_dot(ui, OK, 3.5);
                                                    ui.label(
                                                        RichText::new(display_exec_name(game))
                                                            .color(TEXT)
                                                            .strong()
                                                            .monospace(),
                                                    );
                                                    ui.with_layout(
                                                        egui::Layout::right_to_left(
                                                            egui::Align::Center,
                                                        ),
                                                        |ui| {
                                                            if danger_button(
                                                                ui,
                                                                "Remover",
                                                                None,
                                                                11.0,
                                                            )
                                                            .on_hover_text("Remover jogo da lista")
                                                            .clicked()
                                                            {
                                                                self.app_config
                                                                    .remove_game_name(game);
                                                                self.push_log(format!(
                                                                    "Jogo removido: {game}"
                                                                ));
                                                                self.save_app_config();
                                                                let _ =
                                                                    self.ui_tx.send(UiCmd::Scan);
                                                            }
                                                        },
                                                    );
                                                });
                                            });
                                        ui.add_space(4.0);
                                    }
                                }

                                ui.add_space(10.0);
                                ui.collapsing("Lista de Jogos Nativos Pré-configurados", |ui| {
                                    ui.label(
                                        RichText::new(
                                            "O Game Optimizer reconhece nativamente os processos abaixo sem precisar cadastrá-los:",
                                        )
                                        .color(MUTED)
                                        .size(11.0),
                                    );
                                    ui.add_space(4.0);
                                    let builtin_scroll = egui::ScrollArea::vertical()
                                        .id_salt(SCROLL_AREA_IDS[8])
                                        .max_height(130.0)
                                        .show(ui, |ui| {
                                            ui.horizontal_wrapped(|ui| {
                                                for name in
                                                    crate::config::GameCatalog::builtin_names()
                                                {
                                                    ui.label(
                                                        RichText::new(display_exec_name(&name))
                                                            .color(MUTED)
                                                            .size(11.0)
                                                            .monospace(),
                                                    );
                                                }
                                            });
                                        });
                                    contain_list_scroll(ui, builtin_scroll.inner_rect);
                                });
                            });
                        contain_list_scroll(ui, games_scroll.inner_rect);
                    }

                    ProcessTab::MemoryTrim => {
                        let trim_h = ui.available_height().max(140.0);
                        let trim_scroll = egui::ScrollArea::vertical()
                            .id_salt(SCROLL_AREA_IDS[5])
                            .max_height(trim_h)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.label(
                                    RichText::new(
                                        "Configure as regras de otimização de memória RAM para jogos e aplicativos em segundo plano.",
                                    )
                                    .color(MUTED)
                                    .size(12.0),
                                );
                                ui.add_space(8.0);

                                let mut changed = false;
                                if ui
                                    .checkbox(
                                        &mut self.trim_game_memory,
                                        RichText::new(
                                            "Trim de memória nos Jogos (libera páginas frias do próprio jogo)",
                                        )
                                        .color(TEXT)
                                        .size(12.0),
                                    )
                                    .changed()
                                {
                                    self.app_config.trim_game_memory = self.trim_game_memory;
                                    changed = true;
                                }

                                if ui
                                    .checkbox(
                                        &mut self.app_config.trim_non_game_memory,
                                        RichText::new(
                                            "Trim de memória em Apps de Segundo Plano (Browsers e apps pesados)",
                                        )
                                        .color(TEXT)
                                        .size(12.0),
                                    )
                                    .changed()
                                {
                                    changed = true;
                                }

                                ui.add_space(6.0);
                                ui.horizontal_wrapped(|ui| {
                                    let mut mb = (self.app_config.memory_trim_threshold_bytes
                                        / (1024 * 1024))
                                        as u32;
                                    ui.label(
                                        RichText::new("Consumo mínimo para liberar:")
                                            .color(MUTED)
                                            .size(12.0),
                                    );
                                    if ui
                                        .add(egui::Slider::new(&mut mb, 100..=1024).suffix(" MB"))
                                        .changed()
                                    {
                                        self.app_config.memory_trim_threshold_bytes =
                                            (mb as u64) * 1024 * 1024;
                                        changed = true;
                                    }
                                });

                                if changed {
                                    self.save_app_config();
                                }

                                ui.add_space(10.0);
                                section_label(ui, "APLICATIVOS ALVO DE TRIM (SEGUNDO PLANO)");
                                ui.label(
                                    RichText::new(
                                        "Apenas estes programas terão memória liberada quando o trim de 2º plano estiver ativo:",
                                    )
                                    .color(MUTED)
                                    .size(11.0),
                                );

                                let reclaim_list: Vec<String> =
                                    self.app_config.reclaim_names.iter().cloned().collect();
                                let reclaim_scroll = egui::ScrollArea::vertical()
                                    .id_salt(SCROLL_AREA_IDS[9])
                                    .max_height(96.0)
                                    .show(ui, |ui| {
                                        ui.horizontal_wrapped(|ui| {
                                            for app in &reclaim_list {
                                                egui::Frame::new()
                                                    .fill(BG_ROW)
                                                    .corner_radius(6.0)
                                                    .inner_margin(egui::Margin::symmetric(8, 3))
                                                    .show(ui, |ui| {
                                                        ui.horizontal(|ui| {
                                                            ui.label(
                                                                RichText::new(app)
                                                                    .color(TEXT)
                                                                    .size(11.0),
                                                            );
                                                            if ui.small_button("✕").clicked() {
                                                                self.app_config
                                                                    .remove_reclaim_name(app);
                                                                self.save_app_config();
                                                            }
                                                        });
                                                    });
                                            }
                                        });
                                    });
                                contain_list_scroll(ui, reclaim_scroll.inner_rect);

                                ui.add_space(4.0);
                                ui.horizontal_wrapped(|ui| {
                                    let resp = ui.add(
                                        egui::TextEdit::singleline(&mut self.new_reclaim_input)
                                            .hint_text("Nome do app (ex: opera)")
                                            .desired_width(170.0),
                                    );
                                    let enter = resp.lost_focus()
                                        && ui.input(|i| i.key_pressed(egui::Key::Enter));
                                    if enter
                                        || secondary_button(
                                            ui,
                                            RichText::new("+ Adicionar para Trim")
                                                .color(TEXT)
                                                .size(11.5),
                                            None,
                                        )
                                        .clicked()
                                    {
                                        let name = self.new_reclaim_input.trim().to_string();
                                        if !name.is_empty() {
                                            self.app_config.add_reclaim_name(&name);
                                            self.save_app_config();
                                            self.new_reclaim_input.clear();
                                        }
                                    }
                                });

                                ui.add_space(12.0);
                                section_label(ui, "LISTA DE PROTEÇÃO (NUNCA SOFREM TRIM)");
                                ui.label(
                                    RichText::new(
                                        "Estes processos NUNCA sofrerão esvaziamento de memória (Discord, OBS, gravadores, IDEs):",
                                    )
                                    .color(MUTED)
                                    .size(11.0),
                                );

                                let never_trim_list: Vec<String> =
                                    self.app_config.never_trim_names.iter().cloned().collect();
                                let protected_scroll = egui::ScrollArea::vertical()
                                    .id_salt(SCROLL_AREA_IDS[10])
                                    .max_height(96.0)
                                    .show(ui, |ui| {
                                        ui.horizontal_wrapped(|ui| {
                                            for app in &never_trim_list {
                                                egui::Frame::new()
                                                    .fill(BG_ROW)
                                                    .corner_radius(6.0)
                                                    .inner_margin(egui::Margin::symmetric(8, 3))
                                                    .show(ui, |ui| {
                                                        ui.horizontal(|ui| {
                                                            ui.label(
                                                                RichText::new(app)
                                                                    .color(OK)
                                                                    .size(11.0),
                                                            );
                                                            if ui.small_button("✕").clicked() {
                                                                self.app_config
                                                                    .remove_never_trim_name(app);
                                                                self.save_app_config();
                                                            }
                                                        });
                                                    });
                                            }
                                        });
                                    });
                                contain_list_scroll(ui, protected_scroll.inner_rect);

                                ui.add_space(4.0);
                                ui.horizontal_wrapped(|ui| {
                                    let resp = ui.add(
                                        egui::TextEdit::singleline(&mut self.new_never_trim_input)
                                            .hint_text("Nome protegido (ex: obs64)")
                                            .desired_width(170.0),
                                    );
                                    let enter = resp.lost_focus()
                                        && ui.input(|i| i.key_pressed(egui::Key::Enter));
                                    if enter
                                        || secondary_button(
                                            ui,
                                            RichText::new("+ Proteger App")
                                                .color(TEXT)
                                                .size(11.5),
                                            None,
                                        )
                                        .clicked()
                                    {
                                        let name = self.new_never_trim_input.trim().to_string();
                                        if !name.is_empty() {
                                            self.app_config.add_never_trim_name(&name);
                                            self.save_app_config();
                                            self.new_never_trim_input.clear();
                                        }
                                    }
                                });
                            });
                        contain_list_scroll(ui, trim_scroll.inner_rect);
                    }
                }
            },
        );

        if should_close {
            self.active_modal = None;
        }
    }

    fn open_macro_modal_for_game(&mut self, pid: u32, name: &str, normalized_name: &str) {
        self.macro_target_pid = Some(pid);
        self.macro_target_name = name.to_string();
        self.macro_target_normalized = normalized_name.to_string();
        self.capturing_macro_key_for = None;
        self.recording_macro_sequence = false;
        self.confirm_clear_macros = false;
        self.active_modal = Some(ActiveModal::Macro);
    }

    fn instance_profile_key(
        &self,
        normalized_name: &str,
        window_title: Option<&str>,
        pid: u32,
    ) -> String {
        let candidate = macro_profile_key(normalized_name, window_title, pid);
        if !candidate.contains("#pid:") {
            for st in &self.macro_statuses {
                if st.pid == pid {
                    break;
                }
                if macro_profile_key(&st.normalized_name, st.window_title.as_deref(), st.pid)
                    == candidate
                {
                    return format!("{candidate}#pid:{pid}");
                }
            }
        }
        candidate
    }

    fn set_game_macro_active(&mut self, pid: u32, normalized_name: &str, active: bool) {
        if let Some(inst) = self.macro_statuses.iter_mut().find(|s| s.pid == pid) {
            inst.active = active;
        }
        let _ = self.ui_tx.send(UiCmd::SetMacroActive {
            pid,
            normalized_name: normalized_name.to_string(),
            active,
        });
    }

    fn update_game_macro_rules(
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
        let win_title = self
            .macro_statuses
            .iter()
            .find(|s| s.pid == pid)
            .and_then(|s| s.window_title.clone());
        let profile_key = self.instance_profile_key(normalized_name, win_title.as_deref(), pid);
        let entry = self.app_config.macros.entry(profile_key).or_default();
        entry.mode = mode;
        entry.cycle_interval_ms = clean_cycle;
        entry.toggle_key = toggle_key.clone();
        entry.rules = clean.clone();
        if let Some(inst) = self.macro_statuses.iter_mut().find(|s| s.pid == pid) {
            inst.mode = mode;
            inst.cycle_interval_ms = clean_cycle;
            inst.toggle_key = toggle_key.clone();
            inst.rules = clean.clone();
            inst.rule_triggers.resize(clean.len(), 0);
        }
        let _ = self.ui_tx.send(UiCmd::SetMacroRules {
            pid,
            normalized_name: normalized_name.to_string(),
            mode,
            cycle_interval_ms: clean_cycle,
            toggle_key,
            rules: clean,
        });
        self.save_app_config();
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_macro_steps_list(
        &mut self,
        ui: &mut egui::Ui,
        target_pid: u32,
        norm_name: &str,
        profile: &GameMacroProfile,
        is_active: bool,
        waiting_for_next_cycle: bool,
        current_step_idx: Option<usize>,
        rule_triggers: &[u64],
    ) {
        let mut rules_mut = profile.rules.clone();
        let total_rules = rules_mut.len();
        let mut rules_changed = false;
        let mut remove_idx: Option<usize> = None;
        let mut swap_pair: Option<(usize, usize)> = None;
        let row_w = (ui.available_width() - 6.0).max(240.0);

        for (idx, rule) in rules_mut.iter_mut().enumerate() {
            let trig = rule_triggers.get(idx).copied().unwrap_or(0);
            let is_current_step = is_active
                && profile.mode == MacroExecutionMode::Sequential
                && current_step_idx == Some(idx);

            let border_stroke = if is_current_step {
                Stroke::new(1.6, OK)
            } else if rule.enabled && is_active {
                Stroke::new(1.0, OK.gamma_multiply(0.45))
            } else {
                Stroke::new(1.0, STROKE)
            };

            egui::Frame::new()
                .fill(BG_ROW)
                .stroke(border_stroke)
                .corner_radius(8.0)
                .inner_margin(egui::Margin::symmetric(12, 9))
                .show(ui, |ui| {
                    ui.set_width((row_w - 24.0).max(220.0));

                    // Row 1: Checkbox + Step badge + Key dropdown + Capture + Right-aligned actions
                    ui.horizontal(|ui| {
                        if ui
                            .checkbox(&mut rule.enabled, "")
                            .on_hover_text("Ativar ou desativar este passo")
                            .changed()
                        {
                            rules_changed = true;
                        }

                        let step_badge = if is_current_step {
                            format!("▶ PASSO {}", idx + 1)
                        } else {
                            format!("PASSO {}", idx + 1)
                        };
                        egui::Frame::new()
                            .fill(if is_current_step {
                                Color32::from_rgb(16, 68, 48)
                            } else {
                                BG_RAISED
                            })
                            .corner_radius(5.0)
                            .inner_margin(egui::Margin::symmetric(7, 3))
                            .show(ui, |ui| {
                                ui.label(
                                    RichText::new(step_badge)
                                        .color(if is_current_step { OK } else { ACCENT })
                                        .size(11.0)
                                        .strong()
                                        .monospace(),
                                );
                            });

                        if key_combo_box(
                            ui,
                            ("rule-key-combo", idx),
                            &mut rule.key,
                            150.0,
                            false,
                            false,
                        ) {
                            rules_changed = true;
                        }

                        let is_cap_row = self.capturing_macro_key_for == Some(Some(idx));
                        if styled_button(
                            ui,
                            RichText::new(if is_cap_row {
                                "Aperte... (Esc)"
                            } else {
                                "Capturar"
                            })
                            .color(if is_cap_row { ACCENT } else { MUTED })
                            .size(11.0),
                            None,
                            if is_cap_row { ACCENT_SOFT } else { BG_RAISED },
                            BG_ROW,
                            ACCENT_SOFT,
                            Stroke::new(1.0, if is_cap_row { ACCENT } else { STROKE }),
                            5,
                        )
                        .on_hover_text(
                            "Clique aqui e aperte uma tecla do teclado para definir neste passo",
                        )
                        .clicked()
                        {
                            self.confirm_clear_macros = false;
                            self.capturing_macro_key_for = if is_cap_row {
                                None
                            } else {
                                Some(Some(idx))
                            };
                        }

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.spacing_mut().button_padding = Vec2::new(7.0, 3.0);
                            if danger_button(ui, "Remover", None, 11.0)
                                .on_hover_text("Remover este passo")
                                .clicked()
                            {
                                remove_idx = Some(idx);
                            }

                            if secondary_button(
                                ui,
                                RichText::new("Testar").color(TEXT).size(11.0),
                                None,
                            )
                            .on_hover_text("Enviar esta tecla 1 vez agora para testar no jogo")
                            .clicked()
                            {
                                let _ = self.ui_tx.send(UiCmd::TestMacroKey {
                                    pid: target_pid,
                                    normalized_name: norm_name.to_string(),
                                    key: rule.key.clone(),
                                });
                            }

                            if total_rules > 1 {
                                if ui
                                    .add_enabled(
                                        idx + 1 < total_rules,
                                        egui::Button::new(RichText::new("↓").size(11.0)).small(),
                                    )
                                    .on_hover_text("Mover passo para baixo")
                                    .clicked()
                                {
                                    swap_pair = Some((idx, idx + 1));
                                }
                                if ui
                                    .add_enabled(
                                        idx > 0,
                                        egui::Button::new(RichText::new("↑").size(11.0)).small(),
                                    )
                                    .on_hover_text("Mover passo para cima")
                                    .clicked()
                                {
                                    swap_pair = Some((idx, idx - 1));
                                }
                            }

                            ui.label(
                                RichText::new(format!("{trig}x"))
                                    .color(if trig > 0 { OK } else { MUTED })
                                    .size(11.0)
                                    .monospace(),
                            )
                            .on_hover_text("Quantas vezes este passo já foi enviado nesta sessão");
                        });
                    });

                    ui.add_space(4.0);

                    // Row 2: Delay / interval control
                    ui.horizontal_wrapped(|ui| {
                        ui.add_space(28.0);
                        let is_last_seq_step = profile.mode == MacroExecutionMode::Sequential
                            && idx + 1 == total_rules;
                        if is_last_seq_step {
                            ui.label(
                                RichText::new("→ Última tecla da sequência")
                                    .color(MUTED)
                                    .size(11.5),
                            );
                        } else {
                            let wait_label = match profile.mode {
                                MacroExecutionMode::Sequential => "→ Próxima tecla em:",
                                MacroExecutionMode::Parallel => "Repetir a cada:",
                            };
                            ui.label(RichText::new(wait_label).color(MUTED).size(11.5));
                            if ui
                                .add(
                                    egui::DragValue::new(&mut rule.interval_ms)
                                        .speed(5.0)
                                        .range(MIN_MACRO_INTERVAL_MS..=MAX_MACRO_INTERVAL_MS)
                                        .suffix(" ms"),
                                )
                                .on_hover_text(
                                    "Tempo de espera (em milissegundos) antes da próxima tecla. Ex: 50 ms executa a próxima tecla 50ms depois desta.",
                                )
                                .changed()
                            {
                                rules_changed = true;
                            }
                            if rule.interval_ms >= 1000 {
                                ui.label(
                                    RichText::new(format!(
                                        "({:.1}s)",
                                        rule.interval_ms as f64 / 1000.0
                                    ))
                                    .color(MUTED)
                                    .size(11.0)
                                    .monospace(),
                                );
                            }
                        }
                    });
                });

            if profile.mode == MacroExecutionMode::Sequential {
                if idx + 1 < total_rules {
                    ui.horizontal(|ui| {
                        ui.add_space(28.0);
                        ui.label(
                            RichText::new(format!("↓  após {} ms executa:", rule.interval_ms))
                                .color(MUTED)
                                .size(10.5),
                        );
                    });
                } else {
                    ui.add_space(6.0);
                    egui::Frame::new()
                        .fill(if waiting_for_next_cycle {
                            Color32::from_rgb(16, 68, 48)
                        } else {
                            BG_RAISED
                        })
                        .stroke(Stroke::new(
                            1.0,
                            if waiting_for_next_cycle { OK } else { STROKE },
                        ))
                        .corner_radius(6.0)
                        .inner_margin(egui::Margin::symmetric(10, 6))
                        .show(ui, |ui| {
                            let cycle_sec = profile.cycle_interval_ms as f64 / 1000.0;
                            let text = if waiting_for_next_cycle {
                                format!(
                                    "▶  Fim da sequência — aguardando {} ms ({cycle_sec:.1}s) para disparar novamente do Passo 1...",
                                    profile.cycle_interval_ms
                                )
                            } else {
                                format!(
                                    "●  Ao terminar este último passo, aguarda {} ms ({cycle_sec:.1}s) e repete a sequência do Passo 1",
                                    profile.cycle_interval_ms
                                )
                            };
                            ui.label(
                                RichText::new(text)
                                    .color(if waiting_for_next_cycle { OK } else { ACCENT })
                                    .size(11.5)
                                    .strong(),
                            );
                        });
                }
            } else {
                ui.add_space(4.0);
            }
        }

        if let Some((a, b)) = swap_pair {
            rules_mut.swap(a, b);
            rules_changed = true;
        }
        if let Some(idx) = remove_idx {
            rules_mut.remove(idx);
            rules_changed = true;
        }
        if rules_changed {
            self.update_game_macro_rules(
                target_pid,
                norm_name,
                profile.mode,
                profile.cycle_interval_ms,
                profile.toggle_key.clone(),
                rules_mut,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_macro_modal_content(
        &mut self,
        ui: &mut egui::Ui,
        target_pid: u32,
        norm_name: &str,
        display_name: &str,
        profile_key: &str,
        mut profile: GameMacroProfile,
        running_games: &[crate::ProcessSnapshot],
        is_running: bool,
        is_active: bool,
        waiting_for_next_cycle: bool,
        current_step_idx: Option<usize>,
        window_title: Option<&str>,
        total_sent: u64,
        rule_triggers: &[u64],
        single_scroll_mode: bool,
    ) {
        if self.recording_macro_sequence || self.capturing_macro_key_for.is_some() {
            let banner_text = if self.recording_macro_sequence {
                "Gravação ativa: pressione as teclas no teclado na ordem desejada ou Esc para parar."
            } else {
                "Aguardando tecla: pressione qualquer tecla do teclado para definir ou Esc para cancelar."
            };
            let btn_label = if self.recording_macro_sequence {
                "Parar gravação"
            } else {
                "Cancelar"
            };
            egui::Frame::new()
                .fill(ACCENT_SOFT)
                .stroke(Stroke::new(1.0, ACCENT))
                .corner_radius(8.0)
                .inner_margin(egui::Margin::symmetric(10, 7))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        status_dot(ui, ACCENT, 4.0);
                        let msg_w = (ui.available_width() - 135.0).max(120.0);
                        ui.allocate_ui_with_layout(
                            Vec2::new(msg_w, 0.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.label(
                                    RichText::new(banner_text).color(ACCENT).strong().size(12.0),
                                );
                            },
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if secondary_button(
                                ui,
                                RichText::new(btn_label).color(TEXT).size(11.5),
                                None,
                            )
                            .clicked()
                            {
                                self.recording_macro_sequence = false;
                                self.capturing_macro_key_for = None;
                            }
                        });
                    });
                });
            ui.add_space(8.0);
        }

        #[cfg(target_os = "linux")]
        {
            ui.label(
                RichText::new("O macro pode continuar em segundo plano. Use o atalho com o jogo em foco para iniciar ou pausar.")
                    .color(MUTED)
                    .size(11.5),
            );
            ui.add_space(6.0);
        }

        // 1. Multi-window switcher if multiple game windows are open
        if running_games.len() > 1 {
            section_label(ui, "JANELA SELECIONADA");
            ui.horizontal_wrapped(|ui| {
                for g in running_games {
                    let selected = g.pid == target_pid;
                    let g_status = self.macro_statuses.iter().find(|s| s.pid == g.pid);
                    let g_active = g_status.is_some_and(|s| s.active);
                    let g_win_title = g_status
                        .and_then(|s| s.window_title.as_deref())
                        .map(str::trim)
                        .filter(|t| !t.is_empty() && !t.eq_ignore_ascii_case(&g.name));
                    let status_mark = if g_active { "●" } else { "○" };
                    let btn_label = match g_win_title {
                        Some(t) => format!("{status_mark} {} • {t} (PID {})", g.name, g.pid),
                        None => format!("{status_mark} {} (PID {})", g.name, g.pid),
                    };
                    if styled_button(
                        ui,
                        RichText::new(btn_label)
                            .color(if g_active {
                                OK
                            } else if selected {
                                ACCENT
                            } else {
                                TEXT
                            })
                            .size(11.5)
                            .strong(),
                        None,
                        if selected { ACCENT_SOFT } else { BG_RAISED },
                        BG_ROW,
                        ACCENT_SOFT,
                        Stroke::new(1.0, if selected { ACCENT } else { STROKE }),
                        6,
                    )
                    .clicked()
                    {
                        self.macro_target_pid = Some(g.pid);
                        self.macro_target_name = g.name.clone();
                        self.macro_target_normalized = g.normalized_name.clone();
                        self.capturing_macro_key_for = None;
                        self.recording_macro_sequence = false;
                        self.confirm_clear_macros = false;
                    }
                }
            });
            ui.add_space(8.0);
        }

        // 2. Top Control Card (Status + Play/Pause Button cleanly separated)
        egui::Frame::new()
            .fill(BG_RAISED)
            .stroke(Stroke::new(1.2, if is_active { OK } else { STROKE }))
            .corner_radius(10.0)
            .inner_margin(12.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (dot_col, status_title) = if !is_running {
                        (MUTED, "PROCESSO ENCERRADO")
                    } else if is_active {
                        (OK, "AUTOMAÇÃO EM EXECUÇÃO NESTA JANELA")
                    } else {
                        (ACCENT, "AUTOMAÇÃO PAUSADA NESTA JANELA")
                    };

                    let left_w = (ui.available_width() - 196.0).max(140.0);
                    ui.allocate_ui_with_layout(
                        Vec2::new(left_w, 0.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_max_width(left_w);
                            ui.horizontal(|ui| {
                                status_dot(ui, dot_col, 5.0);
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(status_title)
                                            .color(dot_col)
                                            .strong()
                                            .size(13.0),
                                    )
                                    .truncate(),
                                );
                            });
                            let win_desc = match window_title {
                                Some(t) => format!(
                                    "Janela: \"{t}\" (PID {target_pid}) • Ações enviadas: {total_sent}"
                                ),
                                None => format!(
                                    "Processo: {display_name} (PID {target_pid}) • Ações enviadas: {total_sent}"
                                ),
                            };
                            ui.add(
                                egui::Label::new(RichText::new(win_desc).color(TEXT).size(12.0))
                                    .truncate(),
                            );
                        },
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let can_start =
                            is_running && profile.rules.iter().any(MacroRule::is_ready);
                        if is_active {
                            if danger_button(
                                ui,
                                "⏸  Pausar Automação",
                                Some(Vec2::new(184.0, 36.0)),
                                12.5,
                            )
                            .clicked()
                            {
                                self.set_game_macro_active(target_pid, norm_name, false);
                            }
                        } else {
                            let btn = ui
                                .add_enabled_ui(can_start, |ui| {
                                    primary_button(
                                        ui,
                                        "▶  Iniciar Automação",
                                        Some(Vec2::new(184.0, 36.0)),
                                        12.5,
                                    )
                                })
                                .inner;
                            if btn
                                .on_disabled_hover_text(
                                    "Adicione pelo menos 1 passo abaixo para iniciar",
                                )
                                .clicked()
                            {
                                self.set_game_macro_active(target_pid, norm_name, true);
                            }
                        }
                    });
                });

                ui.add_space(6.0);
                ui.separator();
                ui.add_space(4.0);

                // Execution Mode selector
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new("Modo de reprodução:")
                            .color(MUTED)
                            .size(12.0)
                            .strong(),
                    );
                    let is_seq = profile.mode == MacroExecutionMode::Sequential;
                    if styled_button(
                        ui,
                        RichText::new("1 → 2 → 3  Em Sequência")
                            .color(if is_seq { ACCENT } else { TEXT })
                            .size(11.5)
                            .strong(),
                        None,
                        if is_seq { ACCENT_SOFT } else { BG_ROW },
                        Color32::from_rgb(45, 52, 66),
                        ACCENT_SOFT,
                        Stroke::new(1.0, if is_seq { ACCENT } else { STROKE }),
                        6,
                    )
                    .on_hover_text(
                        "Executa a sequência de teclas em ordem (ex: 50ms entre elas) e aguarda o tempo de disparo configurado para repetir a sequência",
                    )
                    .clicked()
                        && !is_seq
                    {
                        self.update_game_macro_rules(
                            target_pid,
                            norm_name,
                            MacroExecutionMode::Sequential,
                            profile.cycle_interval_ms,
                            profile.toggle_key.clone(),
                            profile.rules.clone(),
                        );
                    }

                    if styled_button(
                        ui,
                        RichText::new("Simultâneo (Timers independentes)")
                            .color(if !is_seq { ACCENT } else { TEXT })
                            .size(11.5)
                            .strong(),
                        None,
                        if !is_seq { ACCENT_SOFT } else { BG_ROW },
                        Color32::from_rgb(45, 52, 66),
                        ACCENT_SOFT,
                        Stroke::new(1.0, if !is_seq { ACCENT } else { STROKE }),
                        6,
                    )
                    .on_hover_text(
                        "Cada tecla tem seu próprio cronômetro separado e dispara independentemente das outras",
                    )
                    .clicked()
                        && is_seq
                    {
                        self.update_game_macro_rules(
                            target_pid,
                            norm_name,
                            MacroExecutionMode::Parallel,
                            profile.cycle_interval_ms,
                            profile.toggle_key.clone(),
                            profile.rules.clone(),
                        );
                    }
                });

                if profile.mode == MacroExecutionMode::Sequential {
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new("Disparar a sequência a cada:")
                                .color(TEXT)
                                .size(12.0)
                                .strong(),
                        );
                        let mut cycle_ms = profile.cycle_interval_ms;
                        if ui
                            .add(
                                egui::DragValue::new(&mut cycle_ms)
                                    .speed(100.0)
                                    .range(MIN_MACRO_INTERVAL_MS..=MAX_MACRO_INTERVAL_MS)
                                    .suffix(" ms"),
                            )
                            .on_hover_text(
                                "Tempo de espera após terminar a sequência completa antes de disparar o Passo 1 novamente (ex: 5000 ms = 5 segundos)",
                            )
                            .changed()
                        {
                            profile.cycle_interval_ms = cycle_ms;
                            self.update_game_macro_rules(
                                target_pid,
                                norm_name,
                                profile.mode,
                                cycle_ms,
                                profile.toggle_key.clone(),
                                profile.rules.clone(),
                            );
                        }
                        ui.label(
                            RichText::new(format!(
                                "({:.1}s)",
                                profile.cycle_interval_ms as f64 / 1000.0
                            ))
                            .color(ACCENT)
                            .size(11.5)
                            .strong()
                            .monospace(),
                        );

                        for (lbl, ms) in [
                            ("1s", 1_000u64),
                            ("2s", 2_000),
                            ("3s", 3_000),
                            ("5s", 5_000),
                            ("10s", 10_000),
                            ("15s", 15_000),
                            ("30s", 30_000),
                        ] {
                            let active_preset = profile.cycle_interval_ms == ms;
                            if styled_button(
                                ui,
                                RichText::new(lbl)
                                    .color(if active_preset { ACCENT } else { MUTED })
                                    .size(11.0)
                                    .strong(),
                                None,
                                if active_preset { ACCENT_SOFT } else { BG_ROW },
                                Color32::from_rgb(45, 52, 66),
                                ACCENT_SOFT,
                                Stroke::new(1.0, if active_preset { ACCENT } else { STROKE }),
                                5,
                            )
                            .on_hover_text(format!(
                                "Repetir a sequência inteira a cada {lbl} ({ms} ms)"
                            ))
                            .clicked()
                            {
                                profile.cycle_interval_ms = ms;
                                self.update_game_macro_rules(
                                    target_pid,
                                    norm_name,
                                    profile.mode,
                                    ms,
                                    profile.toggle_key.clone(),
                                    profile.rules.clone(),
                                );
                            }
                        }
                    });
                }

                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new("Atalho para Parar / Iniciar no jogo:")
                            .color(TEXT)
                            .size(12.0)
                            .strong(),
                    );
                    if key_combo_box(
                        ui,
                        "macro-window-toggle-key",
                        &mut profile.toggle_key,
                        140.0,
                        true,
                        true,
                    ) {
                        self.update_game_macro_rules(
                            target_pid,
                            norm_name,
                            profile.mode,
                            profile.cycle_interval_ms,
                            profile.toggle_key.clone(),
                            profile.rules.clone(),
                        );
                    }

                    let is_cap_toggle = self.capturing_macro_key_for == Some(None);
                    if styled_button(
                        ui,
                        RichText::new(if is_cap_toggle {
                            "Aperte a tecla... (Esc)"
                        } else {
                            "Capturar"
                        })
                        .color(if is_cap_toggle { ACCENT } else { MUTED })
                        .size(11.0),
                        None,
                        if is_cap_toggle { ACCENT_SOFT } else { BG_ROW },
                        Color32::from_rgb(45, 52, 66),
                        ACCENT_SOFT,
                        Stroke::new(1.0, if is_cap_toggle { ACCENT } else { STROKE }),
                        5,
                    )
                    .on_hover_text(
                        "Clique e aperte a tecla que deseja usar dentro da janela do jogo para ligar/desligar o macro naquela janela",
                    )
                    .clicked()
                    {
                        self.recording_macro_sequence = false;
                        self.confirm_clear_macros = false;
                        self.capturing_macro_key_for =
                            if is_cap_toggle { None } else { Some(None) };
                    }

                    ui.label(
                        RichText::new(if profile.toggle_key.is_empty() {
                            "(Atalho desativado)".to_string()
                        } else {
                            format!(
                                "• Alterna apenas nesta janela ({})",
                                key_display_label(&profile.toggle_key)
                            )
                        })
                        .color(MUTED)
                        .size(11.0),
                    );
                });

                ui.add_space(4.0);
                let mode_hint = match profile.mode {
                    MacroExecutionMode::Sequential => {
                        "Executa os passos em ordem (ex: 50 ms entre teclas) e, após o último, aguarda o intervalo de ciclo acima antes de repetir."
                    }
                    MacroExecutionMode::Parallel => {
                        "Cada tecla da lista roda em seu próprio intervalo de tempo de forma independente."
                    }
                };
                ui.label(RichText::new(mode_hint).color(MUTED).size(11.0));

                ui.add_space(4.0);
                let mut auto_start = profile.auto_start;
                if ui
                    .checkbox(
                        &mut auto_start,
                        RichText::new(
                            "Iniciar automaticamente quando esta janela do jogo abrir (Opcional)",
                        )
                        .color(MUTED)
                        .size(11.5),
                    )
                    .changed()
                {
                    self.app_config
                        .macros
                        .entry(profile_key.to_string())
                        .or_default()
                        .auto_start = auto_start;
                    self.save_app_config();
                }
            });

        ui.add_space(10.0);

        // 3. Toolbar (clean wrapped layout, no separator or RTL inside wrap)
        ui.horizontal_wrapped(|ui| {
            if primary_button(ui, "+ Adicionar Passo", None, 12.0)
                .on_hover_text(
                    "Adiciona um novo passo na sequência e aguarda você pressionar ou selecionar a tecla",
                )
                .clicked()
            {
                self.recording_macro_sequence = false;
                self.confirm_clear_macros = false;
                let mut updated = profile.rules.clone();
                let new_idx = updated.len();
                updated.push(
                    MacroRule {
                        label: String::new(),
                        key: String::new(),
                        interval_ms: self.default_macro_delay_ms,
                        hold_ms: DEFAULT_MACRO_HOLD_MS,
                        enabled: true,
                    }
                    .normalized(),
                );
                self.capturing_macro_key_for = Some(Some(new_idx));
                self.update_game_macro_rules(
                    target_pid,
                    norm_name,
                    profile.mode,
                    profile.cycle_interval_ms,
                    profile.toggle_key.clone(),
                    updated,
                );
            }

            let rec_active = self.recording_macro_sequence;
            if styled_button(
                ui,
                RichText::new(if rec_active {
                    "Parar Gravação (Esc)"
                } else {
                    "● Gravar Sequência"
                })
                .color(if rec_active { ACCENT } else { TEXT })
                .strong()
                .size(12.0),
                None,
                if rec_active { ACCENT_SOFT } else { BG_RAISED },
                BG_ROW,
                ACCENT_SOFT,
                Stroke::new(1.0, if rec_active { ACCENT } else { STROKE }),
                7,
            )
            .on_hover_text(
                "Clique e vá apertando as teclas no teclado na ordem desejada; cada tecla será adicionada com o intervalo ao lado",
            )
            .clicked()
            {
                self.recording_macro_sequence = !rec_active;
                self.capturing_macro_key_for = None;
                self.confirm_clear_macros = false;
            }

            ui.label(
                RichText::new("Tempo entre teclas:")
                    .color(MUTED)
                    .size(11.5),
            );
            ui.add(
                egui::DragValue::new(&mut self.default_macro_delay_ms)
                    .speed(5.0)
                    .range(MIN_MACRO_INTERVAL_MS..=MAX_MACRO_INTERVAL_MS)
                    .suffix(" ms"),
            )
            .on_hover_text(
                "Intervalo padrão entre uma tecla e a próxima da sequência (ex: 50 ms)",
            );

            if !profile.rules.is_empty() {
                if secondary_button(
                    ui,
                    RichText::new(format!(
                        "Aplicar {} ms a todos",
                        self.default_macro_delay_ms
                    ))
                    .color(TEXT)
                    .size(11.0),
                    None,
                )
                .on_hover_text(
                    "Define este intervalo entre teclas em todos os passos da sequência abaixo",
                )
                .clicked()
                {
                    self.confirm_clear_macros = false;
                    let updated: Vec<MacroRule> = profile
                        .rules
                        .iter()
                        .cloned()
                        .map(|mut r| {
                            r.interval_ms = self.default_macro_delay_ms;
                            r.normalized()
                        })
                        .collect();
                    self.update_game_macro_rules(
                        target_pid,
                        norm_name,
                        profile.mode,
                        profile.cycle_interval_ms,
                        profile.toggle_key.clone(),
                        updated,
                    );
                }

                if self.confirm_clear_macros {
                    if danger_button(ui, "Confirmar Limpar", None, 11.0).clicked() {
                        self.recording_macro_sequence = false;
                        self.capturing_macro_key_for = None;
                        self.confirm_clear_macros = false;
                        self.set_game_macro_active(target_pid, norm_name, false);
                        self.update_game_macro_rules(
                            target_pid,
                            norm_name,
                            profile.mode,
                            profile.cycle_interval_ms,
                            profile.toggle_key.clone(),
                            Vec::new(),
                        );
                    }
                    if secondary_button(
                        ui,
                        RichText::new("Cancelar").color(MUTED).size(11.0),
                        None,
                    )
                    .clicked()
                    {
                        self.confirm_clear_macros = false;
                    }
                } else if secondary_button(
                    ui,
                    RichText::new("Limpar Tudo").color(MUTED).size(11.0),
                    None,
                )
                .clicked()
                {
                    self.confirm_clear_macros = true;
                }
            }
        });

        ui.add_space(8.0);

        // 4. Unified Step-by-Step Sequence List
        if profile.rules.is_empty() {
            egui::Frame::new()
                .fill(BG_RAISED)
                .stroke(Stroke::new(1.0, STROKE))
                .corner_radius(8.0)
                .inner_margin(20.0)
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new("Nenhum passo configurado ainda.")
                                .color(TEXT)
                                .strong()
                                .size(13.5),
                        );
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(
                                "Use '+ Adicionar Passo' ou '● Gravar Sequência' acima para definir suas teclas na ordem desejada (padrão: 50 ms entre cada tecla).",
                            )
                            .color(MUTED)
                            .size(12.0),
                        );
                    });
                });
        } else if single_scroll_mode {
            self.draw_macro_steps_list(
                ui,
                target_pid,
                norm_name,
                &profile,
                is_active,
                waiting_for_next_cycle,
                current_step_idx,
                rule_triggers,
            );
        } else {
            let list_h = ui.available_height().max(150.0);
            let list = egui::ScrollArea::vertical()
                .id_salt(SCROLL_AREA_IDS[12])
                .max_height(list_h)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    self.draw_macro_steps_list(
                        ui,
                        target_pid,
                        norm_name,
                        &profile,
                        is_active,
                        waiting_for_next_cycle,
                        current_step_idx,
                        rule_triggers,
                    );
                });
            contain_list_scroll(ui, list.inner_rect);
        }
    }

    fn draw_macro_modal(&mut self, ctx: &egui::Context) {
        let Some(target_pid) = self.macro_target_pid else {
            self.active_modal = None;
            return;
        };

        // Keep target metadata synced if the user switches between running game windows
        let running_games = self
            .report
            .as_ref()
            .map(|r| r.games.clone())
            .unwrap_or_default();
        if let Some(g) = running_games.iter().find(|g| g.pid == target_pid) {
            self.macro_target_name = g.name.clone();
            self.macro_target_normalized = g.normalized_name.clone();
        }

        let norm_name = self.macro_target_normalized.clone();
        let display_name = if self.macro_target_name.is_empty() {
            norm_name.clone()
        } else {
            self.macro_target_name.clone()
        };

        let inst_status = self
            .macro_statuses
            .iter()
            .find(|s| s.pid == target_pid)
            .cloned();
        let is_running = running_games.iter().any(|g| g.pid == target_pid);
        let is_active = inst_status.as_ref().is_some_and(|s| s.active);
        let waiting_for_next_cycle = inst_status
            .as_ref()
            .is_some_and(|s| s.waiting_for_next_cycle);
        let current_step_idx = inst_status.as_ref().and_then(|s| s.current_step_index);
        let window_title = inst_status.as_ref().and_then(|s| s.window_title.clone());
        let total_sent = inst_status.as_ref().map(|s| s.total_triggers).unwrap_or(0);
        let rule_triggers = inst_status
            .as_ref()
            .map(|s| s.rule_triggers.clone())
            .unwrap_or_default();

        let profile_key =
            self.instance_profile_key(&norm_name, window_title.as_deref(), target_pid);
        let mut profile: GameMacroProfile = self
            .app_config
            .macros
            .get(&profile_key)
            .cloned()
            .unwrap_or_default()
            .normalized();
        if let Some(st) = &inst_status {
            profile.mode = st.mode;
            profile.cycle_interval_ms = if st.cycle_interval_ms > 0 {
                st.cycle_interval_ms
            } else {
                DEFAULT_CYCLE_INTERVAL_MS
            };
            profile.toggle_key = st.toggle_key.clone();
            profile.rules = st.rules.clone();
        }

        // Handle continuous sequence recording or single-step / toggle-key capture
        let mut escape_consumed = false;
        if self.recording_macro_sequence {
            let mut captured_codes: Vec<&'static str> = Vec::new();
            let mut stop_recording = false;
            ctx.input_mut(|input| {
                if input.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                    stop_recording = true;
                    return;
                }
                for event in &input.events {
                    if let egui::Event::Key {
                        key,
                        pressed: true,
                        repeat: false,
                        ..
                    } = event
                    {
                        if let Some(code) = egui_key_to_code(*key) {
                            captured_codes.push(code);
                        }
                    }
                }
            });
            if stop_recording {
                self.recording_macro_sequence = false;
                escape_consumed = true;
            } else if !captured_codes.is_empty() {
                for code in captured_codes {
                    profile.rules.push(
                        MacroRule {
                            label: String::new(),
                            key: code.to_string(),
                            interval_ms: self.default_macro_delay_ms,
                            hold_ms: DEFAULT_MACRO_HOLD_MS,
                            enabled: true,
                        }
                        .normalized(),
                    );
                }
                self.update_game_macro_rules(
                    target_pid,
                    &norm_name,
                    profile.mode,
                    profile.cycle_interval_ms,
                    profile.toggle_key.clone(),
                    profile.rules.clone(),
                );
            }
        } else if let Some(target_slot) = self.capturing_macro_key_for {
            let mut captured_code: Option<&'static str> = None;
            let mut cancel_capture = false;
            ctx.input_mut(|input| {
                if input.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                    cancel_capture = true;
                    return;
                }
                for event in &input.events {
                    if let egui::Event::Key {
                        key, pressed: true, ..
                    } = event
                    {
                        if let Some(code) = egui_key_to_code(*key) {
                            captured_code = Some(code);
                            break;
                        }
                    }
                }
            });
            if cancel_capture {
                self.capturing_macro_key_for = None;
                escape_consumed = true;
            } else if let Some(code) = captured_code {
                self.capturing_macro_key_for = None;
                match target_slot {
                    None => {
                        profile.toggle_key = code.to_string();
                        self.update_game_macro_rules(
                            target_pid,
                            &norm_name,
                            profile.mode,
                            profile.cycle_interval_ms,
                            profile.toggle_key.clone(),
                            profile.rules.clone(),
                        );
                    }
                    Some(rule_idx) => {
                        if let Some(rule) = profile.rules.get_mut(rule_idx) {
                            rule.key = code.to_string();
                            self.update_game_macro_rules(
                                target_pid,
                                &norm_name,
                                profile.mode,
                                profile.cycle_interval_ms,
                                profile.toggle_key.clone(),
                                profile.rules.clone(),
                            );
                        }
                    }
                }
            }
        }

        let subtitle = match &window_title {
            Some(t) if !t.trim().is_empty() && !t.eq_ignore_ascii_case(&display_name) => {
                format!("{display_name} • \"{t}\" (PID {target_pid})")
            }
            _ => format!("{display_name} (PID {target_pid})"),
        };
        let allow_escape_close = !escape_consumed
            && !self.recording_macro_sequence
            && self.capturing_macro_key_for.is_none();

        let (should_close, ()) = show_centered_modal(
            ctx,
            "macro_modal",
            "Automação de Teclas (Macros)",
            Some(&subtitle),
            Vec2::new(820.0, 620.0),
            true,
            allow_escape_close,
            |ui| {
                let body_h = ui.available_height();
                let single_scroll = body_h < 470.0 || running_games.len() > 3;
                if single_scroll {
                    egui::ScrollArea::vertical()
                        .id_salt(SCROLL_AREA_IDS[11])
                        .max_height(body_h.max(120.0))
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            self.draw_macro_modal_content(
                                ui,
                                target_pid,
                                &norm_name,
                                &display_name,
                                &profile_key,
                                profile,
                                &running_games,
                                is_running,
                                is_active,
                                waiting_for_next_cycle,
                                current_step_idx,
                                window_title.as_deref(),
                                total_sent,
                                &rule_triggers,
                                true,
                            );
                        });
                } else {
                    self.draw_macro_modal_content(
                        ui,
                        target_pid,
                        &norm_name,
                        &display_name,
                        &profile_key,
                        profile,
                        &running_games,
                        is_running,
                        is_active,
                        waiting_for_next_cycle,
                        current_step_idx,
                        window_title.as_deref(),
                        total_sent,
                        &rule_triggers,
                        false,
                    );
                }
            },
        );

        if should_close {
            self.active_modal = None;
            self.capturing_macro_key_for = None;
            self.recording_macro_sequence = false;
            self.confirm_clear_macros = false;
        }
    }
}

fn egui_key_to_code(key: egui::Key) -> Option<&'static str> {
    match key {
        egui::Key::F1 => Some("F1"),
        egui::Key::F2 => Some("F2"),
        egui::Key::F3 => Some("F3"),
        egui::Key::F4 => Some("F4"),
        egui::Key::F5 => Some("F5"),
        egui::Key::F6 => Some("F6"),
        egui::Key::F7 => Some("F7"),
        egui::Key::F8 => Some("F8"),
        egui::Key::F9 => Some("F9"),
        egui::Key::F10 => Some("F10"),
        egui::Key::F11 => Some("F11"),
        egui::Key::F12 => Some("F12"),
        egui::Key::Num0 => Some("0"),
        egui::Key::Num1 => Some("1"),
        egui::Key::Num2 => Some("2"),
        egui::Key::Num3 => Some("3"),
        egui::Key::Num4 => Some("4"),
        egui::Key::Num5 => Some("5"),
        egui::Key::Num6 => Some("6"),
        egui::Key::Num7 => Some("7"),
        egui::Key::Num8 => Some("8"),
        egui::Key::Num9 => Some("9"),
        egui::Key::Space => Some("SPACE"),
        egui::Key::Tab => Some("TAB"),
        egui::Key::Enter => Some("ENTER"),
        egui::Key::Backspace => Some("BACKSPACE"),
        egui::Key::Insert => Some("INSERT"),
        egui::Key::Delete => Some("DELETE"),
        egui::Key::Home => Some("HOME"),
        egui::Key::End => Some("END"),
        egui::Key::PageUp => Some("PAGEUP"),
        egui::Key::PageDown => Some("PAGEDOWN"),
        egui::Key::ArrowUp => Some("UP"),
        egui::Key::ArrowDown => Some("DOWN"),
        egui::Key::ArrowLeft => Some("LEFT"),
        egui::Key::ArrowRight => Some("RIGHT"),
        egui::Key::A => Some("A"),
        egui::Key::B => Some("B"),
        egui::Key::C => Some("C"),
        egui::Key::D => Some("D"),
        egui::Key::E => Some("E"),
        egui::Key::F => Some("F"),
        egui::Key::G => Some("G"),
        egui::Key::H => Some("H"),
        egui::Key::I => Some("I"),
        egui::Key::J => Some("J"),
        egui::Key::K => Some("K"),
        egui::Key::L => Some("L"),
        egui::Key::M => Some("M"),
        egui::Key::N => Some("N"),
        egui::Key::O => Some("O"),
        egui::Key::P => Some("P"),
        egui::Key::Q => Some("Q"),
        egui::Key::R => Some("R"),
        egui::Key::S => Some("S"),
        egui::Key::T => Some("T"),
        egui::Key::U => Some("U"),
        egui::Key::V => Some("V"),
        egui::Key::W => Some("W"),
        egui::Key::X => Some("X"),
        egui::Key::Y => Some("Y"),
        egui::Key::Z => Some("Z"),
        _ => None,
    }
}

fn key_combo_box(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    key: &mut String,
    width: f32,
    allow_disable: bool,
    accent_when_set: bool,
) -> bool {
    let prev = key.clone();
    let label = if key.is_empty() {
        "Desativado".to_string()
    } else {
        key_display_label(key).to_string()
    };
    let text_color = if key.is_empty() {
        MUTED
    } else if accent_when_set {
        ACCENT
    } else {
        TEXT
    };
    egui::ComboBox::from_id_salt(id_salt)
        .selected_text(RichText::new(label).color(text_color).strong().size(12.0))
        .width(width)
        .show_ui(ui, |ui| {
            if allow_disable {
                ui.selectable_value(key, String::new(), "Desativado");
                ui.separator();
            }
            let mut last_cat = "";
            for spec in SUPPORTED_KEYS {
                if allow_disable && spec.vk == 0 {
                    continue;
                }
                if spec.category != last_cat {
                    if !last_cat.is_empty() {
                        ui.separator();
                    }
                    ui.label(
                        RichText::new(spec.category)
                            .color(MUTED)
                            .size(10.5)
                            .strong(),
                    );
                    last_cat = spec.category;
                }
                ui.selectable_value(key, spec.code.to_string(), spec.label);
            }
        });
    *key != prev
}

fn display_exec_name(name: &str) -> String {
    if cfg!(windows) && !name.to_ascii_lowercase().ends_with(".exe") {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn status_dot(ui: &mut egui::Ui, color: Color32, radius: f32) {
    let side = (radius * 2.0 + 2.0).max(8.0);
    let spot = ui.allocate_response(Vec2::splat(side), Sense::hover());
    ui.painter()
        .circle_filled(spot.rect.center(), radius, color);
}

fn modal_size(content_rect: egui::Rect, ideal: Vec2) -> Vec2 {
    let margin = 28.0;
    let max_w = (content_rect.width() - margin).max(300.0);
    let max_h = (content_rect.height() - margin).max(240.0);
    Vec2::new(ideal.x.min(max_w), ideal.y.min(max_h))
}

fn game_card_layout(total_w: f32) -> (usize, f32) {
    let gap = 10.0;
    let cols = ((total_w + gap) / 290.0).floor().clamp(1.0, 4.0) as usize;
    let card_w = ((total_w - gap * (cols.saturating_sub(1) as f32)) / cols as f32).max(230.0);
    (cols, card_w)
}

#[allow(clippy::too_many_arguments)]
fn styled_button(
    ui: &mut egui::Ui,
    label: impl Into<egui::WidgetText>,
    size: Option<Vec2>,
    inactive_bg: Color32,
    hovered_bg: Color32,
    active_bg: Color32,
    stroke: Stroke,
    corner: u8,
) -> egui::Response {
    ui.scope(|ui| {
        let w = &mut ui.style_mut().visuals.widgets;
        w.inactive.bg_fill = inactive_bg;
        w.inactive.weak_bg_fill = inactive_bg;
        w.inactive.bg_stroke = stroke;
        w.inactive.corner_radius = CornerRadius::same(corner);
        w.hovered.bg_fill = hovered_bg;
        w.hovered.bg_stroke = stroke;
        w.hovered.corner_radius = CornerRadius::same(corner);
        w.active.bg_fill = active_bg;
        w.active.bg_stroke = stroke;
        w.active.corner_radius = CornerRadius::same(corner);
        let btn = egui::Button::new(label)
            .stroke(stroke)
            .corner_radius(corner);
        match size {
            Some(sz) => ui.add_sized(sz, btn),
            None => ui.add(btn),
        }
    })
    .inner
}

fn primary_button(
    ui: &mut egui::Ui,
    label: &str,
    size: Option<Vec2>,
    text_size: f32,
) -> egui::Response {
    styled_button(
        ui,
        RichText::new(label)
            .color(ACCENT_TEXT)
            .size(text_size)
            .strong(),
        size,
        ACCENT,
        Color32::from_rgb(251, 177, 45),
        Color32::from_rgb(217, 136, 6),
        Stroke::NONE,
        7,
    )
}

fn secondary_button(
    ui: &mut egui::Ui,
    label: impl Into<egui::WidgetText>,
    size: Option<Vec2>,
) -> egui::Response {
    styled_button(
        ui,
        label,
        size,
        BG_ROW,
        Color32::from_rgb(45, 52, 66),
        ACCENT_SOFT,
        Stroke::new(1.0, STROKE),
        7,
    )
}

fn danger_button(
    ui: &mut egui::Ui,
    label: &str,
    size: Option<Vec2>,
    text_size: f32,
) -> egui::Response {
    styled_button(
        ui,
        RichText::new(label)
            .color(DANGER_TEXT)
            .size(text_size)
            .strong(),
        size,
        DANGER_BG,
        Color32::from_rgb(118, 36, 36),
        Color32::from_rgb(140, 42, 42),
        Stroke::new(1.0, DANGER.gamma_multiply(0.55)),
        7,
    )
}

#[allow(clippy::too_many_arguments)]
fn show_centered_modal<R>(
    ctx: &egui::Context,
    id: &str,
    title: &str,
    subtitle: Option<&str>,
    ideal_size: Vec2,
    accent_border: bool,
    allow_escape_close: bool,
    body: impl FnOnce(&mut egui::Ui) -> R,
) -> (bool, R) {
    let size = modal_size(ctx.content_rect(), ideal_size);
    let inner_size = (size - Vec2::splat(36.0)).max(Vec2::new(260.0, 200.0));
    let border = if accent_border {
        ACCENT.gamma_multiply(0.85)
    } else {
        STROKE_STRONG
    };
    let frame = egui::Frame::new()
        .fill(BG_MODAL)
        .stroke(Stroke::new(1.2, border))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(egui::Margin::same(18))
        .shadow(ctx.style_of(egui::Theme::Dark).visuals.popup_shadow);

    let mut close_clicked = false;
    let modal_resp = egui::Modal::new(egui::Id::new(id))
        .backdrop_color(Color32::from_black_alpha(175))
        .frame(frame)
        .show(ctx, |ui| {
            ui.allocate_ui_with_layout(inner_size, egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_min_size(inner_size);
                ui.set_max_size(inner_size);

                ui.horizontal(|ui| {
                    let title_w = (inner_size.x - 40.0).max(160.0);
                    let header_h = if subtitle.is_some() { 38.0 } else { 24.0 };
                    ui.allocate_ui_with_layout(
                        Vec2::new(title_w, header_h),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_max_width(title_w);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(title).color(TEXT).size(16.0).strong(),
                                )
                                .truncate(),
                            );
                            if let Some(sub) = subtitle {
                                ui.add(
                                    egui::Label::new(RichText::new(sub).color(MUTED).size(12.0))
                                        .truncate(),
                                );
                            }
                        },
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if styled_button(
                            ui,
                            RichText::new("✕").color(MUTED).size(13.0).strong(),
                            Some(Vec2::splat(28.0)),
                            BG_RAISED,
                            BG_ROW,
                            ACCENT_SOFT,
                            Stroke::new(1.0, STROKE),
                            7,
                        )
                        .on_hover_text("Fechar (Esc)")
                        .clicked()
                        {
                            close_clicked = true;
                        }
                    });
                });

                ui.add_space(6.0);
                ui.separator();
                ui.add_space(8.0);

                body(ui)
            })
            .inner
        });

    let escape_pressed = allow_escape_close
        && modal_resp.is_top_modal
        && !modal_resp.any_popup_open
        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));

    let should_close = close_clicked
        || (!modal_resp.any_popup_open && modal_resp.backdrop_response.clicked())
        || modal_resp.response.should_close()
        || escape_pressed;

    (should_close, modal_resp.inner)
}

fn section_label(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).color(MUTED).size(11.0).strong());
    ui.add_space(4.0);
}

fn status_chip(ui: &mut egui::Ui, watching: bool) {
    let (label, color) = if watching {
        ("Vigiando", OK)
    } else {
        ("Em espera", MUTED)
    };
    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(
            color.r(),
            color.g(),
            color.b(),
            28,
        ))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.45)))
        .corner_radius(16.0)
        .inner_margin(egui::Margin::symmetric(10, 5))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                status_dot(ui, color, 3.5);
                ui.label(RichText::new(label).color(color).size(12.0).strong());
            });
        });
}

fn kpi_badge(ui: &mut egui::Ui, label: &str, value: &str, dot: Option<Color32>, accent: Color32) {
    egui::Frame::new()
        .fill(BG_RAISED)
        .stroke(Stroke::new(1.0, STROKE))
        .corner_radius(7.0)
        .inner_margin(egui::Margin::symmetric(10, 5))
        .show(ui, |ui| {
            ui.set_min_height(18.0);
            ui.horizontal(|ui| {
                if let Some(dot_color) = dot {
                    status_dot(ui, dot_color, 3.5);
                }
                ui.label(RichText::new(label).color(MUTED).size(11.5));
                ui.label(RichText::new(value).color(accent).strong().size(12.0));
            });
        });
}

fn kpi_button(
    ui: &mut egui::Ui,
    label: &str,
    value: &str,
    dot: Option<Color32>,
    accent: Color32,
    hover: &str,
) -> egui::Response {
    let mut job = egui::text::LayoutJob::default();
    let font = FontId::new(12.0, FontFamily::Proportional);
    if let Some(dot_color) = dot {
        job.append(
            "● ",
            0.0,
            egui::TextFormat {
                font_id: FontId::new(10.5, FontFamily::Proportional),
                color: dot_color,
                ..Default::default()
            },
        );
    }
    job.append(
        label,
        0.0,
        egui::TextFormat {
            font_id: font.clone(),
            color: MUTED,
            ..Default::default()
        },
    );
    job.append(
        &format!(" {value}"),
        0.0,
        egui::TextFormat {
            font_id: font,
            color: accent,
            ..Default::default()
        },
    );
    ui.scope(|ui| {
        ui.spacing_mut().button_padding = Vec2::new(10.0, 5.0);
        styled_button(
            ui,
            job,
            Some(Vec2::new(0.0, 28.0)),
            BG_RAISED,
            BG_ROW,
            ACCENT_SOFT,
            Stroke::new(1.0, STROKE),
            7,
        )
    })
    .inner
    .on_hover_text(hover)
}

fn spawn_worker(
    config_path: PathBuf,
    initial_config: AppConfig,
    mut config_valid: bool,
    ui_rx: Receiver<UiCmd>,
    worker_tx: Sender<WorkerMsg>,
    watch_flag: Arc<AtomicBool>,
    ctx: egui::Context,
) {
    thread::spawn(move || {
        let mut interval = initial_config.gui.watch_interval_secs;
        let mut trim_game = initial_config.trim_game_memory;
        let mut config = initial_config;
        let mut tunnel_mgr = UiTunnelManager::new(config.wireguard.clone());
        let mut macro_mgr = MacroManager::new(config.macros.clone());
        let mut save_due: Option<Instant> = None;

        // Start last_scan in the past so already-open games are scanned immediately on app launch
        let mut last_scan = Instant::now() - Duration::from_secs(10);
        let mut last_optimize = Instant::now();
        let mut prev_game_count = 0usize;
        let mut active_game_pids: Vec<u32> = Vec::new();
        let mut active_games: Vec<(u32, String)> = Vec::new();
        let mut last_fast_tunnel_sync = Instant::now();
        let mut last_macro_ui_sync = Instant::now();

        let sync_tunnel = |mgr: &mut UiTunnelManager, pids: &[u32], tx: &Sender<WorkerMsg>| {
            let before = mgr.status();
            let pass = mgr.process_game_pids(pids);
            let after = mgr.status();
            #[cfg(target_os = "linux")]
            if before != after {
                ctx.request_repaint();
            }
            if cfg!(target_os = "linux") && before == after {
                return;
            }
            if pass.fallback_bypassed {
                send_worker(&ctx, tx, WorkerMsg::Log(
                    "⚠️ [Túnel VPS] Falha na conexão com a VPS detectada! Fallback automático ativado para novas conexões via operadora".to_string(),
                ));
            }
            if pass.recovered {
                send_worker(&ctx, tx, WorkerMsg::Log(
                    "🟢 [Túnel VPS] Conexão com a VPS restabelecida com sucesso! Reativando tunelamento otimizado".to_string(),
                ));
            }
            for conn in &pass.newly_routed {
                send_worker(
                    &ctx,
                    tx,
                    WorkerMsg::Log(format!(
                        "[Túnel VPS] Roteando {} ({}:{}) via VPS",
                        conn.protocol, conn.remote_ip, conn.remote_port
                    )),
                );
            }
            if !pass.removed_routes.is_empty() && !pass.fallback_bypassed {
                send_worker(
                    &ctx,
                    tx,
                    WorkerMsg::Log(format!(
                        "[Túnel VPS] {} rota(s) removida(s) de jogo finalizado",
                        pass.removed_routes.len()
                    )),
                );
            }
            send_worker(&ctx, tx, WorkerMsg::Tunnel(after));
        };

        #[cfg(target_os = "linux")]
        if config_valid {
            sync_tunnel(&mut tunnel_mgr, &[], &worker_tx);
        }
        let mut pending_macro_test = None;

        let test_macro_key =
            |mgr: &mut MacroManager, pid: u32, normalized_name: &str, key: &str| {
                let sent = mgr.test_single_key(pid, normalized_name, key);
                let label = key_display_label(key);
                let message = if sent > 0 {
                    format!("Teste de '{label}' enviado para {normalized_name} (PID {pid}).")
                } else if cfg!(target_os = "linux") {
                    format!("A janela do jogo não aceitou o teste de '{label}'.")
                } else {
                    format!("Janela visível indisponível para PID {pid} ao testar '{label}'.")
                };
                send_worker(&ctx, &worker_tx, WorkerMsg::Log(message));
            };

        loop {
            let poll_wait = if macro_mgr.has_active_instances() {
                Duration::from_millis(10)
            } else if macro_mgr.has_configured_instances() {
                Duration::from_millis(30)
            } else {
                Duration::from_millis(150)
            };
            match ui_rx.recv_timeout(poll_wait) {
                Ok(UiCmd::Scan) => {
                    if !config_valid {
                        continue;
                    }
                    let report = optimize_system(&OptimizeRequest {
                        config: config.clone(),
                        dry_run: true,
                    });
                    prev_game_count = report.games.len();
                    active_game_pids = report.games.iter().map(|g| g.pid).collect();
                    active_games = report
                        .games
                        .iter()
                        .map(|g| (g.pid, g.normalized_name.clone()))
                        .collect();
                    send_worker(
                        &ctx,
                        &worker_tx,
                        WorkerMsg::Log(format!(
                            "Scan: {} jogo(s) monitorado(s).",
                            report.games.len()
                        )),
                    );
                    sync_tunnel(&mut tunnel_mgr, &active_game_pids, &worker_tx);
                    let macro_statuses = macro_mgr.tick(&active_games, Instant::now());
                    send_worker(&ctx, &worker_tx, WorkerMsg::Macros(macro_statuses));
                    send_worker(&ctx, &worker_tx, WorkerMsg::Report(report));
                    last_scan = Instant::now();
                }
                Ok(UiCmd::RefreshProcesses(config)) => {
                    if !config_valid {
                        continue;
                    }
                    let processes = crate::detect::list_running_processes(&config);
                    send_worker(&ctx, &worker_tx, WorkerMsg::Processes(processes));
                }
                Ok(UiCmd::Optimize { trim_game: tg }) => {
                    if !config_valid {
                        continue;
                    }
                    trim_game = tg;
                    config.trim_game_memory = trim_game;
                    let report = optimize_system(&OptimizeRequest {
                        config: config.clone(),
                        dry_run: false,
                    });
                    prev_game_count = report.games.len();
                    active_game_pids = report.games.iter().map(|g| g.pid).collect();
                    active_games = report
                        .games
                        .iter()
                        .map(|g| (g.pid, g.normalized_name.clone()))
                        .collect();
                    let ok = report.results.iter().filter(|r| r.ok).count();
                    let fail = report.results.iter().filter(|r| !r.ok).count();
                    send_worker(
                        &ctx,
                        &worker_tx,
                        WorkerMsg::Log(format!(
                            "Otimização: {ok} ok, {fail} falha(s), {} jogo(s).",
                            report.games.len()
                        )),
                    );
                    sync_tunnel(&mut tunnel_mgr, &active_game_pids, &worker_tx);
                    let macro_statuses = macro_mgr.tick(&active_games, Instant::now());
                    send_worker(&ctx, &worker_tx, WorkerMsg::Macros(macro_statuses));
                    send_worker(&ctx, &worker_tx, WorkerMsg::Report(report));
                    last_optimize = Instant::now();
                    last_scan = Instant::now();
                }
                Ok(UiCmd::StartWatch {
                    interval: secs,
                    trim_game: tg,
                }) => {
                    if !config_valid {
                        continue;
                    }
                    interval = secs.max(15);
                    trim_game = tg;
                    config.gui.watch_enabled = true;
                    config.gui.watch_interval_secs = interval;
                    config.trim_game_memory = trim_game;
                    watch_flag.store(true, Ordering::SeqCst);
                    send_worker(&ctx, &worker_tx, WorkerMsg::Watching(true));
                    send_worker(
                        &ctx,
                        &worker_tx,
                        WorkerMsg::Log(format!("Vigilância a cada {interval}s.")),
                    );
                    last_optimize = Instant::now();
                }
                Ok(UiCmd::StopWatch) => {
                    watch_flag.store(false, Ordering::SeqCst);
                    config.gui.watch_enabled = false;
                    send_worker(&ctx, &worker_tx, WorkerMsg::Watching(false));
                    send_worker(
                        &ctx,
                        &worker_tx,
                        WorkerMsg::Log("Vigilância parada.".into()),
                    );
                }
                Ok(UiCmd::UpdateConfig(new_cfg)) => {
                    config = *new_cfg;
                    trim_game = config.trim_game_memory;
                    interval = config.gui.watch_interval_secs;
                    macro_mgr.update_profiles(config.macros.clone());
                    save_due = Some(Instant::now());
                }
                Ok(UiCmd::ReplaceConfig(new_cfg)) => {
                    config = *new_cfg;
                    config_valid = true;
                    interval = config.gui.watch_interval_secs;
                    trim_game = config.trim_game_memory;
                    watch_flag.store(config.gui.watch_enabled, Ordering::SeqCst);
                    tunnel_mgr.update_config(config.wireguard.clone());
                    macro_mgr.update_profiles(config.macros.clone());
                    send_worker(&ctx, &worker_tx, WorkerMsg::Tunnel(tunnel_mgr.status()));
                    let macro_statuses = macro_mgr.tick(&active_games, Instant::now());
                    send_worker(&ctx, &worker_tx, WorkerMsg::Macros(macro_statuses));
                }
                Ok(UiCmd::CommitConfig(new_cfg)) => {
                    config = *new_cfg;
                    config_valid = true;
                    interval = config.gui.watch_interval_secs;
                    trim_game = config.trim_game_memory;
                    tunnel_mgr.update_config(config.wireguard.clone());
                    #[cfg(target_os = "linux")]
                    tunnel_mgr.retry();
                    macro_mgr.update_profiles(config.macros.clone());
                    match config.save_to_path(&config_path) {
                        Ok(()) => {
                            save_due = None;
                            send_worker(
                                &ctx,
                                &worker_tx,
                                WorkerMsg::Log("Configuração salva no perfil.".into()),
                            );
                        }
                        Err(err) => {
                            send_worker(
                                &ctx,
                                &worker_tx,
                                WorkerMsg::Log(format!("Erro ao salvar configuração: {err}")),
                            );
                        }
                    }
                    send_worker(&ctx, &worker_tx, WorkerMsg::Tunnel(tunnel_mgr.status()));
                }
                #[cfg(target_os = "linux")]
                Ok(UiCmd::SetGameTunnel(game, enabled)) => {
                    if config_valid
                        && active_game_pids.contains(&game.pid)
                        && GameProcess::read(game.pid) == Some(game)
                    {
                        tunnel_mgr.set_game_enabled(game, enabled);
                        sync_tunnel(&mut tunnel_mgr, &active_game_pids, &worker_tx);
                    }
                }
                Ok(UiCmd::TestVpsPing) => {
                    if !config_valid {
                        continue;
                    }
                    send_worker(
                        &ctx,
                        &worker_tx,
                        WorkerMsg::Log("Testando ping para o endpoint da VPS...".into()),
                    );
                    match tunnel_mgr.test_vps_ping() {
                        Ok(ms) => {
                            send_worker(
                                &ctx,
                                &worker_tx,
                                WorkerMsg::Log(format!("Ping VPS: {ms}ms.")),
                            );
                        }
                        Err(err) => {
                            send_worker(&ctx, &worker_tx, WorkerMsg::Log(format!(
                                "Aviso ping VPS: Sem resposta ICMP ({err}). Se o firewall da sua VPS bloquear ping, isso é normal — o túnel WireGuard UDP opera normalmente e não depende de ping."
                            )));
                        }
                    }
                    send_worker(&ctx, &worker_tx, WorkerMsg::Tunnel(tunnel_mgr.status()));
                }
                Ok(UiCmd::SetMacroActive {
                    pid,
                    normalized_name,
                    active,
                }) => {
                    let st = macro_mgr.set_instance_active(pid, &normalized_name, active);
                    let state_str = if st.active { "ATIVADA" } else { "PAUSADA" };
                    send_worker(
                        &ctx,
                        &worker_tx,
                        WorkerMsg::Log(format!(
                            "⌨️ [Macro] Automação {state_str} para {normalized_name} (PID {pid})."
                        )),
                    );
                    let statuses = macro_mgr.tick(&active_games, Instant::now());
                    send_worker(&ctx, &worker_tx, WorkerMsg::Macros(statuses));
                }
                Ok(UiCmd::SetMacroRules {
                    pid,
                    normalized_name,
                    mode,
                    cycle_interval_ms,
                    toggle_key,
                    rules,
                }) => {
                    macro_mgr.set_instance_rules(
                        pid,
                        &normalized_name,
                        mode,
                        cycle_interval_ms,
                        toggle_key,
                        rules,
                    );
                    let statuses = macro_mgr.tick(&active_games, Instant::now());
                    send_worker(&ctx, &worker_tx, WorkerMsg::Macros(statuses));
                }
                Ok(UiCmd::TestMacroKey {
                    pid,
                    normalized_name,
                    key,
                }) => {
                    if cfg!(target_os = "linux") {
                        #[cfg(target_os = "linux")]
                        macro_mgr.prepare_background_test(pid);
                        pending_macro_test = Some((
                            Instant::now() + Duration::from_secs(3),
                            pid,
                            normalized_name,
                            key,
                        ));
                        send_worker(
                            &ctx,
                            &worker_tx,
                            WorkerMsg::Log("Teste de macro em 3 segundos.".into()),
                        );
                    } else {
                        test_macro_key(&mut macro_mgr, pid, &normalized_name, &key);
                    }
                    let statuses = macro_mgr.tick(&active_games, Instant::now());
                    send_worker(&ctx, &worker_tx, WorkerMsg::Macros(statuses));
                }
                Ok(UiCmd::Shutdown) => {
                    watch_flag.store(false, Ordering::SeqCst);
                    macro_mgr.shutdown();
                    tunnel_mgr.shutdown();
                    if save_due.take().is_some() && config_valid {
                        if let Err(err) = config.save_to_path(&config_path) {
                            send_worker(
                                &ctx,
                                &worker_tx,
                                WorkerMsg::Log(format!(
                                    "Erro ao salvar perfil no encerramento: {err}"
                                )),
                            );
                        }
                    }
                    send_worker(&ctx, &worker_tx, WorkerMsg::Stopped);
                    break;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if pending_macro_test
                        .as_ref()
                        .is_some_and(|(due, ..)| Instant::now() >= *due)
                    {
                        if let Some((_, pid, name, key)) = pending_macro_test.take() {
                            if active_games.iter().any(|(active_pid, active_name)| {
                                *active_pid == pid && *active_name == name
                            }) {
                                test_macro_key(&mut macro_mgr, pid, &name, &key);
                                send_worker(
                                    &ctx,
                                    &worker_tx,
                                    WorkerMsg::Macros(
                                        macro_mgr.tick(&active_games, Instant::now()),
                                    ),
                                );
                            }
                        }
                    }

                    if !active_games.is_empty()
                        && (macro_mgr.has_active_instances()
                            || macro_mgr.has_configured_instances())
                    {
                        let statuses = macro_mgr.tick(&active_games, Instant::now());
                        let mut any_toggled = false;
                        for st in &statuses {
                            if st.toggled_by_hotkey {
                                any_toggled = true;
                                let state_str = if st.active { "INICIADA" } else { "PAUSADA" };
                                let win_label =
                                    st.window_title.as_deref().unwrap_or(&st.normalized_name);
                                send_worker(
                                    &ctx,
                                    &worker_tx,
                                    WorkerMsg::Log(format!(
                                        "⌨️ [Macro] Tecla {} pressionada na janela \"{win_label}\" (PID {}): Automação {state_str} apenas nesta janela.",
                                        key_display_label(&st.toggle_key),
                                        st.pid
                                    )),
                                );
                            }
                        }
                        if any_toggled || last_macro_ui_sync.elapsed() >= Duration::from_millis(100)
                        {
                            last_macro_ui_sync = Instant::now();
                            send_worker(&ctx, &worker_tx, WorkerMsg::Macros(statuses));
                        }
                    }
                    if save_due.is_some_and(|changed_at| {
                        debounce_elapsed(changed_at, Instant::now(), Duration::from_millis(450))
                    }) && config_valid
                    {
                        save_due = None;
                        match config.save_to_path(&config_path) {
                            Ok(()) => send_worker(
                                &ctx,
                                &worker_tx,
                                WorkerMsg::Log("Preferências salvas.".into()),
                            ),
                            Err(err) => send_worker(
                                &ctx,
                                &worker_tx,
                                WorkerMsg::Log(format!("Erro ao salvar preferências: {err}")),
                            ),
                        }
                    }
                    // Fast socket sync loop: Every 150ms when games are active,
                    // catch new game server connections instantaneously!
                    if config_valid
                        && (cfg!(target_os = "linux") || !active_game_pids.is_empty())
                        && last_fast_tunnel_sync.elapsed() >= Duration::from_millis(150)
                    {
                        last_fast_tunnel_sync = Instant::now();
                        sync_tunnel(&mut tunnel_mgr, &active_game_pids, &worker_tx);
                    }

                    // Periodic system scan every 2 seconds to detect new/closed processes
                    if config_valid && last_scan.elapsed() >= Duration::from_secs(2) {
                        last_scan = Instant::now();
                        let is_watch_active = watch_flag.load(Ordering::SeqCst);
                        let should_optimize = !cfg!(target_os = "linux")
                            && is_watch_active
                            && last_optimize.elapsed() >= Duration::from_secs(interval.max(15));

                        config.trim_game_memory = trim_game;
                        let report = optimize_system(&OptimizeRequest {
                            config: config.clone(),
                            dry_run: !should_optimize,
                        });

                        if should_optimize {
                            last_optimize = Instant::now();
                            let ok = report.results.iter().filter(|r| r.ok).count();
                            send_worker(
                                &ctx,
                                &worker_tx,
                                WorkerMsg::Log(format!(
                                    "Vigilância periódica: {ok} ações aplicadas."
                                )),
                            );
                        }

                        active_game_pids = report.games.iter().map(|g| g.pid).collect();
                        active_games = report
                            .games
                            .iter()
                            .map(|g| (g.pid, g.normalized_name.clone()))
                            .collect();
                        if !report.games.is_empty() || prev_game_count > 0 {
                            sync_tunnel(&mut tunnel_mgr, &active_game_pids, &worker_tx);
                            let statuses = macro_mgr.tick(&active_games, Instant::now());
                            send_worker(&ctx, &worker_tx, WorkerMsg::Macros(statuses));
                            send_worker(&ctx, &worker_tx, WorkerMsg::Report(report.clone()));
                        }
                        prev_game_count = report.games.len();
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    if save_due.is_some() && config_valid {
                        let _ = config.save_to_path(&config_path);
                    }
                    macro_mgr.shutdown();
                    tunnel_mgr.shutdown();
                    send_worker(&ctx, &worker_tx, WorkerMsg::Stopped);
                    break;
                }
            }
        }
    });
}

#[cfg(test)]
mod gui_state_tests {
    use super::{
        contain_list_scroll, debounce_elapsed, game_card_layout, macro_profile_key, modal_size,
        narrow_layout, show_centered_modal, SCROLL_AREA_IDS,
    };
    use std::collections::HashSet;
    use std::time::{Duration, Instant};

    #[test]
    fn macro_profile_key_isolates_windows_and_games() {
        let win1 = macro_profile_key("mir4g", Some("Mir4G[1]"), 101);
        let win2 = macro_profile_key("mir4g", Some("Mir4G[2]"), 202);
        let generic1 = macro_profile_key("mir4g", Some("Mir4G"), 101);
        let generic2 = macro_profile_key("mir4g", None, 202);
        let other_game = macro_profile_key("albion-online", None, 303);

        assert_eq!(win1, "mir4g::Mir4G[1]");
        assert_eq!(win2, "mir4g::Mir4G[2]");
        assert_ne!(win1, win2);
        assert_eq!(generic1, "mir4g#pid:101");
        assert_eq!(generic2, "mir4g#pid:202");
        assert_ne!(generic1, generic2);
        assert_ne!(generic1, other_game);
    }

    #[test]
    fn preference_snapshot_round_trips_values_and_normalizes_bounds() {
        let preferences = crate::config::GuiPreferences {
            watch_interval_secs: 90,
            watch_enabled: true,
            window_width: 1280,
            window_height: 720,
            window_maximized: true,
        };
        let serialized = toml::to_string(&preferences).unwrap();
        let snapshot: crate::config::GuiPreferences = toml::from_str(&serialized).unwrap();
        assert_eq!(snapshot, preferences);
        let normalized = snapshot.normalized();
        assert_eq!(normalized.window_width, 1280);
        assert_eq!(normalized.watch_interval_secs, 90);
    }

    #[test]
    fn preference_snapshot_clamps_invalid_dimensions_and_interval() {
        let preferences = crate::config::GuiPreferences {
            watch_interval_secs: 1,
            window_width: 1,
            window_height: 1,
            ..Default::default()
        };
        let normalized = preferences.normalized();
        assert_eq!(normalized.watch_interval_secs, 15);
        assert_eq!(
            (normalized.window_width, normalized.window_height),
            (640, 420)
        );
    }

    #[test]
    fn debounce_only_expires_after_delay() {
        let changed = Instant::now();
        assert!(!debounce_elapsed(
            changed,
            changed + Duration::from_millis(299),
            Duration::from_millis(300)
        ));
        assert!(debounce_elapsed(
            changed,
            changed + Duration::from_millis(300),
            Duration::from_millis(300)
        ));
    }

    #[test]
    fn all_scroll_areas_have_independent_stable_ids() {
        let unique: HashSet<_> = SCROLL_AREA_IDS.iter().copied().collect();
        assert_eq!(unique.len(), SCROLL_AREA_IDS.len());
    }

    #[test]
    fn narrow_layout_breakpoint_keeps_sidebar_in_vertical_scroll_content() {
        assert!(narrow_layout(640.0));
        assert!(!narrow_layout(880.0));
    }

    struct ScrollSnapshot {
        first_id: egui::Id,
        first_rect: egui::Rect,
        first_offset: f32,
        first_max: f32,
        sibling_offset: f32,
        parent_offset: f32,
    }

    fn scroll_fixture(
        ctx: &egui::Context,
        pointer: Option<egui::Pos2>,
        delta: f32,
        contain: bool,
    ) -> ScrollSnapshot {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400.0, 320.0),
            )),
            events: pointer.map(egui::Event::PointerMoved).into_iter().collect(),
            ..Default::default()
        };
        let mut snapshot = None;
        let output = ctx.run_ui(input, |ui| {
            // Inject the same wheel delta consumed by egui's ScrollArea, without
            // making this regression depend on platform wheel smoothing.
            ui.input_mut(|input| input.smooth_scroll_delta = egui::vec2(0.0, delta));
            let parent = egui::ScrollArea::vertical()
                .id_salt(SCROLL_AREA_IDS[5])
                .max_height(220.0)
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    let first = egui::ScrollArea::vertical()
                        .id_salt(SCROLL_AREA_IDS[6])
                        .max_height(80.0)
                        .auto_shrink([false; 2])
                        .show(ui, |ui| {
                            ui.allocate_space(egui::vec2(200.0, 500.0));
                        });
                    if contain {
                        contain_list_scroll(ui, first.inner_rect);
                    }
                    let sibling = egui::ScrollArea::vertical()
                        .id_salt(SCROLL_AREA_IDS[7])
                        .max_height(80.0)
                        .auto_shrink([false; 2])
                        .show(ui, |ui| {
                            ui.allocate_space(egui::vec2(200.0, 500.0));
                        });
                    ui.allocate_space(egui::vec2(200.0, 400.0));
                    (first, sibling)
                });
            let (first, sibling) = parent.inner;
            snapshot = Some(ScrollSnapshot {
                first_id: first.id,
                first_rect: first.inner_rect,
                first_offset: first.state.offset.y,
                first_max: first.content_size.y - first.inner_rect.height(),
                sibling_offset: sibling.state.offset.y,
                parent_offset: parent.state.offset.y,
            });
        });
        output.drop_without_applying_deltas();
        snapshot.unwrap()
    }

    #[test]
    fn wheel_at_process_list_boundary_does_not_scroll_the_dialog() {
        for contain in [false, true] {
            let ctx = egui::Context::default();
            ctx.set_fonts(egui::FontDefinitions::empty());
            let initial = scroll_fixture(&ctx, None, 0.0, contain);
            let mut state = egui::scroll_area::State::load(&ctx, initial.first_id).unwrap();
            state.offset.y = initial.first_max;
            state.store(&ctx, initial.first_id);
            let result = scroll_fixture(&ctx, Some(initial.first_rect.center()), -40.0, contain);
            assert_eq!(result.sibling_offset, 0.0);
            if contain {
                assert_eq!(result.parent_offset, 0.0);
            } else {
                assert!(
                    result.parent_offset > 0.0,
                    "fixture must reproduce scroll chaining"
                );
            }
        }
    }

    #[test]
    fn wheel_scrolls_only_the_hovered_process_list() {
        let ctx = egui::Context::default();
        ctx.set_fonts(egui::FontDefinitions::empty());
        let initial = scroll_fixture(&ctx, None, 0.0, true);
        let result = scroll_fixture(&ctx, Some(initial.first_rect.center()), -40.0, true);
        assert!(result.first_offset > 0.0);
        assert_eq!(result.sibling_offset, 0.0);
        assert_eq!(result.parent_offset, 0.0);
    }

    #[test]
    fn modal_size_fits_within_small_default_and_large_viewports() {
        let macro_ideal = egui::vec2(820.0, 620.0);
        for (vw, vh, expected) in [
            (640.0, 420.0, egui::vec2(612.0, 392.0)),
            (880.0, 560.0, egui::vec2(820.0, 532.0)),
            (1040.0, 680.0, egui::vec2(820.0, 620.0)),
            (1920.0, 1080.0, egui::vec2(820.0, 620.0)),
        ] {
            let content = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(vw, vh));
            let sized = modal_size(content, macro_ideal);
            assert_eq!(sized, expected, "unexpected modal size for {vw}x{vh}");
            assert!(sized.x <= vw - 28.0);
            assert!(sized.y <= vh - 28.0);
        }

        let small_content = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640.0, 420.0));
        for ideal in [egui::vec2(780.0, 580.0), egui::vec2(660.0, 560.0)] {
            let sized = modal_size(small_content, ideal);
            assert!(sized.x <= 612.0);
            assert!(sized.y <= 392.0);
        }
    }

    #[test]
    fn game_card_layout_scales_fluidly_across_widths() {
        assert_eq!(game_card_layout(360.0), (1, 360.0));

        let (cols_600, w_600) = game_card_layout(600.0);
        assert_eq!(cols_600, 2);
        assert!((w_600 - 295.0).abs() < 0.01);

        let (cols_920, w_920) = game_card_layout(920.0);
        assert_eq!(cols_920, 3);
        assert!((w_920 - 300.0).abs() < 0.01);

        let (cols_1280, w_1280) = game_card_layout(1280.0);
        assert_eq!(cols_1280, 4);
        assert!((w_1280 - 312.5).abs() < 0.01);
    }

    #[test]
    fn centered_modal_renders_centered_inside_viewport_and_respects_escape_guard() {
        let ctx = egui::Context::default();
        ctx.set_fonts(egui::FontDefinitions::empty());
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1040.0, 680.0));

        let run_frame = |events: Vec<egui::Event>, allow_escape: bool| {
            let input = egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            };
            let mut body_rect = egui::Rect::NOTHING;
            let mut should_close = false;
            let output = ctx.run_ui(input, |_ui| {
                let (closed, rect) = show_centered_modal(
                    &ctx,
                    "test_centered_modal",
                    "Modal de Teste",
                    Some("Subtítulo de teste"),
                    egui::vec2(820.0, 620.0),
                    true,
                    allow_escape,
                    |ui| {
                        let rect = ui.max_rect();
                        ui.allocate_space(ui.available_size());
                        rect
                    },
                );
                should_close = closed;
                body_rect = rect;
            });
            output.drop_without_applying_deltas();
            (should_close, body_rect)
        };

        // Frame 1: settles modal layout
        let (closed_1, _) = run_frame(Vec::new(), true);
        assert!(!closed_1);

        // Frame 2: verify centered bounds & Escape blocked when allow_escape_close is false
        let escape_event = egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let (closed_2, rect_2) = run_frame(vec![escape_event.clone()], false);
        assert!(!closed_2);
        assert!(
            screen.contains_rect(rect_2),
            "modal rect {rect_2:?} outside {screen:?}"
        );
        assert!((rect_2.center().x - screen.center().x).abs() <= 4.0);
        assert!(
            (rect_2.width() - 784.0).abs() <= 4.0,
            "expected inner width ~784, got {}",
            rect_2.width()
        );

        // Frame 3: Escape closes when allow_escape_close is true
        let (closed_3, _) = run_frame(vec![escape_event], true);
        assert!(closed_3);
    }

    #[test]
    fn combobox_inside_centered_modal_keeps_modal_open_on_popup_escape() {
        let ctx = egui::Context::default();
        ctx.set_fonts(egui::FontDefinitions::empty());
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1040.0, 680.0));
        let mut selected = "F1".to_string();

        let run_step = |events: Vec<egui::Event>, selected: &mut String| {
            let input = egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            };
            let mut should_close = false;
            let mut btn_rect = egui::Rect::NOTHING;
            let output = ctx.run_ui(input, |_ui| {
                let (closed, rect) = show_centered_modal(
                    &ctx,
                    "test_combo_modal",
                    "Modal Combo",
                    None,
                    egui::vec2(640.0, 480.0),
                    false,
                    true,
                    |ui| {
                        let resp = egui::ComboBox::from_id_salt("test-modal-combo")
                            .selected_text(selected.as_str())
                            .show_ui(ui, |ui| {
                                ui.selectable_value(selected, "F1".into(), "F1");
                                ui.selectable_value(selected, "F2".into(), "F2");
                            });
                        resp.response.rect
                    },
                );
                should_close = closed;
                btn_rect = rect;
            });
            output.drop_without_applying_deltas();
            (should_close, btn_rect)
        };

        let _ = run_step(Vec::new(), &mut selected);
        let (_, combo_rect) = run_step(Vec::new(), &mut selected);

        // Click ComboBox to open its popup
        let click_pos = combo_rect.center();
        let (closed_open, _) = run_step(
            vec![
                egui::Event::PointerMoved(click_pos),
                egui::Event::PointerButton {
                    pos: click_pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos: click_pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            &mut selected,
        );
        assert!(!closed_open);
        assert!(egui::Popup::is_any_open(&ctx));

        // Pressing Escape while ComboBox popup is open should close only the popup, not the modal
        let (closed_after_esc, _) = run_step(
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            &mut selected,
        );
        assert!(!closed_after_esc);
    }
}
