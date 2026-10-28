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
use image::{ImageBuffer, Rgba};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder, TrayIconEvent};

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

enum AppTrayEvent {
    Menu(MenuEvent),
    Tray(TrayIconEvent),
}

/// Launch the native GUI (blocks until quit).
pub fn run(config_path: PathBuf) -> eframe::Result<()> {
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

    let (tray_tx, tray_rx) = mpsc::channel::<AppTrayEvent>();

    eframe::run_native(
        "Game Optimizer",
        options,
        Box::new(move |cc| {
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

            install_fonts(&cc.egui_ctx);
            style_visuals(&cc.egui_ctx);
            Ok(
                Box::new(GuiApp::new(config_path, icon, tray_rx, cc.egui_ctx.clone()))
                    as Box<dyn App>,
            )
        }),
    )
}

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    // Prefer Windows Segoe UI for a friendlier native feel.
    let candidates = [
        r"C:\Windows\Fonts\segoeui.ttf",
        r"C:\Windows\Fonts\calibri.ttf",
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
    visuals.window_fill = BG;
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
        style.spacing.window_margin = egui::Margin::same(0);
        style.spacing.indent = 12.0;
    });
}

fn tray_rgba_icon() -> Icon {
    let size = 32u32;
    let mut img = ImageBuffer::<Rgba<u8>, Vec<u8>>::new(size, size);
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 - 15.5;
            let dy = y as f32 - 15.5;
            let r = (dx * dx + dy * dy).sqrt();
            let pixel = if r < 14.0 {
                Rgba([232, 168, 56, 255])
            } else if r < 16.0 {
                Rgba([22, 24, 28, 255])
            } else {
                Rgba([0, 0, 0, 0])
            };
            img.put_pixel(x, y, pixel);
        }
    }
    Icon::from_rgba(img.into_raw(), size, size).expect("tray icon")
}

fn eframe_icon() -> egui::IconData {
    let size = 32u32;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 - 15.5;
            let dy = y as f32 - 15.5;
            let r = (dx * dx + dy * dy).sqrt();
            if r < 14.0 {
                rgba.extend_from_slice(&[232, 168, 56, 255]);
            } else if r < 16.0 {
                rgba.extend_from_slice(&[22, 24, 28, 255]);
            } else {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    egui::IconData {
        rgba,
        width: size,
        height: size,
    }
}

fn paint_bg(ui: &egui::Ui) {
    let rect = ui.max_rect();
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, BG);
    // Soft warm glow top-left (atmosphere without flat fill).
    let glow = egui::Rect::from_min_size(rect.min, Vec2::new(rect.width() * 0.55, 120.0));
    painter.rect_filled(glow, 0.0, Color32::from_rgba_unmultiplied(232, 168, 56, 18));
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
    start_minimized_hint: bool,
    wireguard_cfg: WireGuardConfig,
    tunnel_status: Option<TunnelStatus>,
    show_tunnel_modal: bool,
    show_process_modal: bool,
    process_tab: ProcessTab,
    running_processes: Vec<RunningProcessInfo>,
    process_search_filter: String,
    new_game_input: String,
    new_reclaim_input: String,
    new_never_trim_input: String,
    show_paste_conf: bool,
    paste_conf_text: String,
    macro_statuses: Vec<MacroInstanceStatus>,
    show_macro_modal: bool,
    macro_target_pid: Option<u32>,
    macro_target_name: String,
    macro_target_normalized: String,
    capturing_macro_key_for: Option<Option<usize>>,
    recording_macro_sequence: bool,
    default_macro_delay_ms: u64,
    /// Must stay owned so the tray icon is not destroyed.
    #[allow(dead_code)]
    tray: Option<TrayIcon>,
    tray_show: MenuItem,
    tray_optimize: MenuItem,
    tray_quit: MenuItem,
    ui_tx: Sender<UiCmd>,
    worker_rx: Receiver<WorkerMsg>,
    tray_rx: Receiver<AppTrayEvent>,
    started: Instant,
    geometry_dirty_at: Option<Instant>,
    exiting: bool,
    shutdown_sent: bool,
    worker_stopped: bool,
}

