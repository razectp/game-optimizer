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

enum WorkerMsg {
    Report(OptimizeReport),
    Log(String),
    Watching(bool),
    Tunnel(TunnelStatus),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProcessTab {
    RunningProcesses,
    Games,
    MemoryTrim,
}

enum UiCmd {
    Optimize { trim_game: bool },
    Scan,
    StartWatch { interval: u64, trim_game: bool },
    StopWatch,
    UpdateConfig(Box<AppConfig>),
    TestVpsPing,
}

enum AppTrayEvent {
    Menu(MenuEvent),
    Tray(TrayIconEvent),
}

/// Launch the native GUI (blocks until quit).
pub fn run(config_path: PathBuf) -> eframe::Result<()> {
    let icon = tray_rgba_icon();
    let options = NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([880.0, 560.0])
            .with_min_inner_size([780.0, 500.0])
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
            Ok(Box::new(GuiApp::new(
                config_path,
                icon,
                tray_rx,
                cc.egui_ctx.clone(),
            )) as Box<dyn App>)
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
    /// Must stay owned so the tray icon is not destroyed.
    #[allow(dead_code)]
    tray: Option<TrayIcon>,
    tray_show: MenuItem,
    tray_optimize: MenuItem,
    tray_quit: MenuItem,
    ui_tx: Sender<UiCmd>,
    worker_rx: Receiver<WorkerMsg>,
    tray_rx: Receiver<AppTrayEvent>,
    watch_flag: Arc<AtomicBool>,
    started: Instant,
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

        let initial_cfg = AppConfig::load_or_default(&config_path).unwrap_or_default();
        let wireguard_cfg = initial_cfg.wireguard.clone();
        let trim_game_memory = initial_cfg.trim_game_memory;

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
            ui_rx,
            worker_tx,
            Arc::clone(&watch_flag),
            ctx,
        );