impl GuiApp {
    fn new(
        config_path: PathBuf,
        icon: Icon,
        tray_rx: Receiver<AppTrayEvent>,
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
            logs: vec!["Pronto. O X ou botão Bandeja minimizam para a bandeja.".into()],
            watching: false,
            watch_interval,
            trim_game_memory,
            autostart,
            start_minimized_hint: false,
            wireguard_cfg,
            tunnel_status: None,
            show_tunnel_modal: false,
            show_process_modal: false,
            process_tab: ProcessTab::RunningProcesses,
            running_processes: Vec::new(),
            process_search_filter: String::new(),
            new_game_input: String::new(),
            new_reclaim_input: String::new(),
            new_never_trim_input: String::new(),
            show_paste_conf: false,
            paste_conf_text: String::new(),
            macro_statuses: Vec::new(),
            show_macro_modal: false,
            macro_target_pid: None,
            macro_target_name: String::new(),
            macro_target_normalized: String::new(),
            capturing_macro_key_for: None,
            recording_macro_sequence: false,
            default_macro_delay_ms: DEFAULT_MACRO_INTERVAL_MS,
            tray,
            tray_show,
            tray_optimize,
            tray_quit,
            ui_tx,
            worker_rx,
            tray_rx,
            started: Instant::now(),
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
        if proc.root_name != proc.normalized_name && self.app_config.add_game_name(&proc.root_name)
        {
            added = true;
        }
        if added {
            if proc.root_name != proc.normalized_name {
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
        if proc.root_name != proc.normalized_name
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
                        self.push_log(format!(
                            "Configuração WireGuard reimportada com sucesso de '{file_name}'."
                        ));
                        self.commit_wireguard_config();
                        let _ = self.ui_tx.send(UiCmd::Scan);
                    }
                    Err(err) => {
                        self.push_log(format!("Erro ao processar arquivo .conf: {err}"));
                    }
                },
                Err(err) => {
                    self.push_log(format!("Falha ao ler arquivo: {err}"));
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
                self.push_log(format!(
                    "Falha ao recarregar configuração: {err}. A última configuração válida foi preservada; nada foi sobrescrito."
                ));
                return;
            }
        };
        self.app_config = loaded;
        self.app_config.gui = self.app_config.gui.clone().normalized();
        self.wireguard_cfg = self.app_config.wireguard.clone();
        self.trim_game_memory = self.app_config.trim_game_memory;
        self.watch_interval = self.app_config.gui.watch_interval_secs;
        self.watching = self.app_config.gui.watch_enabled;
        self.push_log("Configurações recarregadas do perfil (%LOCALAPPDATA%\\GameOptimizer).");
        let _ = self
            .ui_tx
            .send(UiCmd::ReplaceConfig(Box::new(self.app_config.clone())));
        let _ = self.ui_tx.send(UiCmd::Scan);
    }

    fn pick_game_exe(&mut self) {
        if let Some(exe_path) = rfd::FileDialog::new()
            .add_filter("Executáveis (*.exe)", &["exe"])
            .set_title("Selecione o executável do Jogo")
            .pick_file()
        {
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
                    self.report = Some(report);
                }
                WorkerMsg::Log(line) => self.push_log(line),
                WorkerMsg::Watching(active) => {
                    self.watching = active;
                }
                WorkerMsg::Tunnel(status) => {
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

    fn restore_window(&self, ctx: &egui::Context) {
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
        self.poll_tray(ctx);
        if self.exiting && self.worker_stopped {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            ctx.request_repaint();
            return;
        }

        let viewport = ctx.input(|i| i.viewport().clone());
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
            if self.exiting || self.tray.is_none() {
                if !self.exiting {
                    self.exiting = true;
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            }
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

        let repaint_ms = if self.show_macro_modal && self.macro_statuses.iter().any(|s| s.active) {
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
                self.draw_header(ui, &ctx);
                ui.add_space(10.0);
                self.draw_body(ui, &ctx);
            });

        if self.show_tunnel_modal {
            self.draw_tunnel_modal(&ctx);
        }
        if self.show_process_modal {
            self.draw_process_modal(&ctx);
        }
        if self.show_macro_modal {
            self.draw_macro_modal(&ctx);
        }
    }
}

impl GuiApp {
    fn draw_header(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let pulse = ((self.started.elapsed().as_secs_f32() * 1.6).sin() + 1.0) * 0.5;
        let brand_glow = Color32::from_rgba_unmultiplied(
            ACCENT.r(),
            ACCENT.g(),
            ACCENT.b(),
            35 + (pulse * 45.0) as u8,
        );

        ui.horizontal_wrapped(|ui| {
            // Brand mark
            let mark = ui.allocate_response(Vec2::splat(32.0), Sense::hover());
            ui.painter()
                .circle_filled(mark.rect.center(), 14.0, brand_glow);
            ui.painter().circle_filled(mark.rect.center(), 9.0, ACCENT);

            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Game Optimizer")
                            .font(FontId::new(21.0, FontFamily::Proportional))
                            .color(TEXT)
                            .strong(),
                    );
                    ui.add_space(6.0);
                    status_chip(ui, self.watching);
                });
                ui.label(
                    RichText::new("Tunelamento VPS Autônomo & Acelerador de Jogos")
                        .color(MUTED)
                        .size(12.0),
                );
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let primary = ui
                    .add_enabled_ui(self.config_valid, |ui| {
                        ui.add_sized(
                            [150.0, 36.0],
                            egui::Button::new(
                                RichText::new("⚡ Otimizar Agora")
                                    .strong()
                                    .size(13.0)
                                    .color(Color32::from_rgb(18, 14, 8)),
                            )
                            .fill(ACCENT)
                            .corner_radius(8.0),
                        )
                    })
                    .inner;
                if primary.clicked() {
                    let _ = self.ui_tx.send(UiCmd::Optimize {
                        trim_game: self.trim_game_memory,
                    });
                }

                ui.add_space(6.0);

                if ui
                    .add(
                        egui::Button::new(RichText::new("Bandeja").color(MUTED).size(12.0))
                            .fill(BG_RAISED)
                            .stroke(Stroke::new(1.0, STROKE))
                            .corner_radius(6.0),
                    )
                    .on_hover_text("Minimizar para a bandeja do sistema")
                    .clicked()
                {
                    if self.tray.is_some() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                    } else {
                        self.push_log("A bandeja não está disponível; a janela permanece aberta.");
                    }
                }
            });
        });

        ui.add_space(10.0);
        // Metric KPI strip
        ui.horizontal_wrapped(|ui| {
            if ui
                .add(
                    egui::Button::new(
                        RichText::new(format!("🎮 Jogos: {}", self.game_count()))
                            .color(ACCENT)
                            .strong()
                            .size(12.0),
                    )
                    .fill(BG_RAISED)
                    .stroke(Stroke::new(1.0, STROKE))
                    .corner_radius(8.0),
                )
                .on_hover_text("Clique para gerenciar jogos monitorados")
                .clicked()
            {
                self.process_tab = ProcessTab::Games;
                self.show_process_modal = true;
            }

            let (trim_label, trim_color) = if self.trim_game_memory {
                ("Ativo (Jogos)", OK)
            } else if self.app_config.trim_non_game_memory {
                ("Ativo (Segundo Plano)", OK)
            } else {
                ("Desativado", MUTED)
            };
            if ui
                .add(
                    egui::Button::new(
                        RichText::new(format!("🧹 Trim: {trim_label}"))
                            .color(trim_color)
                            .strong()
                            .size(12.0),
                    )
                    .fill(BG_RAISED)
                    .stroke(Stroke::new(1.0, STROKE))
                    .corner_radius(8.0),
                )
                .on_hover_text("Clique para gerenciar regras de trim de memória")
                .clicked()
            {
                self.process_tab = ProcessTab::MemoryTrim;
                self.show_process_modal = true;
            }

            metric_pill(
                ui,
                "⏱️ Intervalo",
                &format!("{}s", self.watch_interval),
                MUTED,
            );

            let (tunnel_label, tunnel_col) = match &self.tunnel_status {
                Some(s) if s.health == TunnelHealth::Healthy => {
                    if let Some(ms) = s.vps_ping_ms {
                        (format!("Ativo ({ms}ms)"), OK)
                    } else {
                        (format!("Ativo ({} rota(s))", s.active_routes_count), OK)
                    }
                }
                Some(s) if s.health == TunnelHealth::Connecting => ("Conectando…".into(), ACCENT),
                Some(s) if s.health == TunnelHealth::Degraded => {
                    ("⚠️ Falha VPS (Fallback Direto)".into(), WARN)
                }
                _ if self.wireguard_cfg.enabled => ("Pronto".into(), ACCENT),
                _ => ("Desativado".into(), MUTED),
            };

            if ui
                .add(
                    egui::Button::new(
                        RichText::new(format!("🛡️ Túnel VPS: {tunnel_label}"))
                            .color(tunnel_col)
                            .size(12.0)
                            .strong(),
                    )
                    .fill(BG_RAISED)
                    .stroke(Stroke::new(1.0, STROKE))
                    .corner_radius(8.0),
                )
                .on_hover_text("Clique para abrir configurações e opções do túnel WireGuard")
                .clicked()
            {
                self.show_tunnel_modal = true;
            }

            if ui
                .add(
                    egui::Button::new(
                        RichText::new("📥 Importar .conf")
                            .color(Color32::from_rgb(28, 22, 12))
                            .size(12.0)
                            .strong(),
                    )
                    .fill(ACCENT)
                    .corner_radius(8.0),
                )
                .on_hover_text("Importe ou reimporte diretamente um arquivo .conf do WireGuard")
                .clicked()
            {
                self.import_conf_file();
            }
        });
    }

    fn draw_body(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) {
        if !self.config_valid {
            egui::Frame::new().fill(Color32::from_rgb(55, 25, 22)).stroke(Stroke::new(1.0, WARN)).inner_margin(8.0).show(ui, |ui| {
                ui.label(RichText::new("Configuração inválida. Otimizações e gravações estão bloqueadas para preservar o perfil. Use ‘Recarregar do Perfil’ após corrigir o arquivo.").color(TEXT).strong());
            });
            ui.add_space(6.0);
        }
        let available = ui.available_height().max(1.0);
        let width = ui.available_width();
        if narrow_layout(width) || available < 340.0 {
            egui::ScrollArea::vertical()
                .id_salt(SCROLL_AREA_IDS[0])
                .max_height(available)
                .show(ui, |ui| {
                    ui.set_width(width);
                    self.draw_game_panels(ui, available);
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
                    |ui| self.draw_game_panels(ui, available),
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

    fn draw_game_panels(&mut self, ui: &mut egui::Ui, available: f32) {
        section_label(ui, "JOGOS EM EXECUÇÃO");
        egui::Frame::new()
            .fill(BG_RAISED)
            .stroke(Stroke::new(1.0, STROKE))
            .corner_radius(10.0)
            .inner_margin(12.0)
            .show(ui, |ui| {
                let list_h = (available * 0.44).clamp(110.0, 190.0);
                let list = egui::ScrollArea::vertical()
                    .id_salt(SCROLL_AREA_IDS[1])
                    .max_height(list_h)
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        self.draw_games(ui);
                    });
                contain_list_scroll(ui, list.inner_rect);
            });

        ui.add_space(10.0);
        section_label(ui, "LOG DE ATIVIDADES DO TÚNEL & SISTEMA");
        egui::Frame::new()
            .fill(Color32::from_rgb(11, 13, 17))
            .stroke(Stroke::new(1.0, STROKE))
            .corner_radius(10.0)
            .inner_margin(10.0)
            .show(ui, |ui| {
                let rest = ui
                    .available_height()
                    .min((available * 0.4).clamp(90.0, 220.0))
                    .max(80.0);
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
                contain_list_scroll(ui, log.inner_rect);
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
                if ui
                    .add_sized(
                        [ui.available_width(), 34.0],
                        egui::Button::new(
                            RichText::new("🔄 Atualizar Detecção").color(TEXT).strong(),
                        )
                        .fill(BG_ROW)
                        .stroke(Stroke::new(1.0, STROKE))
                        .corner_radius(7.0),
                    )
                    .clicked()
                {
                    let _ = self.ui_tx.send(UiCmd::Scan);
                }
                ui.add_space(6.0);
                if self.watching {
                    if ui
                        .add_sized(
                            [ui.available_width(), 34.0],
                            egui::Button::new(
                                RichText::new("⏸️ Parar Vigilância")
                                    .color(Color32::from_rgb(254, 202, 202))
                                    .strong(),
                            )
                            .fill(Color32::from_rgb(88, 28, 28))
                            .corner_radius(7.0),
                        )
                        .clicked()
                    {
                        self.watching = false;
                        self.app_config.gui.watch_enabled = false;
                        let _ = self.ui_tx.send(UiCmd::StopWatch);
                        self.save_app_config();
                    }
                } else if self.config_valid
                    && ui
                        .add_sized(
                            [ui.available_width(), 34.0],
                            egui::Button::new(
                                RichText::new("🛡️ Vigiar Automaticamente")
                                    .color(TEXT)
                                    .strong(),
                            )
                            .fill(BG_ROW)
                            .stroke(Stroke::new(1.0, STROKE))
                            .corner_radius(7.0),
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
                                if ui
                                    .checkbox(
                                        &mut self.autostart,
                                        RichText::new("Iniciar com o Windows")
                                            .color(TEXT)
                                            .size(13.0),
                                    )
                                    .on_hover_text("O Windows pedirá autorização de administrador (UAC) ao entrar. Se você cancelar, o programa não será iniciado.")
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
                                            self.push_log(format!("Falha ao alterar inicialização automática: {err}"));
                                        }
                                    }
                                }

                                ui.add_space(4.0);
                                if ui
                                    .checkbox(
                                        &mut self.trim_game_memory,
                                        RichText::new("Trim de memória nos Jogos").color(TEXT).size(13.0),
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
                                        let _ = self.ui_tx.send(UiCmd::StartWatch { interval: self.watch_interval, trim_game: self.trim_game_memory });
                                    }
                                    self.save_app_config();
                                }
                                ui.label(
                                    RichText::new("Focado apenas nos jogos ativos. Apps em 2º plano nunca sofrem trim.")
                                        .color(MUTED)
                                        .size(11.0),
                                );

                                ui.add_space(8.0);
                                ui.label(
                                    RichText::new("Intervalo de vigilância (s)")
                                        .color(MUTED)
                                        .size(12.0),
                                );
                                let interval_changed = ui.add(
                                    egui::Slider::new(&mut self.watch_interval, 15..=120)
                                        .clamping(egui::SliderClamping::Always),
                                ).changed();
                                if interval_changed {
                                    self.app_config.gui.watch_interval_secs = self.watch_interval;
                                    if self.watching {
                                        let _ = self.ui_tx.send(UiCmd::StartWatch { interval: self.watch_interval, trim_game: self.trim_game_memory });
                                    }
                                    self.save_app_config();
                                }

                                ui.add_space(6.0);
                                if ui
                                    .add_sized(
                                        [ui.available_width(), 34.0],
                                        egui::Button::new(
                                            RichText::new("⚙️ Gerenciar Jogos & Processos")
                                                .color(TEXT)
                                                .strong(),
                                        )
                                        .fill(BG_ROW)
                                        .stroke(Stroke::new(1.0, STROKE))
                                        .corner_radius(7.0),
                                    )
                                    .clicked()
                                {
                                    self.show_process_modal = true;
                                    self.process_tab = ProcessTab::RunningProcesses;
                                    self.refresh_running_processes();
                                }

                                ui.add_space(10.0);
                                section_label(ui, "TÚNEL WIREGUARD (VPS)");

                                if ui
                                    .add_sized(
                                        [ui.available_width(), 34.0],
                                        egui::Button::new(
                                            RichText::new("📥 Importar / Reimportar (.conf)")
                                                .color(Color32::from_rgb(28, 22, 12))
                                                .strong(),
                                        )
                                        .fill(ACCENT)
                                        .corner_radius(7.0),
                                    )
                                    .on_hover_text("Selecione um arquivo .conf do WireGuard para importar ou reimportar suas credenciais imediatamente")
                                    .clicked()
                                {
                                    self.import_conf_file();
                                }

                                ui.add_space(4.0);
                                if ui
                                    .add_sized(
                                        [ui.available_width(), 30.0],
                                        egui::Button::new(
                                            RichText::new("🛡️ Configurações do Túnel")
                                                .color(TEXT)
                                                .strong(),
                                        )
                                        .fill(BG_ROW)
                                        .stroke(Stroke::new(1.0, STROKE))
                                        .corner_radius(7.0),
                                    )
                                    .on_hover_text("Abra a janela de opções do túnel WireGuard, ping da VPS e rotas ativas")
                                    .clicked()
                                {
                                    self.show_tunnel_modal = true;
                                }

                                ui.add_space(10.0);
                                ui.separator();
                                ui.add_space(6.0);
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new("📁 Perfil:").color(MUTED).size(11.0));
                                    ui.label(
                                        RichText::new("%LOCALAPPDATA%\\GameOptimizer")
                                            .color(ACCENT)
                                            .size(11.0)
                                            .monospace(),
                                    );
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
                ui.add_space(8.0);
                ui.label(RichText::new("🎮").size(24.0));
                ui.add_space(4.0);
                ui.label(
                    RichText::new("Nenhum jogo detectado no momento.")
                        .color(TEXT)
                        .strong()
                        .size(13.0),
                );
                ui.label(
                    RichText::new("Inicie seu jogo ou escolha diretamente dos processos abertos.")
                        .color(MUTED)
                        .size(12.0),
                );
                ui.add_space(8.0);
                if ui
                    .add_sized(
                        [240.0, 32.0],
                        egui::Button::new(
                            RichText::new("🎯 Escolher dos Processos em Execução")
                                .strong()
                                .color(Color32::from_rgb(28, 22, 12)),
                        )
                        .fill(ACCENT)
                        .corner_radius(6.0),
                    )
                    .on_hover_text(
                        "Escolha diretamente da lista de processos abertos sem navegar pastas",
                    )
                    .clicked()
                {
                    self.show_process_modal = true;
                    self.process_tab = ProcessTab::RunningProcesses;
                    self.refresh_running_processes();
                }
                ui.add_space(8.0);
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
                    if ui
                        .small_button(
                            RichText::new("➕ Adicionar outro processo")
                                .color(TEXT)
                                .size(11.0),
                        )
                        .on_hover_text("Escolha outro processo aberto para monitorar")
                        .clicked()
                    {
                        self.show_process_modal = true;
                        self.process_tab = ProcessTab::RunningProcesses;
                        self.refresh_running_processes();
                    }
                });
            });
            ui.add_space(4.0);

            for game in &games {
                let inst_status = self
                    .macro_statuses
                    .iter()
                    .find(|s| s.pid == game.pid)
                    .cloned();
                let profile = self
                    .app_config
                    .macros
                    .get(&game.normalized_name)
                    .cloned()
                    .unwrap_or_default();
                let rules_count = inst_status
                    .as_ref()
                    .map(|s| s.rules.len())
                    .unwrap_or(profile.rules.len());
                let enabled_rules_count = inst_status
                    .as_ref()
                    .map(|s| s.rules.iter().filter(|r| r.enabled).count())
                    .unwrap_or_else(|| profile.enabled_rules_count());
                let is_macro_active = inst_status.as_ref().is_some_and(|s| s.active);
                let total_sent = inst_status.as_ref().map(|s| s.total_triggers).unwrap_or(0);
                let win_title = inst_status.as_ref().and_then(|s| s.window_title.clone());

                let card_stroke = if is_macro_active {
                    Stroke::new(1.2, OK.gamma_multiply(0.65))
                } else {
                    Stroke::new(1.0, STROKE)
                };

                egui::Frame::new()
                    .fill(BG_ROW)
                    .stroke(card_stroke)
                    .corner_radius(8.0)
                    .inner_margin(egui::Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("🎮").size(16.0));
                            let title_btn = ui
                                .add(
                                    egui::Button::new(
                                        RichText::new(&game.name)
                                            .color(TEXT)
                                            .strong()
                                            .size(15.0),
                                    )
                                    .frame(false),
                                )
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
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

                            if let Some(title) = &win_title {
                                if !title.eq_ignore_ascii_case(&game.name) {
                                    ui.label(
                                        RichText::new(format!("({title})"))
                                            .color(MUTED)
                                            .size(11.5),
                                    );
                                }
                            }

                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new(format_mib(game.memory_bytes))
                                            .color(ACCENT)
                                            .strong()
                                            .size(13.0),
                                    );
                                },
                            );
                        });

                        ui.add_space(4.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                RichText::new(format!("PID {}", game.pid))
                                    .color(MUTED)
                                    .size(11.0)
                                    .monospace(),
                            );
                            if let Some(status) = &self.tunnel_status {
                                match status.health {
                                    TunnelHealth::Healthy => {
                                        ui.label(
                                            RichText::new("• 🟢 Tunelado via VPS")
                                                .color(OK)
                                                .size(11.0)
                                                .strong(),
                                        );
                                    }
                                    TunnelHealth::Connecting => {
                                        ui.label(
                                            RichText::new("• 🟡 Conectando VPS (Handshake...)")
                                                .color(ACCENT)
                                                .size(11.0)
                                                .strong(),
                                        );
                                    }
                                    TunnelHealth::Degraded => {
                                        ui.label(
                                            RichText::new(
                                                "• 🛡️ Fallback direto ativo (evitando queda)",
                                            )
                                            .color(WARN)
                                            .size(11.0)
                                            .strong(),
                                        );
                                    }
                                    TunnelHealth::Idle => {}
                                }
                            }

                            if is_macro_active {
                                ui.label(
                                    RichText::new(format!(
                                        "• ⚡ {enabled_rules_count} Hotkey(s) ativa(s) ({total_sent} envios)"
                                    ))
                                    .color(OK)
                                    .size(11.0)
                                    .strong(),
                                );
                            } else if rules_count > 0 {
                                ui.label(
                                    RichText::new(format!(
                                        "• ⌨️ {rules_count} Hotkey(s) configurada(s) (Pausado)"
                                    ))
                                    .color(MUTED)
                                    .size(11.0),
                                );
                            }
                        });

                        ui.add_space(6.0);
                        ui.horizontal_wrapped(|ui| {
                            let cfg_label = if rules_count > 0 {
                                format!("⌨️ Hotkeys / Macros ({rules_count})")
                            } else {
                                "⌨️ Configurar Hotkeys (Opcional)".to_string()
                            };
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new(cfg_label)
                                            .color(TEXT)
                                            .size(11.5)
                                            .strong(),
                                    )
                                    .fill(BG_RAISED)
                                    .stroke(Stroke::new(1.0, STROKE))
                                    .corner_radius(6.0),
                                )
                                .on_hover_text(
                                    "Configurar envio autônomo de teclas e cliques para esta janela do jogo",
                                )
                                .clicked()
                            {
                                self.open_macro_modal_for_game(
                                    game.pid,
                                    &game.name,
                                    &game.normalized_name,
                                );
                            }

                            if rules_count > 0 {
                                if is_macro_active {
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new("⏸️ Pausar Janela")
                                                    .color(Color32::from_rgb(254, 202, 202))
                                                    .size(11.5)
                                                    .strong(),
                                            )
                                            .fill(Color32::from_rgb(88, 28, 28))
                                            .corner_radius(6.0),
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
                                } else if enabled_rules_count > 0
                                    && ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new("▶️ Iniciar nesta Janela")
                                                    .color(Color32::from_rgb(18, 14, 8))
                                                    .size(11.5)
                                                    .strong(),
                                            )
                                            .fill(ACCENT)
                                            .corner_radius(6.0),
                                        )
                                        .on_hover_text(
                                            "Iniciar o envio autônomo de hotkeys para esta janela do jogo",
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
                        });

                        // Show actively routed destinations if available
                        if let Some(status) = &self.tunnel_status {
                            if status.active && !status.routed_destinations.is_empty() {
                                ui.add_space(4.0);
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(
                                        RichText::new("📡 Servidores roteados:")
                                            .color(MUTED)
                                            .size(11.0),
                                    );
                                    for dest in status.routed_destinations.iter().take(3) {
                                        ui.label(
                                            RichText::new(dest.to_string())
                                                .color(TUNNEL_COLOR)
                                                .size(11.0)
                                                .monospace(),
                                        );
                                    }
                                });
                            }
                        }
                    });
                ui.add_space(6.0);
            }
        }
    }

    fn draw_tunnel_modal(&mut self, ctx: &egui::Context) {
        let mut open = self.show_tunnel_modal;
        egui::Window::new("🛡️ Configuração do Túnel WireGuard (VPS)")
            .open(&mut open)
            .default_width(560.0)
            .default_height(520.0)
            .resizable(true)
            .show(ctx, |ui| {
                egui::ScrollArea::both().id_salt(SCROLL_AREA_IDS[4]).max_height(ui.available_height()).show(ui, |ui| {
                ui.label(
                    RichText::new(
                        "Isolamento por PID: Apenas as conexões do jogo passam pela VPS. Chrome, Discord e o resto do PC continuam na sua internet normal.",
                    )
                    .color(MUTED)
                    .size(12.0),
                );
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    ui.checkbox(
                        &mut self.wireguard_cfg.enabled,
                        RichText::new("Ativar Tunelamento para Jogos Detectados")
                            .strong()
                            .color(TEXT),
                    );
                    if let Some(status) = &self.tunnel_status {
                        match status.health {
                            TunnelHealth::Healthy => {
                                if let Some(ms) = status.vps_ping_ms {
                                    ui.label(
                                        RichText::new(format!("🟢 Latência VPS: {ms}ms"))
                                            .color(OK)
                                            .strong(),
                                    );
                                } else {
                                    ui.label(
                                        RichText::new("🟢 Conectado e Saudável")
                                            .color(OK)
                                            .strong(),
                                    );
                                }
                            }
                            TunnelHealth::Connecting => {
                                ui.label(
                                    RichText::new("🟡 Aguardando Handshake...")
                                        .color(ACCENT)
                                        .strong(),
                                );
                            }
                            TunnelHealth::Degraded => {
                                ui.label(
                                    RichText::new("⚠️ Falha VPS - Fallback Ativo")
                                        .color(WARN)
                                        .strong(),
                                );
                            }
                            TunnelHealth::Idle => {}
                        }
                    }
                });

                if let Some(status) = &self.tunnel_status {
                    if status.health == TunnelHealth::Degraded && self.wireguard_cfg.enabled {
                        ui.add_space(6.0);
                        egui::Frame::new()
                            .fill(Color32::from_rgb(38, 22, 18))
                            .stroke(Stroke::new(1.0, Color32::from_rgb(180, 80, 60)))
                            .corner_radius(6.0)
                            .inner_margin(8.0)
                            .show(ui, |ui| {
                                ui.label(
                                    RichText::new("🛡️ Proteção Ativa contra Quedas: A VPS não respondeu ou a conexão falhou. As rotas dos jogos foram desviadas automaticamente para a sua internet local. Seu jogo continua funcionando diretamente sem cair! Assim que a VPS voltar a responder, o tunelamento otimizado será reativado.")
                                        .color(Color32::from_rgb(255, 200, 180))
                                        .size(11.0),
                                );
                            });
                    }
                }

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("📥 Importar / Reimportar (.conf)")
                                    .strong()
                                    .color(Color32::from_rgb(28, 22, 12)),
                            )
                            .fill(ACCENT)
                            .corner_radius(6.0),
                        )
                        .on_hover_text("Selecione um arquivo .conf do WireGuard para substituir as credenciais atuais")
                        .clicked()
                    {
                        self.import_conf_file();
                    }

                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(if self.show_paste_conf {
                                    "✖ Fechar Editor"
                                } else {
                                    "📋 Colar Configuração (.conf)"
                                })
                                .color(TEXT),
                            )
                            .fill(BG_ROW)
                            .stroke(Stroke::new(1.0, STROKE))
                            .corner_radius(6.0),
                        )
                        .on_hover_text("Abra um campo para colar o texto do arquivo .conf")
                        .clicked()
                    {
                        self.show_paste_conf = !self.show_paste_conf;
                    }

                    if ui
                        .add(
                            egui::Button::new(RichText::new("🔄 Recarregar do Perfil").color(MUTED))
                                .fill(BG_ROW)
                                .stroke(Stroke::new(1.0, STROKE))
                                .corner_radius(6.0),
                        )
                        .on_hover_text("Recarregar configurações salvas no perfil (%LOCALAPPDATA%\\GameOptimizer)")
                        .clicked()
                    {
                        self.reload_config_from_profile();
                    }
                });

                if self.show_paste_conf {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("Cole o conteúdo do seu arquivo WireGuard .conf abaixo:")
                            .color(MUTED)
                            .size(11.0),
                    );
                    ui.add(
                        egui::TextEdit::multiline(&mut self.paste_conf_text)
                            .hint_text("[Interface]\nPrivateKey = ...\nAddress = 10.8.0.2/24\n\n[Peer]\nPublicKey = ...\nEndpoint = 198.51.100.1:51820")
                            .desired_rows(6)
                            .desired_width(ui.available_width()),
                    );
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("Aplicar Texto Colado")
                                        .strong()
                                        .color(Color32::from_rgb(28, 22, 12)),
                                )
                                .fill(OK)
                                .corner_radius(6.0),
                            )
                            .clicked()
                        {
                            match WireGuardConfig::parse_conf(&self.paste_conf_text) {
                                Ok(parsed) => {
                                    self.wireguard_cfg = parsed;
                                    self.push_log("Configuração WireGuard colada e aplicada com sucesso.");
                                    self.commit_wireguard_config();
                                    self.show_paste_conf = false;
                                    self.paste_conf_text.clear();
                                }
                                Err(err) => {
                                    self.push_log(format!("Erro no formato colado: {err}"));
                                }
                            }
                        }
                        if ui.button("Cancelar").clicked() {
                            self.show_paste_conf = false;
                        }
                    });
                }

                ui.add_space(8.0);
                egui::Grid::new("wg_grid")
                    .num_columns(2)
                    .spacing([12.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("Endpoint VPS (IP:Porta):");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.wireguard_cfg.endpoint)
                                .hint_text("ex: 198.51.100.1:51820")
                                .desired_width(320.0),
                        );
                        ui.end_row();

                        ui.label("Chave Privada (Cliente):");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.wireguard_cfg.private_key)
                                .password(true)
                                .hint_text("Chave privada base64")
                                .desired_width(320.0),
                        );
                        ui.end_row();

                        ui.label("Chave Pública (VPS):");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.wireguard_cfg.peer_public_key)
                                .hint_text("Chave pública base64 da VPS")
                                .desired_width(320.0),
                        );
                        ui.end_row();

                        ui.label("IP do Cliente no Túnel:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.wireguard_cfg.tunnel_ip)
                                .hint_text("10.0.0.2")
                                .desired_width(320.0),
                        );
                        ui.end_row();

                        ui.label("Gateway da VPN (VPS):");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.wireguard_cfg.tunnel_gateway)
                                .hint_text("10.0.0.1")
                                .desired_width(320.0),
                        );
                        ui.end_row();

                        ui.label("DNS Opcional:");
                        let mut dns_str = self.wireguard_cfg.dns.clone().unwrap_or_default();
                        if ui
                            .add(
                                egui::TextEdit::singleline(&mut dns_str)
                                    .hint_text("1.1.1.1")
                                    .desired_width(320.0),
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

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(RichText::new("📡 Testar Ping VPS").color(TEXT))
                                .fill(BG_ROW)
                                .corner_radius(6.0),
                        )
                        .clicked()
                    {
                        let _ = self.ui_tx.send(UiCmd::TestVpsPing);
                    }

                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("💾 Salvar no Perfil (%LOCALAPPDATA%)")
                                    .strong()
                                    .color(Color32::from_rgb(28, 22, 12)),
                            )
                            .fill(ACCENT)
                            .corner_radius(6.0),
                        )
                        .clicked()
                    {
                        self.commit_wireguard_config();
                    }
                });

                if let Some(status) = &self.tunnel_status {
                    if !status.routed_destinations.is_empty() {
                        ui.add_space(8.0);
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
                ui.collapsing("ℹ️ Comandos necessários na VPS Linux", |ui| {
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
            });
        self.show_tunnel_modal = open;
    }

    fn draw_process_modal(&mut self, ctx: &egui::Context) {
        let mut open = self.show_process_modal;
        egui::Window::new("⚙️ Gerenciador de Processos, Jogos & Trim")
            .open(&mut open)
            .default_width(600.0)
            .default_height(420.0)
            .resizable(true)
            .show(ctx, |ui| {
                egui::ScrollArea::both().id_salt(SCROLL_AREA_IDS[5]).max_height(ui.available_height()).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .selectable_label(
                            self.process_tab == ProcessTab::RunningProcesses,
                            RichText::new("📋 Processos em Execução").strong(),
                        )
                        .clicked()
                    {
                        self.process_tab = ProcessTab::RunningProcesses;
                        self.refresh_running_processes();
                    }
                    if ui
                        .selectable_label(
                            self.process_tab == ProcessTab::Games,
                            RichText::new("🎮 Jogos Cadastrados").strong(),
                        )
                        .clicked()
                    {
                        self.process_tab = ProcessTab::Games;
                    }
                    if ui
                        .selectable_label(
                            self.process_tab == ProcessTab::MemoryTrim,
                            RichText::new("🧹 Memória & Regras de Trim").strong(),
                        )
                        .clicked()
                    {
                        self.process_tab = ProcessTab::MemoryTrim;
                    }
                });
                ui.separator();
                ui.add_space(6.0);

                match self.process_tab {
                    ProcessTab::RunningProcesses => {
                        ui.label(
                            RichText::new(
                                "Escolha diretamente qualquer processo aberto no seu PC.\n\
                                O Game Optimizer detecta automaticamente o processo e o PID raiz para priorização, memória e tunelamento WireGuard.",
                            )
                            .color(MUTED)
                            .size(12.0),
                        );
                        ui.add_space(8.0);

                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.process_search_filter)
                                    .hint_text("🔍 Filtrar por nome ou PID...")
                                    .desired_width(260.0),
                            );
                            if ui
                                .add(
                                    egui::Button::new(RichText::new("🔄 Atualizar Lista").color(TEXT))
                                        .fill(BG_ROW)
                                        .corner_radius(6.0),
                                )
                                .on_hover_text("Atualiza a lista de processos ativos")
                                .clicked()
                            {
                                self.refresh_running_processes();
                            }

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(
                                    RichText::new(format!("{} processos ativos", self.running_processes.len()))
                                        .color(MUTED)
                                        .size(11.0),
                                );
                            });
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
                            ui.add_space(30.0);
                            ui.vertical_centered(|ui| {
                                ui.label(RichText::new("Nenhum processo encontrado com este filtro.").color(MUTED));
                            });
                        } else {
                        let list = egui::ScrollArea::vertical().id_salt(SCROLL_AREA_IDS[6])
                                .max_height(340.0)
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    for proc in &filtered_procs {
                                        egui::Frame::new()
                                            .fill(BG_ROW)
                                            .stroke(Stroke::new(1.0, STROKE))
                                            .corner_radius(6.0)
                                            .inner_margin(egui::Margin::symmetric(10, 8))
                                            .show(ui, |ui| {
                                                ui.horizontal(|ui| {
                                                    let icon = if proc.is_game { "🎮" } else { "⚙️" };
                                                    ui.label(RichText::new(icon).size(16.0));

                                                    ui.vertical(|ui| {
                                                        ui.horizontal(|ui| {
                                                            ui.label(
                                                                RichText::new(&proc.name)
                                                                    .color(TEXT)
                                                                    .strong()
                                                                    .size(13.0),
                                                            );
                                                            ui.label(
                                                                RichText::new(format!("PID {}", proc.pid))
                                                                    .color(MUTED)
                                                                    .size(11.0)
                                                                    .monospace(),
                                                            );
                                                        });

                                                        if proc.root_pid != proc.pid || proc.root_name != proc.normalized_name {
                                                            ui.horizontal(|ui| {
                                                                ui.label(
                                                                    RichText::new(format!(
                                                                        "↳ Raiz detectada: {}.exe (PID {})",
                                                                        proc.root_name, proc.root_pid
                                                                    ))
                                                                    .color(ACCENT)
                                                                    .size(11.0),
                                                                );
                                                            });
                                                        }
                                                    });

                                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                        ui.label(
                                                            RichText::new(format_mib(proc.memory_bytes))
                                                                .color(ACCENT)
                                                                .strong()
                                                                .size(12.0),
                                                        );

                                                        ui.add_space(8.0);

                                                        // Game monitoring button/badge
                                                        if proc.is_game {
                                                            egui::Frame::new()
                                                                .fill(Color32::from_rgba_unmultiplied(34, 197, 94, 25))
                                                                .corner_radius(4.0)
                                                                .inner_margin(egui::Margin::symmetric(6, 3))
                                                                .show(ui, |ui| {
                                                                    ui.label(RichText::new("🎮 Jogo").color(OK).size(11.0).strong());
                                                                });

                                                            if ui.small_button("❌").on_hover_text("Remover jogo do monitoramento").clicked() {
                                                                self.remove_process_as_game(proc);
                                                            }
                                                        } else if ui
                                                            .add(
                                                                egui::Button::new(
                                                                    RichText::new("🎮 Monitorar Jogo")
                                                                        .color(Color32::from_rgb(28, 22, 12))
                                                                        .strong()
                                                                        .size(11.0),
                                                                )
                                                                .fill(ACCENT)
                                                                .corner_radius(4.0),
                                                            )
                                                            .on_hover_text("Detecta o processo e sua raiz (prioridade alta, WireGuard, P-cores)")
                                                            .clicked()
                                                        {
                                                            self.add_process_as_game(proc);
                                                        }

                                                        // Trim button
                                                        let is_reclaim = self.app_config.reclaim_names.contains(&proc.normalized_name);
                                                        if is_reclaim {
                                                            if ui
                                                                .small_button(RichText::new("🧹 Trim ✓").color(OK).size(11.0))
                                                                .on_hover_text("Clique para remover do Trim")
                                                                .clicked()
                                                            {
                                                                self.app_config.remove_reclaim_name(&proc.normalized_name);
                                                                self.save_app_config();
                                                                self.refresh_running_processes();
                                                            }
                                                        } else if !proc.is_game
                                                            && ui
                                                                .small_button(RichText::new("🧹 Trim").color(MUTED).size(11.0))
                                                                .on_hover_text("Adicionar ao trim de memória de segundo plano")
                                                                .clicked()
                                                        {
                                                            self.app_config.add_reclaim_name(&proc.normalized_name);
                                                            self.save_app_config();
                                                            self.refresh_running_processes();
                                                        }

                                                        // Protect button
                                                        let is_protected = self.app_config.never_trim_names.contains(&proc.normalized_name);
                                                        if is_protected {
                                                            if ui
                                                                .small_button(RichText::new("🛡️ Protegido ✓").color(OK).size(11.0))
                                                                .on_hover_text("Clique para desproteger")
                                                                .clicked()
                                                            {
                                                                self.app_config.remove_never_trim_name(&proc.normalized_name);
                                                                self.save_app_config();
                                                                self.refresh_running_processes();
                                                            }
                                                        } else if !proc.is_game
                                                            && ui
                                                                .small_button(RichText::new("🛡️ Proteger").color(MUTED).size(11.0))
                                                                .on_hover_text("Proteger este processo contra trim")
                                                                .clicked()
                                                        {
                                                            self.app_config.add_never_trim_name(&proc.normalized_name);
                                                            self.save_app_config();
                                                            self.refresh_running_processes();
                                                        }
                                                    });
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
                            RichText::new(
                                "Processos identificados como jogos recebem prioridade alta, P-Cores exclusivos e tunelamento WireGuard automático.",
                            )
                            .color(MUTED)
                            .size(12.0),
                        );
                        ui.add_space(8.0);

                        ui.horizontal(|ui| {
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("📋 Escolher dos Processos em Execução")
                                            .strong()
                                            .color(Color32::from_rgb(28, 22, 12)),
                                    )
                                    .fill(ACCENT)
                                    .corner_radius(6.0),
                                )
                                .on_hover_text("Escolha diretamente da lista de processos abertos no sistema")
                                .clicked()
                            {
                                self.process_tab = ProcessTab::RunningProcesses;
                                self.refresh_running_processes();
                            }

                            ui.add_space(4.0);

                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("📁 Procurar .exe")
                                            .strong()
                                            .color(TEXT),
                                    )
                                    .fill(BG_ROW)
                                    .corner_radius(6.0),
                                )
                                .on_hover_text("Abra o Explorer para selecionar o executável (.exe) do jogo instalado")
                                .clicked()
                            {
                                self.pick_game_exe();
                            }

                            ui.add_space(4.0);
                            ui.label(RichText::new("ou").color(MUTED).size(12.0));
                            ui.add_space(4.0);

                            ui.add(
                                egui::TextEdit::singleline(&mut self.new_game_input)
                                    .hint_text("Nome (ex: mir4)")
                                    .desired_width(140.0),
                            );
                            if ui
                                .add(
                                    egui::Button::new(RichText::new("➕ Adicionar").color(TEXT))
                                        .fill(BG_ROW)
                                        .corner_radius(6.0),
                                )
                                .clicked()
                            {
                                let name = self.new_game_input.trim().to_string();
                                if !name.is_empty() && self.app_config.add_game_name(&name) {
                                    self.push_log(format!("Jogo adicionado: {name}"));
                                    self.save_app_config();
                                    self.new_game_input.clear();
                                    let _ = self.ui_tx.send(UiCmd::Scan);
                                }
                            }
                        });

                        ui.add_space(10.0);
                        section_label(ui, "JOGOS PERSONALIZADOS DO USUÁRIO");

                        let custom_games = self.app_config.user_custom_games();
                        if custom_games.is_empty() {
                            ui.label(
                                RichText::new(
                                    "Nenhum jogo personalizado cadastrado ainda. Use a aba 'Processos em Execução' acima para selecionar jogos abertos.",
                                )
                                .color(MUTED)
                                .size(11.0),
                            );
                        } else {
                            egui::ScrollArea::vertical().id_salt(SCROLL_AREA_IDS[7]).max_height(130.0).show(ui, |ui| {
                                for game in &custom_games {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new("🎮").size(14.0));
                                        ui.label(
                                            RichText::new(format!("{game}.exe"))
                                                .color(TEXT)
                                                .strong()
                                                .monospace(),
                                        );
                                        if ui
                                            .add(
                                                egui::Button::new(
                                                    RichText::new("❌").color(WARN).size(11.0),
                                                )
                                                .fill(BG_ROW)
                                                .corner_radius(4.0),
                                            )
                                            .on_hover_text("Remover jogo da lista")
                                            .clicked()
                                        {
                                            self.app_config.remove_game_name(game);
                                            self.push_log(format!("Jogo removido: {game}"));
                                            self.save_app_config();
                                            let _ = self.ui_tx.send(UiCmd::Scan);
                                        }
                                    });
                                }
                            });
                        }

                        ui.add_space(10.0);
                        ui.collapsing("ℹ️ Lista de Jogos Nativos Pré-configurados", |ui| {
                            ui.label(
                                RichText::new(
                                    "O Game Optimizer reconhece nativamente os executáveis abaixo sem precisar cadastrá-los:",
                                )
                                .color(MUTED)
                                .size(11.0),
                            );
                            ui.add_space(4.0);
                            egui::ScrollArea::vertical().id_salt(SCROLL_AREA_IDS[8]).max_height(100.0).show(ui, |ui| {
                                ui.horizontal_wrapped(|ui| {
                                    for name in crate::config::GameCatalog::builtin_names() {
                                        ui.label(
                                            RichText::new(format!("{name}.exe"))
                                                .color(MUTED)
                                                .size(11.0)
                                                .monospace(),
                                        );
                                    }
                                });
                            });
                        });
                    }

                    ProcessTab::MemoryTrim => {
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
                                RichText::new("Trim de memória nos Jogos (libera páginas frias do próprio jogo)")
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
                                RichText::new("Trim de memória em Apps de Segundo Plano (Browsers e apps pesados)")
                                    .color(TEXT)
                                    .size(12.0),
                            )
                            .changed()
                        {
                            changed = true;
                        }

                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            let mut mb = (self.app_config.memory_trim_threshold_bytes / (1024 * 1024)) as u32;
                            ui.label(RichText::new("Consumo mínimo para liberar:").color(MUTED).size(12.0));
                            if ui.add(egui::Slider::new(&mut mb, 100..=1024).suffix(" MB")).changed() {
                                self.app_config.memory_trim_threshold_bytes = (mb as u64) * 1024 * 1024;
                                changed = true;
                            }
                        });

                        if changed {
                            self.save_app_config();
                        }

                        ui.add_space(10.0);
                        section_label(ui, "APLICATIVOS ALVO DE TRIM (SEGUNDO PLANO)");
                        ui.label(
                            RichText::new("Apenas estes programas terão memória liberada quando o trim de 2º plano estiver ativo:")
                                .color(MUTED)
                                .size(11.0),
                        );

                        let reclaim_list: Vec<String> =
                            self.app_config.reclaim_names.iter().cloned().collect();
                        egui::ScrollArea::vertical().id_salt(SCROLL_AREA_IDS[9]).max_height(88.0).show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            for app in &reclaim_list {
                                egui::Frame::new()
                                    .fill(BG_ROW)
                                    .corner_radius(6.0)
                                    .inner_margin(egui::Margin::symmetric(6, 3))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(RichText::new(app).color(TEXT).size(11.0));
                                            if ui.small_button("❌").clicked() {
                                                self.app_config.remove_reclaim_name(app);
                                                self.save_app_config();
                                            }
                                        });
                                    });
                            }
                        });
                        });

                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.new_reclaim_input)
                                    .hint_text("Nome do app (ex: opera)")
                                    .desired_width(160.0),
                            );
                            if ui.button("➕ Adicionar para Trim").clicked() {
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
                            RichText::new("Estes processos NUNCA sofrerão esvaziamento de memória (Discord, OBS, gravadores, IDEs):")
                                .color(MUTED)
                                .size(11.0),
                        );

                        let never_trim_list: Vec<String> =
                            self.app_config.never_trim_names.iter().cloned().collect();
                        egui::ScrollArea::vertical().id_salt(SCROLL_AREA_IDS[10]).max_height(88.0).show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            for app in &never_trim_list {
                                egui::Frame::new()
                                    .fill(BG_ROW)
                                    .corner_radius(6.0)
                                    .inner_margin(egui::Margin::symmetric(6, 3))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(RichText::new(app).color(OK).size(11.0));
                                            if ui.small_button("❌").clicked() {
                                                self.app_config.remove_never_trim_name(app);
                                                self.save_app_config();
                                            }
                                        });
                                    });
                            }
                        });
                        });

                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.new_never_trim_input)
                                    .hint_text("Nome protegido (ex: obs64)")
                                    .desired_width(160.0),
                            );
                            if ui.button("➕ Proteger App").clicked() {
                                let name = self.new_never_trim_input.trim().to_string();
                                if !name.is_empty() {
                                    self.app_config.add_never_trim_name(&name);
                                    self.save_app_config();
                                    self.new_never_trim_input.clear();
                                }
                            }
                        });
                    }
                }
                });
            });
        self.show_process_modal = open;
    }

    fn open_macro_modal_for_game(&mut self, pid: u32, name: &str, normalized_name: &str) {
        self.macro_target_pid = Some(pid);
        self.macro_target_name = name.to_string();
        self.macro_target_normalized = normalized_name.to_string();
        self.capturing_macro_key_for = None;
        self.show_macro_modal = true;
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
        let entry = self
            .app_config
            .macros
            .entry(normalized_name.to_string())
            .or_default();
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

    fn draw_macro_modal(&mut self, ctx: &egui::Context) {
        let Some(target_pid) = self.macro_target_pid else {
            self.show_macro_modal = false;
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

        let mut profile: GameMacroProfile = self
            .app_config
            .macros
            .get(&norm_name)
            .cloned()
            .unwrap_or_default()
            .normalized();
        if let Some(st) = &inst_status {
            profile.mode = st.mode;
            if st.cycle_interval_ms > 0 {
                profile.cycle_interval_ms = st.cycle_interval_ms;
            }
            if !st.rules.is_empty() || profile.rules.is_empty() {
                profile.rules = st.rules.clone();
            }
        }

        // Handle continuous sequence recording or single-step / toggle-key capture
        if self.recording_macro_sequence {
            let mut captured_codes: Vec<&'static str> = Vec::new();
            let mut stop_recording = false;
            ctx.input(|input| {
                if input.key_pressed(egui::Key::Escape) {
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
            ctx.input(|input| {
                if input.key_pressed(egui::Key::Escape) {
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

        let mut open = self.show_macro_modal;
        egui::Window::new(format!("⌨️ Automação de Teclas (Macros) — {display_name}"))
            .open(&mut open)
            .default_width(650.0)
            .min_width(520.0)
            .resizable(true)
            .collapsible(false)
            .frame(
                egui::Frame::new()
                    .fill(BG)
                    .stroke(Stroke::new(1.5, ACCENT))
                    .corner_radius(12.0)
                    .inner_margin(16.0),
            )
            .show(ctx, |ui| {
                let max_h = (ctx.content_rect().height() - 90.0).max(280.0);
                egui::ScrollArea::vertical()
                    .id_salt(SCROLL_AREA_IDS[11])
                    .max_height(max_h)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        // 1. Multi-window switcher if multiple game windows are open
                        if running_games.len() > 1 {
                            section_label(ui, "JANELA SELECIONADA");
                            ui.horizontal_wrapped(|ui| {
                                for g in &running_games {
                                    let selected = g.pid == target_pid;
                                    let g_active = self
                                        .macro_statuses
                                        .iter()
                                        .any(|s| s.pid == g.pid && s.active);
                                    let status_dot = if g_active { "●" } else { "○" };
                                    let fill = if selected { ACCENT_SOFT } else { BG_RAISED };
                                    let stroke = if selected {
                                        Stroke::new(1.2, ACCENT)
                                    } else {
                                        Stroke::new(1.0, STROKE)
                                    };
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new(format!(
                                                    "{status_dot} {} (PID {})",
                                                    g.name, g.pid
                                                ))
                                                .color(if g_active {
                                                    OK
                                                } else if selected {
                                                    ACCENT
                                                } else {
                                                    TEXT
                                                })
                                                .size(11.5)
                                                .strong(),
                                            )
                                            .fill(fill)
                                            .stroke(stroke)
                                            .corner_radius(6.0),
                                        )
                                        .clicked()
                                    {
                                        self.macro_target_pid = Some(g.pid);
                                        self.macro_target_name = g.name.clone();
                                        self.macro_target_normalized = g.normalized_name.clone();
                                        self.capturing_macro_key_for = None;
                                    }
                                }
                            });
                            ui.add_space(8.0);
                        }

                        // 2. Top Control Card (Status + Play/Pause Button cleanly separated)
                        egui::Frame::new()
                            .fill(BG_RAISED)
                            .stroke(Stroke::new(
                                1.2,
                                if is_active { OK } else { STROKE },
                            ))
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
                                    ui.label(
                                        RichText::new("●")
                                            .color(dot_col)
                                            .size(16.0)
                                            .strong(),
                                    );
                                    ui.vertical(|ui| {
                                        ui.label(
                                            RichText::new(status_title)
                                                .color(dot_col)
                                                .strong()
                                                .size(13.5),
                                        );
                                        let win_desc = match &window_title {
                                            Some(t) => format!(
                                                "Janela: \"{t}\" (PID {target_pid})  •  Ações enviadas: {total_sent}"
                                            ),
                                            None => format!(
                                                "Processo: {display_name} (PID {target_pid})  •  Ações enviadas: {total_sent}"
                                            ),
                                        };
                                        ui.label(
                                            RichText::new(win_desc)
                                                .color(TEXT)
                                                .size(12.0),
                                        );
                                    });

                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            let can_start = is_running
                                                && profile.rules.iter().any(MacroRule::is_ready);
                                            if is_active {
                                                if ui
                                                    .add_sized(
                                                        [180.0, 38.0],
                                                        egui::Button::new(
                                                            RichText::new("⏸  Pausar Automação")
                                                                .color(Color32::from_rgb(
                                                                    254, 202, 202,
                                                                ))
                                                                .strong()
                                                                .size(13.0),
                                                        )
                                                        .fill(Color32::from_rgb(95, 28, 28))
                                                        .corner_radius(8.0),
                                                    )
                                                    .clicked()
                                                {
                                                    self.set_game_macro_active(
                                                        target_pid, &norm_name, false,
                                                    );
                                                }
                                            } else {
                                                let btn = ui
                                                    .add_enabled_ui(can_start, |ui| {
                                                        ui.add_sized(
                                                            [180.0, 38.0],
                                                            egui::Button::new(
                                                                RichText::new(
                                                                    "▶  Iniciar Automação",
                                                                )
                                                                .color(Color32::from_rgb(
                                                                    18, 14, 8,
                                                                ))
                                                                .strong()
                                                                .size(13.0),
                                                            )
                                                            .fill(ACCENT)
                                                            .corner_radius(8.0),
                                                        )
                                                    })
                                                    .inner;
                                                if btn
                                                    .on_disabled_hover_text(
                                                        "Adicione pelo menos 1 passo abaixo para iniciar",
                                                    )
                                                    .clicked()
                                                {
                                                    self.set_game_macro_active(
                                                        target_pid, &norm_name, true,
                                                    );
                                                }
                                            }
                                        },
                                    );
                                });

                                ui.add_space(6.0);
                                ui.separator();
                                ui.add_space(4.0);

                                // Execution Mode selector (Sequential vs Parallel)
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(
                                        RichText::new("Modo de reprodução:")
                                            .color(MUTED)
                                            .size(12.0)
                                            .strong(),
                                    );
                                    let is_seq = profile.mode == MacroExecutionMode::Sequential;
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new(
                                                    "1 → 2 → 3  Em Sequência (Uma após a outra)",
                                                )
                                                .color(if is_seq { ACCENT } else { TEXT })
                                                .size(11.5)
                                                .strong(),
                                            )
                                            .fill(if is_seq { ACCENT_SOFT } else { BG_ROW })
                                            .stroke(Stroke::new(
                                                1.0,
                                                if is_seq { ACCENT } else { STROKE },
                                            ))
                                            .corner_radius(6.0),
                                        )
                                        .on_hover_text(
                                            "Executa a sequência de teclas em ordem (ex: 50ms entre elas) e aguarda o tempo de disparo configurado para repetir a sequência",
                                        )
                                        .clicked()
                                        && !is_seq
                                    {
                                        self.update_game_macro_rules(
                                            target_pid,
                                            &norm_name,
                                            MacroExecutionMode::Sequential,
                                            profile.cycle_interval_ms,
                                            profile.toggle_key.clone(),
                                            profile.rules.clone(),
                                        );
                                    }

                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new(
                                                    "⚡ Simultâneo (Timers independentes)",
                                                )
                                                .color(if !is_seq { ACCENT } else { TEXT })
                                                .size(11.5)
                                                .strong(),
                                            )
                                            .fill(if !is_seq { ACCENT_SOFT } else { BG_ROW })
                                            .stroke(Stroke::new(
                                                1.0,
                                                if !is_seq { ACCENT } else { STROKE },
                                            ))
                                            .corner_radius(6.0),
                                        )
                                        .on_hover_text(
                                            "Cada tecla tem seu próprio cronômetro separado e dispara independentemente das outras",
                                        )
                                        .clicked()
                                        && is_seq
                                    {
                                        self.update_game_macro_rules(
                                            target_pid,
                                            &norm_name,
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
                                                    .range(
                                                        MIN_MACRO_INTERVAL_MS
                                                            ..=MAX_MACRO_INTERVAL_MS,
                                                    )
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
                                                &norm_name,
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
                                            if ui
                                                .add(
                                                    egui::Button::new(
                                                        RichText::new(lbl)
                                                            .color(if active_preset {
                                                                ACCENT
                                                            } else {
                                                                MUTED
                                                            })
                                                            .size(11.0)
                                                            .strong(),
                                                    )
                                                    .fill(if active_preset {
                                                        ACCENT_SOFT
                                                    } else {
                                                        BG_ROW
                                                    })
                                                    .stroke(Stroke::new(
                                                        1.0,
                                                        if active_preset { ACCENT } else { STROKE },
                                                    ))
                                                    .corner_radius(5.0),
                                                )
                                                .on_hover_text(format!(
                                                    "Repetir a sequência inteira a cada {lbl} ({ms} ms)"
                                                ))
                                                .clicked()
                                            {
                                                profile.cycle_interval_ms = ms;
                                                self.update_game_macro_rules(
                                                    target_pid,
                                                    &norm_name,
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
                                        RichText::new(
                                            "Tecla para Parar / Iniciar dentro da janela do jogo:",
                                        )
                                        .color(TEXT)
                                        .size(12.0)
                                        .strong(),
                                    );
                                    let prev_toggle = profile.toggle_key.clone();
                                    let toggle_display = if profile.toggle_key.is_empty() {
                                        "Desativado".to_string()
                                    } else {
                                        format!("⌨️ {}", key_display_label(&profile.toggle_key))
                                    };
                                    egui::ComboBox::from_id_salt("macro-window-toggle-key")
                                        .selected_text(
                                            RichText::new(toggle_display)
                                                .color(if profile.toggle_key.is_empty() {
                                                    MUTED
                                                } else {
                                                    ACCENT
                                                })
                                                .strong()
                                                .size(12.0),
                                        )
                                        .width(140.0)
                                        .show_ui(ui, |ui| {
                                            ui.selectable_value(
                                                &mut profile.toggle_key,
                                                String::new(),
                                                "Desativado",
                                            );
                                            ui.separator();
                                            let mut last_cat = "";
                                            for spec in SUPPORTED_KEYS {
                                                if spec.vk == 0 {
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
                                                ui.selectable_value(
                                                    &mut profile.toggle_key,
                                                    spec.code.to_string(),
                                                    spec.label,
                                                );
                                            }
                                        });
                                    if profile.toggle_key != prev_toggle {
                                        self.update_game_macro_rules(
                                            target_pid,
                                            &norm_name,
                                            profile.mode,
                                            profile.cycle_interval_ms,
                                            profile.toggle_key.clone(),
                                            profile.rules.clone(),
                                        );
                                    }

                                    let is_cap_toggle = self.capturing_macro_key_for == Some(None);
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new(if is_cap_toggle {
                                                    "Aperte a tecla... (Esc)"
                                                } else {
                                                    "Capturar"
                                                })
                                                .color(if is_cap_toggle { ACCENT } else { MUTED })
                                                .size(11.0),
                                            )
                                            .fill(if is_cap_toggle {
                                                ACCENT_SOFT
                                            } else {
                                                BG_ROW
                                            })
                                            .stroke(Stroke::new(
                                                1.0,
                                                if is_cap_toggle { ACCENT } else { STROKE },
                                            ))
                                            .corner_radius(5.0),
                                        )
                                        .on_hover_text(
                                            "Clique e aperte a tecla que deseja usar dentro da janela do jogo para ligar/desligar o macro naquela janela",
                                        )
                                        .clicked()
                                    {
                                        self.recording_macro_sequence = false;
                                        self.capturing_macro_key_for = if is_cap_toggle {
                                            None
                                        } else {
                                            Some(None)
                                        };
                                    }

                                    ui.label(
                                        RichText::new(if profile.toggle_key.is_empty() {
                                            "(Atalho desativado)".to_string()
                                        } else {
                                            format!(
                                                "• Ao apertar {} dentro de uma janela do jogo, para/inicia APENAS naquela janela.",
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
                                        "Como funciona: Executa os passos na ordem (ex: 50 ms entre uma tecla e outra) e, ao terminar o último passo, aguarda o tempo de disparo acima (ex: 5s ou 10s) antes de repetir."
                                    }
                                    MacroExecutionMode::Parallel => {
                                        "Como funciona: Cada tecla da lista roda no seu próprio intervalo de tempo de forma independente."
                                    }
                                };
                                ui.label(RichText::new(mode_hint).color(MUTED).size(11.0));

                                ui.add_space(4.0);
                                let mut auto_start = profile.auto_start;
                                if ui
                                    .checkbox(
                                        &mut auto_start,
                                        RichText::new(
                                            "Iniciar automaticamente sempre que este jogo for aberto (Opcional)",
                                        )
                                        .color(MUTED)
                                        .size(11.5),
                                    )
                                    .changed()
                                {
                                    self.app_config
                                        .macros
                                        .entry(norm_name.clone())
                                        .or_default()
                                        .auto_start = auto_start;
                                    self.save_app_config();
                                }
                            });

                        ui.add_space(12.0);

                        // 3. Toolbar to add steps into the sequence (no hardcoded keys or preset timers)
                        ui.horizontal_wrapped(|ui| {
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("+  Adicionar Passo")
                                            .color(Color32::from_rgb(18, 14, 8))
                                            .strong()
                                            .size(12.5),
                                    )
                                    .fill(ACCENT)
                                    .corner_radius(7.0),
                                )
                                .on_hover_text(
                                    "Adiciona um novo passo na sequência e aguarda você pressionar ou selecionar a tecla",
                                )
                                .clicked()
                            {
                                self.recording_macro_sequence = false;
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
                                    &norm_name,
                                    profile.mode,
                                    profile.cycle_interval_ms,
                                    profile.toggle_key.clone(),
                                    updated,
                                );
                            }

                            let rec_active = self.recording_macro_sequence;
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new(if rec_active {
                                            "Parar Gravação (Esc)"
                                        } else {
                                            "●  Gravar Sequência pelo Teclado"
                                        })
                                        .color(if rec_active { ACCENT } else { TEXT })
                                        .strong()
                                        .size(12.0),
                                    )
                                    .fill(if rec_active { ACCENT_SOFT } else { BG_RAISED })
                                    .stroke(Stroke::new(
                                        1.0,
                                        if rec_active { ACCENT } else { STROKE },
                                    ))
                                    .corner_radius(7.0),
                                )
                                .on_hover_text(
                                    "Clique e vá apertando as teclas no teclado na ordem desejada; cada tecla será adicionada uma após a outra com o intervalo padrão ao lado",
                                )
                                .clicked()
                            {
                                self.recording_macro_sequence = !rec_active;
                                self.capturing_macro_key_for = None;
                            }

                            ui.separator();

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
                                "Intervalo padrão entre uma tecla e a próxima da sequência (ex: 50 ms para executar uma tecla 50ms atrás da outra)",
                            );

                            if !profile.rules.is_empty()
                                && ui
                                    .small_button(
                                        RichText::new(format!(
                                            "Aplicar {} ms a todos",
                                            self.default_macro_delay_ms
                                        ))
                                        .color(TEXT)
                                        .size(11.0),
                                    )
                                    .on_hover_text(
                                        "Define este intervalo entre teclas em todos os passos da sequência abaixo",
                                    )
                                    .clicked()
                            {
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
                                    &norm_name,
                                    profile.mode,
                                    profile.cycle_interval_ms,
                                    profile.toggle_key.clone(),
                                    updated,
                                );
                            }

                            if !profile.rules.is_empty() {
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui
                                            .small_button(
                                                RichText::new("Limpar Tudo")
                                                    .color(MUTED)
                                                    .size(11.0),
                                            )
                                            .clicked()
                                        {
                                            self.recording_macro_sequence = false;
                                            self.capturing_macro_key_for = None;
                                            self.set_game_macro_active(
                                                target_pid, &norm_name, false,
                                            );
                                            self.update_game_macro_rules(
                                                target_pid,
                                                &norm_name,
                                                profile.mode,
                                                profile.cycle_interval_ms,
                                                profile.toggle_key.clone(),
                                                Vec::new(),
                                            );
                                        }
                                    },
                                );
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
                                            RichText::new(
                                                "Nenhum passo configurado ainda.",
                                            )
                                            .color(TEXT)
                                            .strong()
                                            .size(13.5),
                                        );
                                        ui.add_space(4.0);
                                        ui.label(
                                            RichText::new(
                                                "Use '+ Adicionar Passo' ou '● Gravar Sequência pelo Teclado' acima para definir suas teclas na ordem desejada (padrão: 50 ms entre cada tecla).",
                                            )
                                            .color(MUTED)
                                            .size(12.0),
                                        );
                                    });
                                });
                        } else {
                            let mut rules_mut = profile.rules.clone();
                            let total_rules = rules_mut.len();
                            let mut rules_changed = false;
                            let mut remove_idx: Option<usize> = None;
                            let mut swap_pair: Option<(usize, usize)> = None;

                            let list = egui::ScrollArea::vertical()
                                .id_salt(SCROLL_AREA_IDS[12])
                                .max_height(290.0)
                                .auto_shrink([false; 2])
                                .show(ui, |ui| {
                                    for (idx, rule) in rules_mut.iter_mut().enumerate() {
                                        let trig =
                                            rule_triggers.get(idx).copied().unwrap_or(0);
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
                                                // Top line of step card: Step badge + Key selector + Delay + Actions
                                                ui.horizontal_wrapped(|ui| {
                                                    if ui
                                                        .checkbox(&mut rule.enabled, "")
                                                        .on_hover_text(
                                                            "Ativar ou desativar este passo",
                                                        )
                                                        .changed()
                                                    {
                                                        rules_changed = true;
                                                    }

                                                    // Step number pill
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
                                                        .inner_margin(egui::Margin::symmetric(
                                                            7, 3,
                                                        ))
                                                        .show(ui, |ui| {
                                                            ui.label(
                                                                RichText::new(step_badge)
                                                                    .color(if is_current_step {
                                                                        OK
                                                                    } else {
                                                                        ACCENT
                                                                    })
                                                                    .size(11.0)
                                                                    .strong()
                                                                    .monospace(),
                                                            );
                                                        });

                                                    // Key dropdown
                                                    let prev_key = rule.key.clone();
                                                    egui::ComboBox::from_id_salt((
                                                        "rule-key-combo",
                                                        idx,
                                                    ))
                                                    .selected_text(
                                                        RichText::new(format!(
                                                            "⌨️ {}",
                                                            key_display_label(&rule.key)
                                                        ))
                                                        .color(TEXT)
                                                        .strong()
                                                        .size(12.5),
                                                    )
                                                    .width(165.0)
                                                    .show_ui(ui, |ui| {
                                                        let mut last_cat = "";
                                                        for spec in SUPPORTED_KEYS {
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
                                                            ui.selectable_value(
                                                                &mut rule.key,
                                                                spec.code.to_string(),
                                                                spec.label,
                                                            );
                                                        }
                                                    });
                                                    if rule.key != prev_key {
                                                        rules_changed = true;
                                                    }

                                                    // Key capture button for this step
                                                    let is_cap_row = self.capturing_macro_key_for
                                                        == Some(Some(idx));
                                                    if ui
                                                        .add(
                                                            egui::Button::new(
                                                                RichText::new(if is_cap_row {
                                                                    "Aperte a tecla... (Esc)"
                                                                } else {
                                                                    "Capturar"
                                                                })
                                                                .color(if is_cap_row {
                                                                    ACCENT
                                                                } else {
                                                                    MUTED
                                                                })
                                                                .size(11.0),
                                                            )
                                                            .fill(if is_cap_row {
                                                                ACCENT_SOFT
                                                            } else {
                                                                BG_RAISED
                                                            })
                                                            .stroke(Stroke::new(
                                                                1.0,
                                                                if is_cap_row {
                                                                    ACCENT
                                                                } else {
                                                                    STROKE
                                                                },
                                                            ))
                                                            .corner_radius(5.0),
                                                        )
                                                        .on_hover_text(
                                                            "Clique aqui e aperte uma tecla do teclado para definir neste passo",
                                                        )
                                                        .clicked()
                                                    {
                                                        self.capturing_macro_key_for =
                                                            if is_cap_row {
                                                                None
                                                            } else {
                                                                Some(Some(idx))
                                                            };
                                                    }

                                                    // Wait / Interval control
                                                    let is_last_seq_step = profile.mode
                                                        == MacroExecutionMode::Sequential
                                                        && idx + 1 == total_rules;
                                                    if is_last_seq_step {
                                                        ui.label(
                                                            RichText::new(
                                                                "→ Última tecla da sequência",
                                                            )
                                                            .color(MUTED)
                                                            .size(11.5),
                                                        );
                                                    } else {
                                                        let wait_label = match profile.mode {
                                                            MacroExecutionMode::Sequential => {
                                                                "→ Próxima tecla em:"
                                                            }
                                                            MacroExecutionMode::Parallel => {
                                                                "Repetir a cada:"
                                                            }
                                                        };
                                                        ui.label(
                                                            RichText::new(wait_label)
                                                                .color(MUTED)
                                                                .size(11.5),
                                                        );
                                                        if ui
                                                            .add(
                                                                egui::DragValue::new(
                                                                    &mut rule.interval_ms,
                                                                )
                                                                .speed(5.0)
                                                                .range(
                                                                    MIN_MACRO_INTERVAL_MS
                                                                        ..=MAX_MACRO_INTERVAL_MS,
                                                                )
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

                                                    // Right-side controls: Reorder (↑ ↓), Trigger count, Test, Delete
                                                    ui.with_layout(
                                                        egui::Layout::right_to_left(
                                                            egui::Align::Center,
                                                        ),
                                                        |ui| {
                                                            if ui
                                                                .small_button(
                                                                    RichText::new("Remover")
                                                                        .color(WARN)
                                                                        .size(11.0),
                                                                )
                                                                .on_hover_text("Remover este passo")
                                                                .clicked()
                                                            {
                                                                remove_idx = Some(idx);
                                                            }

                                                            if ui
                                                                .small_button(
                                                                    RichText::new("Testar")
                                                                        .color(TEXT)
                                                                        .size(11.0),
                                                                )
                                                                .on_hover_text(
                                                                    "Enviar esta tecla 1 vez agora para testar no jogo",
                                                                )
                                                                .clicked()
                                                            {
                                                                let _ = self.ui_tx.send(
                                                                    UiCmd::TestMacroKey {
                                                                        pid: target_pid,
                                                                        normalized_name: norm_name
                                                                            .clone(),
                                                                        key: rule.key.clone(),
                                                                    },
                                                                );
                                                            }

                                                            if total_rules > 1 {
                                                                if ui
                                                                    .add_enabled(
                                                                        idx + 1 < total_rules,
                                                                        egui::Button::new(
                                                                            RichText::new("↓")
                                                                                .size(11.0),
                                                                        )
                                                                        .small(),
                                                                    )
                                                                    .on_hover_text(
                                                                        "Mover passo para baixo",
                                                                    )
                                                                    .clicked()
                                                                {
                                                                    swap_pair =
                                                                        Some((idx, idx + 1));
                                                                }
                                                                if ui
                                                                    .add_enabled(
                                                                        idx > 0,
                                                                        egui::Button::new(
                                                                            RichText::new("↑")
                                                                                .size(11.0),
                                                                        )
                                                                        .small(),
                                                                    )
                                                                    .on_hover_text(
                                                                        "Mover passo para cima",
                                                                    )
                                                                    .clicked()
                                                                {
                                                                    swap_pair =
                                                                        Some((idx, idx - 1));
                                                                }
                                                            }

                                                            ui.label(
                                                                RichText::new(format!(
                                                                    "{trig}x"
                                                                ))
                                                                .color(if trig > 0 {
                                                                    OK
                                                                } else {
                                                                    MUTED
                                                                })
                                                                .size(11.0)
                                                                .monospace(),
                                                            )
                                                            .on_hover_text(
                                                                "Quantas vezes este passo já foi enviado nesta sessão",
                                                            );
                                                        },
                                                    );
                                                });
                                            });

                                        // Visual connector arrow between sequential steps or cycle footer
                                        if profile.mode == MacroExecutionMode::Sequential {
                                            if idx + 1 < total_rules {
                                                ui.horizontal(|ui| {
                                                    ui.add_space(28.0);
                                                    ui.label(
                                                        RichText::new(format!(
                                                            "↓  após {} ms executa:",
                                                            rule.interval_ms
                                                        ))
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
                                                        if waiting_for_next_cycle {
                                                            OK
                                                        } else {
                                                            STROKE
                                                        },
                                                    ))
                                                    .corner_radius(6.0)
                                                    .inner_margin(egui::Margin::symmetric(10, 6))
                                                    .show(ui, |ui| {
                                                        let cycle_sec = profile.cycle_interval_ms
                                                            as f64
                                                            / 1000.0;
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
                                                                .color(if waiting_for_next_cycle {
                                                                    OK
                                                                } else {
                                                                    ACCENT
                                                                })
                                                                .size(11.5)
                                                                .strong(),
                                                        );
                                                    });
                                            }
                                        } else {
                                            ui.add_space(4.0);
                                        }
                                    }
                                });
                            contain_list_scroll(ui, list.inner_rect);

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
                                    &norm_name,
                                    profile.mode,
                                    profile.cycle_interval_ms,
                                    profile.toggle_key.clone(),
                                    rules_mut,
                                );
                            }
                        }
                    });
            });
        if !open {
            self.capturing_macro_key_for = None;
            self.recording_macro_sequence = false;
        }
        self.show_macro_modal = open;
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
                let dot = ui.allocate_response(Vec2::splat(8.0), Sense::hover());
                ui.painter().circle_filled(dot.rect.center(), 3.5, color);
                ui.label(RichText::new(label).color(color).size(12.0).strong());
            });
        });
}

fn metric_pill(ui: &mut egui::Ui, label: &str, value: &str, accent: Color32) {
    egui::Frame::new()
        .fill(BG_RAISED)
        .stroke(Stroke::new(1.0, STROKE))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(label).color(MUTED).size(11.0));
                ui.label(RichText::new(value).color(accent).strong().size(13.0));
            });
        });
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
        let mut tunnel_mgr = crate::tunnel::TunnelManager::new(config.wireguard.clone());
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

        let sync_tunnel = |mgr: &mut crate::tunnel::TunnelManager,
                           pids: &[u32],
                           tx: &Sender<WorkerMsg>| {
            let pass = mgr.process_game_pids(pids);
            if pass.fallback_bypassed {
                send_worker(&ctx, tx, WorkerMsg::Log(
                    "⚠️ [Túnel VPS] Falha na conexão com a VPS detectada! Fallback automático ativado (tráfego direto via operadora para evitar queda do jogo)".to_string(),
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
            send_worker(&ctx, tx, WorkerMsg::Tunnel(mgr.status()));
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
                    let sent = macro_mgr.test_single_key(pid, &normalized_name, &key);
                    let label = key_display_label(&key);
                    if sent > 0 {
                        send_worker(
                            &ctx,
                            &worker_tx,
                            WorkerMsg::Log(format!(
                                "⌨️ [Macro] Teste de '{label}' enviado para {normalized_name} (PID {pid})."
                            )),
                        );
                    } else {
                        send_worker(
                            &ctx,
                            &worker_tx,
                            WorkerMsg::Log(format!(
                                "⚠️ [Macro] Não foi possível encontrar janela visível para PID {pid} ao testar '{label}'."
                            )),
                        );
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
                    if !active_game_pids.is_empty()
                        && last_fast_tunnel_sync.elapsed() >= Duration::from_millis(150)
                    {
                        last_fast_tunnel_sync = Instant::now();
                        sync_tunnel(&mut tunnel_mgr, &active_game_pids, &worker_tx);
                    }

                    // Periodic system scan every 2 seconds to detect new/closed processes
                    if config_valid && last_scan.elapsed() >= Duration::from_secs(2) {
                        last_scan = Instant::now();
                        let is_watch_active = watch_flag.load(Ordering::SeqCst);
                        let should_optimize = is_watch_active
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
    use super::{contain_list_scroll, debounce_elapsed, narrow_layout, SCROLL_AREA_IDS};
    use std::collections::HashSet;
    use std::time::{Duration, Instant};

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
}