        let mut app = Self {
            config_path,
            app_config: initial_cfg,
            report: None,
            logs: vec!["Pronto. O X ou botão Bandeja minimizam para a bandeja.".into()],
            watching: false,
            watch_interval: 45,
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
            tray,
            tray_show,
            tray_optimize,
            tray_quit,
            ui_tx,
            worker_rx,
            tray_rx,
            watch_flag,
            started: Instant::now(),
        };
        app.refresh_running_processes();
        let _ = app.ui_tx.send(UiCmd::Scan);
        app
    }

    fn refresh_running_processes(&mut self) {
        self.running_processes = crate::detect::list_running_processes(&self.app_config);
    }

    fn add_process_as_game(&mut self, proc: &RunningProcessInfo) {
        let mut added = false;
        if self.app_config.add_game_name(&proc.normalized_name) {
            added = true;
        }
        if proc.root_name != proc.normalized_name && self.app_config.add_game_name(&proc.root_name) {
            added = true;
        }
        if added {
            if proc.root_name != proc.normalized_name {
                self.push_log(format!(
                    "Jogo adicionado: {} (Processo raiz detectado: {}, PID: {})",
                    proc.name, proc.root_name, proc.root_pid
                ));
            } else {
                self.push_log(format!("Jogo adicionado: {} (PID: {})", proc.name, proc.pid));
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
        if proc.root_name != proc.normalized_name && self.app_config.remove_game_name(&proc.root_name) {
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
        self.app_config.wireguard = self.wireguard_cfg.clone();
        self.app_config.trim_game_memory = self.trim_game_memory;
        if let Err(err) = self.app_config.save_to_user_dir() {
            self.push_log(format!("Erro ao salvar configuração no perfil: {err}"));
        } else {
            self.push_log("Configuração salva no seu perfil (%LOCALAPPDATA%\\GameOptimizer).");
        }
        let _ = self.ui_tx.send(UiCmd::UpdateConfig(Box::new(self.app_config.clone())));
    }

    fn import_conf_file(&mut self) {
        if let Some(file_path) = rfd::FileDialog::new()
            .add_filter("Configuração WireGuard (*.conf)", &["conf"])
            .set_title("Selecione o arquivo .conf do WireGuard")
            .pick_file()
        {
            match std::fs::read_to_string(&file_path) {
                Ok(content) => match WireGuardConfig::parse_conf(&content) {
                    Ok(parsed) => {
                        self.wireguard_cfg = parsed;
                        let file_name = file_path
                            .file_name()
                            .map(|s| s.to_string_lossy())
                            .unwrap_or_default();
                        self.push_log(format!("Túnel importado com sucesso de {file_name}"));
                        self.save_app_config();
                    }
                    Err(err) => {
                        self.push_log(format!("Erro ao processar .conf: {err}"));
                    }
                },
                Err(err) => {
                    self.push_log(format!("Falha ao ler arquivo: {err}"));
                }
            }
        }
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
        while let Ok(msg) = self.worker_rx.try_recv() {
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
                        let _ = self.ui_tx.send(UiCmd::Optimize {
                            trim_game: self.trim_game_memory,
                        });
                    } else if ev.id == self.tray_quit.id() {
                        self.watch_flag.store(false, Ordering::SeqCst);
                        std::process::exit(0);
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

        let close_requested = ctx.input(|i| i.viewport().close_requested());
        if close_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            if !self.start_minimized_hint {
                self.push_log("Na bandeja do sistema. Clique no ícone para reabrir.");
                self.start_minimized_hint = true;
            }
        }

        ctx.request_repaint_after(Duration::from_millis(200));
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

        ui.horizontal(|ui| {
            // Brand mark
            let mark = ui.allocate_response(Vec2::splat(32.0), Sense::hover());
            ui.painter().circle_filled(mark.rect.center(), 14.0, brand_glow);
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
                let primary = ui.add_sized(
                    [150.0, 36.0],
                    egui::Button::new(
                        RichText::new("⚡ Otimizar Agora")
                            .strong()
                            .size(13.0)
                            .color(Color32::from_rgb(18, 14, 8)),
                    )
                    .fill(ACCENT)
                    .corner_radius(8.0),
                );
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
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                }
            });
        });

        ui.add_space(10.0);
        // Metric KPI strip
        ui.horizontal(|ui| {
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
                Some(s) if s.health == TunnelHealth::Connecting => {
                    ("Conectando…".into(), ACCENT)
                }
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
                .on_hover_text("Clique para visualizar ou importar túnel WireGuard (.conf)")
                .clicked()
            {
                self.show_tunnel_modal = true;
            }
        });
    }

    fn draw_body(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) {
        let available = ui.available_height();
        ui.horizontal(|ui| {
            ui.set_min_height(available);

            // Left Main column ~67%
            ui.vertical(|ui| {
                ui.set_width(ui.available_width() * 0.67 - 6.0);
                ui.set_min_height(available);

                section_label(ui, "JOGOS EM EXECUÇÃO");
                egui::Frame::new()
                    .fill(BG_RAISED)
                    .stroke(Stroke::new(1.0, STROKE))
                    .corner_radius(10.0)
                    .inner_margin(12.0)
                    .show(ui, |ui| {
                        let list_h = (available * 0.44).clamp(110.0, 190.0);
                        egui::ScrollArea::vertical()
                            .max_height(list_h)
                            .auto_shrink([false; 2])
                            .show(ui, |ui| {
                                self.draw_games(ui);
                            });
                    });

                ui.add_space(10.0);
                section_label(ui, "LOG DE ATIVIDADES DO TÚNEL & SISTEMA");
                egui::Frame::new()
                    .fill(Color32::from_rgb(11, 13, 17))
                    .stroke(Stroke::new(1.0, STROKE))
                    .corner_radius(10.0)
                    .inner_margin(10.0)
                    .show(ui, |ui| {
                        let rest = ui.available_height().max(90.0);
                        egui::ScrollArea::vertical()
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
                    });
            });

            ui.add_space(10.0);

            // Right Sidebar (Ações e Ajustes)
            ui.vertical(|ui| {
                ui.set_width(ui.available_width());
                ui.set_min_height(available);

                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
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
                                        let _ = self.ui_tx.send(UiCmd::StopWatch);
                                    }
                                } else if ui
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
                                    let _ = self.ui_tx.send(UiCmd::StartWatch {
                                        interval: self.watch_interval,
                                        trim_game: self.trim_game_memory,
                                    });
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
                                    .changed()
                                {
                                    match autostart::set_enabled(self.autostart) {
                                        Ok(()) => self.push_log(if self.autostart {
                                            "Inicialização automática ativada."
                                        } else {
                                            "Inicialização automática desativada."
                                        }),
                                        Err(err) => self.push_log(format!("Auto-start: {err}")),
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
                                ui.add(
                                    egui::Slider::new(&mut self.watch_interval, 15..=120)
                                        .clamping(egui::SliderClamping::Always),
                                );

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
                    });
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
                    .on_hover_text("Escolha diretamente da lista de processos abertos sem navegar pastas")
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
                ui.label(RichText::new("JOGOS ATIVOS").color(MUTED).size(11.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button(RichText::new("➕ Adicionar outro processo").color(TEXT).size(11.0))
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
                egui::Frame::new()
                    .fill(BG_ROW)
                    .stroke(Stroke::new(1.0, STROKE))
                    .corner_radius(8.0)
                    .inner_margin(egui::Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("🎮").size(16.0));
                            ui.label(RichText::new(&game.name).color(TEXT).strong().size(15.0));

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
                        ui.horizontal(|ui| {
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
                                            RichText::new("• 🛡️ Fallback direto ativo (evitando queda)")
                                                .color(WARN)
                                                .size(11.0)
                                                .strong(),
                                        );
                                    }
                                    TunnelHealth::Idle => {}
                                }
                            }
                        });

                        // Show actively routed destinations if available
                        if let Some(status) = &self.tunnel_status {
                            if status.active && !status.routed_destinations.is_empty() {
                                ui.add_space(4.0);
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(RichText::new("📡 Servidores roteados:").color(MUTED).size(11.0));
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
            .resizable(false)
            .show(ctx, |ui| {
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
                                RichText::new("📥 Importar Túnel (.conf)")
                                    .strong()
                                    .color(Color32::from_rgb(28, 22, 12)),
                            )
                            .fill(ACCENT)
                            .corner_radius(6.0),
                        )
                        .on_hover_text("Importe diretamente um arquivo .conf do WireGuard gerado pelo seu servidor")
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
                                    self.save_app_config();
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
                        self.save_app_config();
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
        self.show_tunnel_modal = open;
    }

    fn draw_process_modal(&mut self, ctx: &egui::Context) {
        let mut open = self.show_process_modal;
        egui::Window::new("⚙️ Gerenciador de Processos, Jogos & Trim")
            .open(&mut open)
            .default_width(700.0)
            .default_height(500.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
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
                            egui::ScrollArea::vertical()
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
                            egui::ScrollArea::vertical().max_height(130.0).show(ui, |ui| {
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
                            egui::ScrollArea::vertical().max_height(100.0).show(ui, |ui| {
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
        self.show_process_modal = open;
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
    ui_rx: Receiver<UiCmd>,
    worker_tx: Sender<WorkerMsg>,
    watch_flag: Arc<AtomicBool>,
    ctx: egui::Context,
) {
    thread::spawn(move || {
        let mut interval = 45u64;
        let mut trim_game = false;
        let initial_cfg = AppConfig::load_or_default(&config_path).unwrap_or_default();
        let mut tunnel_mgr = crate::tunnel::TunnelManager::new(initial_cfg.wireguard);

        let mut last_scan = Instant::now();
        let mut last_optimize = Instant::now();
        let mut prev_game_count = 0usize;

        let sync_tunnel = |mgr: &mut crate::tunnel::TunnelManager,
                           report: &OptimizeReport,
                           tx: &Sender<WorkerMsg>| {
            let pids: Vec<u32> = report.games.iter().map(|g| g.pid).collect();
            let pass = mgr.process_game_pids(&pids);
            if pass.fallback_bypassed {
                let _ = tx.send(WorkerMsg::Log(
                    "⚠️ [Túnel VPS] Falha na conexão com a VPS detectada! Fallback automático ativado (tráfego direto via operadora para evitar queda do jogo)".to_string(),
                ));
            }
            if pass.recovered {
                let _ = tx.send(WorkerMsg::Log(
                    "🟢 [Túnel VPS] Conexão com a VPS restabelecida com sucesso! Reativando tunelamento otimizado".to_string(),
                ));
            }
            for conn in &pass.newly_routed {
                let _ = tx.send(WorkerMsg::Log(format!(
                    "[Túnel VPS] Roteando {} ({}:{}) via VPS",
                    conn.protocol, conn.remote_ip, conn.remote_port
                )));
            }
            if !pass.removed_routes.is_empty() && !pass.fallback_bypassed {
                let _ = tx.send(WorkerMsg::Log(format!(
                    "[Túnel VPS] {} rota(s) removida(s) de jogo finalizado",
                    pass.removed_routes.len()
                )));
            }
            let _ = tx.send(WorkerMsg::Tunnel(mgr.status()));
        };

        loop {
            // Heartbeat: wake up egui event loop so tray events & worker msgs are processed while hidden
            ctx.request_repaint();

            match ui_rx.recv_timeout(Duration::from_millis(200)) {
                Ok(UiCmd::Scan) => {
                    let config = AppConfig::load_or_default(&config_path).unwrap_or_default();
                    let report = optimize_system(&OptimizeRequest {
                        config,
                        dry_run: true,
                    });
                    prev_game_count = report.games.len();
                    let _ = worker_tx.send(WorkerMsg::Log(format!(
                        "Scan: {} jogo(s) monitorado(s).",
                        report.games.len()
                    )));
                    sync_tunnel(&mut tunnel_mgr, &report, &worker_tx);
                    let _ = worker_tx.send(WorkerMsg::Report(report));
                    last_scan = Instant::now();
                }
                Ok(UiCmd::Optimize { trim_game: tg }) => {
                    trim_game = tg;
                    let mut config = AppConfig::load_or_default(&config_path).unwrap_or_default();
                    config.trim_game_memory = trim_game;
                    let report = optimize_system(&OptimizeRequest {
                        config,
                        dry_run: false,
                    });
                    prev_game_count = report.games.len();
                    let ok = report.results.iter().filter(|r| r.ok).count();
                    let fail = report.results.iter().filter(|r| !r.ok).count();
                    let _ = worker_tx.send(WorkerMsg::Log(format!(
                        "Otimização: {ok} ok, {fail} falha(s), {} jogo(s).",
                        report.games.len()
                    )));
                    sync_tunnel(&mut tunnel_mgr, &report, &worker_tx);
                    let _ = worker_tx.send(WorkerMsg::Report(report));
                    last_optimize = Instant::now();
                    last_scan = Instant::now();
                }
                Ok(UiCmd::StartWatch {
                    interval: secs,
                    trim_game: tg,
                }) => {
                    interval = secs.max(15);
                    trim_game = tg;
                    watch_flag.store(true, Ordering::SeqCst);
                    let _ = worker_tx.send(WorkerMsg::Watching(true));
                    let _ =
                        worker_tx.send(WorkerMsg::Log(format!("Vigilância a cada {interval}s.")));
                    last_optimize = Instant::now();
                }
                Ok(UiCmd::StopWatch) => {
                    watch_flag.store(false, Ordering::SeqCst);
                    let _ = worker_tx.send(WorkerMsg::Watching(false));
                    let _ = worker_tx.send(WorkerMsg::Log("Vigilância parada.".into()));
                }
                Ok(UiCmd::UpdateConfig(new_cfg)) => {
                    if let Err(err) = new_cfg.save_to_user_dir() {
                        let _ = worker_tx
                            .send(WorkerMsg::Log(format!("Erro ao salvar configuração: {err}")));
                    } else {
                        let _ = worker_tx.send(WorkerMsg::Log(
                            "Configurações atualizadas no perfil (%LOCALAPPDATA%\\GameOptimizer).".into(),
                        ));
                    }
                    tunnel_mgr.update_config(new_cfg.wireguard.clone());
                    let _ = worker_tx.send(WorkerMsg::Tunnel(tunnel_mgr.status()));
                }
                Ok(UiCmd::TestVpsPing) => {
                    let _ = worker_tx.send(WorkerMsg::Log(
                        "Testando ping para o endpoint da VPS...".into(),
                    ));
                    match tunnel_mgr.test_vps_ping() {
                        Ok(ms) => {
                            let _ = worker_tx.send(WorkerMsg::Log(format!("Ping VPS: {ms}ms.")));
                        }
                        Err(err) => {
                            let _ =
                                worker_tx.send(WorkerMsg::Log(format!("Falha no ping VPS: {err}")));
                        }
                    }
                    let _ = worker_tx.send(WorkerMsg::Tunnel(tunnel_mgr.status()));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // Continuous automatic monitoring for running games & tunnel sync every 2 seconds
                    if last_scan.elapsed() >= Duration::from_secs(2) {
                        last_scan = Instant::now();
                        let mut config =
                            AppConfig::load_or_default(&config_path).unwrap_or_default();

                        let is_watch_active = watch_flag.load(Ordering::SeqCst);
                        let should_optimize = is_watch_active
                            && last_optimize.elapsed() >= Duration::from_secs(interval.max(15));

                        config.trim_game_memory = trim_game;
                        let report = optimize_system(&OptimizeRequest {
                            config,
                            dry_run: !should_optimize,
                        });

                        if should_optimize {
                            last_optimize = Instant::now();
                            let ok = report.results.iter().filter(|r| r.ok).count();
                            let _ = worker_tx.send(WorkerMsg::Log(format!(
                                "Vigilância periódica: {ok} ações aplicadas."
                            )));
                        }

                        // Sync tunnel if games are currently active, or if games were running and closed
                        if !report.games.is_empty() || prev_game_count > 0 {
                            sync_tunnel(&mut tunnel_mgr, &report, &worker_tx);
                            let _ = worker_tx.send(WorkerMsg::Report(report.clone()));
                        }
                        prev_game_count = report.games.len();
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    tunnel_mgr.shutdown();
                    break;
                }
            }
        }
    });
}
