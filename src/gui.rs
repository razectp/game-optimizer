//! End-user GUI (Portuguese) with system-tray minimize.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, RichText, Sense,
    Stroke, Vec2, ViewportCommand,
};
use eframe::{App, Frame, NativeOptions};
use image::{ImageBuffer, Rgba};
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::autostart;
use crate::cache::{self, CacheCleanReport};
use crate::config::AppConfig;
use crate::optimize::{optimize_system, OptimizeReport, OptimizeRequest};
use crate::report::format_mib;
use crate::setup_wizard::{self, SetupWizard};
use crate::win_window;

/// Warm amber — primary actions.
const ACCENT: Color32 = Color32::from_rgb(240, 176, 72);
const ACCENT_SOFT: Color32 = Color32::from_rgb(72, 52, 24);
const OK: Color32 = Color32::from_rgb(120, 186, 140);
const BG: Color32 = Color32::from_rgb(28, 26, 24);
const BG_RAISED: Color32 = Color32::from_rgb(42, 38, 34);
const BG_ROW: Color32 = Color32::from_rgb(54, 48, 42);
const STROKE: Color32 = Color32::from_rgb(72, 64, 56);
const TEXT: Color32 = Color32::from_rgb(250, 246, 238);
const MUTED: Color32 = Color32::from_rgb(168, 158, 146);
const WARN: Color32 = Color32::from_rgb(220, 150, 80);

enum WorkerMsg {
    Report(OptimizeReport),
    Status(String),
    Watching(bool),
    Cache(CacheCleanReport),
    Busy(bool),
}

enum UiCmd {
    Optimize { trim_game: bool },
    Scan,
    StartWatch { interval: u64, trim_game: bool },
    StopWatch,
    CleanCache,
}

struct TraySignals {
    show: AtomicBool,
    optimize: AtomicBool,
    quit: AtomicBool,
}

/// Launch the native GUI (blocks until quit).
pub fn run(config_path: PathBuf, force_setup: bool) -> eframe::Result<()> {
    let icon = tray_rgba_icon();
    let options = NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([760.0, 540.0])
            .with_min_inner_size([680.0, 480.0])
            .with_title(win_window::WINDOW_TITLE)
            .with_icon(eframe_icon()),
        ..Default::default()
    };

    eframe::run_native(
        "Game Optimizer",
        options,
        Box::new(move |cc| {
            install_fonts(&cc.egui_ctx);
            style_visuals(&cc.egui_ctx);
            Ok(Box::new(GuiApp::new(config_path, icon, force_setup)) as Box<dyn App>)
        }),
    )
}

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
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
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(68, 60, 52);
    visuals.widgets.active.bg_fill = ACCENT_SOFT;
    visuals.selection.bg_fill = ACCENT_SOFT;
    visuals.extreme_bg_color = BG_RAISED;
    visuals.widgets.inactive.corner_radius = CornerRadius::same(10);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(10);
    visuals.widgets.active.corner_radius = CornerRadius::same(10);
    ctx.set_visuals(visuals);

    ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.spacing.item_spacing = Vec2::new(10.0, 8.0);
        style.spacing.button_padding = Vec2::new(14.0, 8.0);
        style.spacing.window_margin = egui::Margin::same(0);
        style.spacing.indent = 12.0;
    });
}

fn brand_icon_rgba(size: u32) -> Vec<u8> {
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    let cx = size as f32 / 2.0 - 0.5;
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 - cx;
            let dy = y as f32 - cx;
            let r = (dx * dx + dy * dy).sqrt();
            let outer = size as f32 / 2.0;
            let inner = outer - 2.0;
            if r < inner - 4.0 {
                rgba.extend_from_slice(&[240, 176, 72, 255]);
            } else if r < inner {
                rgba.extend_from_slice(&[28, 26, 24, 255]);
            } else if r < outer {
                rgba.extend_from_slice(&[240, 176, 72, 80]);
            } else {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    rgba
}

fn tray_rgba_icon() -> Icon {
    let size = 32u32;
    let mut img = ImageBuffer::<Rgba<u8>, Vec<u8>>::new(size, size);
    let raw = brand_icon_rgba(size);
    for y in 0..size {
        for x in 0..size {
            let i = ((y * size + x) * 4) as usize;
            img.put_pixel(x, y, Rgba([raw[i], raw[i + 1], raw[i + 2], raw[i + 3]]));
        }
    }
    Icon::from_rgba(img.into_raw(), size, size).expect("tray icon")
}

fn eframe_icon() -> egui::IconData {
    let size = 32u32;
    egui::IconData {
        rgba: brand_icon_rgba(size),
        width: size,
        height: size,
    }
}

struct GuiApp {
    report: Option<OptimizeReport>,
    status: String,
    watching: bool,
    watch_interval: u64,
    trim_game_memory: bool,
    autostart: bool,
    busy: bool,
    in_tray: bool,
    restore_maximized: bool,
    cache_armed: bool,
    tray_pump_started: bool,
    #[allow(dead_code)]
    tray: Option<TrayIcon>,
    tray_show: MenuItem,
    tray_optimize: MenuItem,
    tray_quit: MenuItem,
    tray_signals: Arc<TraySignals>,
    ui_tx: Sender<UiCmd>,
    worker_rx: Receiver<WorkerMsg>,
    watch_flag: Arc<AtomicBool>,
    wizard: Option<SetupWizard>,
}

impl GuiApp {
    fn new(config_path: PathBuf, icon: Icon, force_setup: bool) -> Self {
        let show_wizard = setup_wizard::should_show(force_setup);
        let autostart = if show_wizard {
            autostart::is_enabled() || !autostart::initialized_marker_exists()
        } else {
            autostart::ensure_default_enabled().unwrap_or_else(|_| autostart::is_enabled())
        };

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
        );

        let app = Self {
            report: None,
            status: "Pronto para otimizar seus jogos.".into(),
            watching: false,
            watch_interval: 45,
            trim_game_memory: false,
            autostart,
            busy: false,
            in_tray: false,
            restore_maximized: false,
            cache_armed: false,
            tray_pump_started: false,
            tray,
            tray_show,
            tray_optimize,
            tray_quit,
            tray_signals: Arc::new(TraySignals {
                show: AtomicBool::new(false),
                optimize: AtomicBool::new(false),
                quit: AtomicBool::new(false),
            }),
            ui_tx,
            worker_rx,
            watch_flag,
            wizard: if show_wizard {
                Some(SetupWizard::new(autostart, true))
            } else {
                None
            },
        };
        let _ = app.ui_tx.send(UiCmd::Scan);
        app
    }

    fn snapshot_maximized(&mut self, ctx: &egui::Context) {
        self.restore_maximized = win_window::remembered_maximized(
            ctx.input(|i| i.viewport().maximized),
            win_window::is_main_window_maximized(),
        );
    }

    fn hide_to_tray(&mut self, ctx: &egui::Context) {
        self.snapshot_maximized(ctx);
        self.in_tray = true;
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        let _ = win_window::hide_main_window();
    }

    fn restore_from_tray(&mut self, ctx: &egui::Context) {
        self.in_tray = false;
        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(ViewportCommand::Maximized(self.restore_maximized));
        ctx.send_viewport_cmd(ViewportCommand::Focus);
        ctx.send_viewport_cmd(ViewportCommand::WindowLevel(egui::WindowLevel::AlwaysOnTop));
        ctx.send_viewport_cmd(ViewportCommand::WindowLevel(egui::WindowLevel::Normal));
        let _ = win_window::restore_main_window(self.restore_maximized);
        ctx.request_repaint();
    }

    fn ensure_tray_pump(&mut self, ctx: &egui::Context) {
        if self.tray_pump_started {
            return;
        }
        self.tray_pump_started = true;
        spawn_tray_pump(
            ctx.clone(),
            Arc::clone(&self.tray_signals),
            self.tray_show.id().clone(),
            self.tray_optimize.id().clone(),
            self.tray_quit.id().clone(),
        );
    }

    fn poll_worker(&mut self) {
        while let Ok(msg) = self.worker_rx.try_recv() {
            match msg {
                WorkerMsg::Report(report) => {
                    self.report = Some(report);
                }
                WorkerMsg::Status(line) => self.status = line,
                WorkerMsg::Watching(active) => {
                    self.watching = active;
                    self.status = if active {
                        "Cuidando dos jogos em segundo plano.".into()
                    } else {
                        "Pausado. Toque em Otimizar quando quiser.".into()
                    };
                }
                WorkerMsg::Cache(report) => {
                    self.status = report.friendly_summary();
                    self.cache_armed = false;
                }
                WorkerMsg::Busy(busy) => self.busy = busy,
            }
        }
    }

    fn poll_tray_signals(&mut self, ctx: &egui::Context) {
        if self.tray_signals.show.swap(false, Ordering::SeqCst) {
            self.restore_from_tray(ctx);
        }
        if self.tray_signals.optimize.swap(false, Ordering::SeqCst) {
            let _ = self.ui_tx.send(UiCmd::Optimize {
                trim_game: self.trim_game_memory,
            });
        }
        if self.tray_signals.quit.swap(false, Ordering::SeqCst) {
            self.watch_flag.store(false, Ordering::SeqCst);
            std::process::exit(0);
        }
    }

    fn game_count(&self) -> usize {
        self.report.as_ref().map(|r| r.games.len()).unwrap_or(0)
    }

    fn reclaim_count(&self) -> usize {
        self.report
            .as_ref()
            .map(|r| r.reclaim_candidates.len())
            .unwrap_or(0)
    }
}

impl App for GuiApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        self.ensure_tray_pump(ctx);
        self.poll_worker();
        self.poll_tray_signals(ctx);

        let close_requested = ctx.input(|i| i.viewport().close_requested());
        if close_requested {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.hide_to_tray(ctx);
        }

        // Keep the event loop alive only while hidden so tray clicks restore
        // without a constant idle repaint when the window is visible.
        if self.in_tray {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut Frame) {
        let ctx = ui.ctx().clone();
        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, 0.0, BG);
        let glow = egui::Rect::from_min_size(rect.min, Vec2::new(rect.width() * 0.6, 140.0));
        ui.painter()
            .rect_filled(glow, 0.0, Color32::from_rgba_unmultiplied(240, 176, 72, 16));

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(Color32::TRANSPARENT)
                    .inner_margin(egui::Margin::symmetric(20, 16)),
            )
            .show(ui, |ui| {
                if self.wizard.is_some() {
                    self.draw_setup_wizard(ui);
                } else {
                    self.draw_header(ui, &ctx);
                    ui.add_space(14.0);
                    self.draw_body(ui, &ctx);
                }
            });
    }
}

impl GuiApp {
    fn finish_setup_wizard(&mut self) {
        let Some(wizard) = self.wizard.take() else {
            return;
        };
        self.autostart = wizard.autostart;
        match autostart::set_enabled(wizard.autostart) {
            Ok(()) => {
                self.status = if wizard.autostart {
                    "Vai abrir junto com o Windows.".into()
                } else {
                    "Não abre automaticamente com o Windows.".into()
                };
            }
            Err(err) => {
                self.status = format!("Não foi possível alterar o início: {err}");
            }
        }
        if let Err(err) = setup_wizard::mark_complete() {
            self.status = format!("Não foi possível gravar o assistente: {err}");
        }
        if wizard.start_watching {
            let _ = self.ui_tx.send(UiCmd::StartWatch {
                interval: self.watch_interval,
                trim_game: self.trim_game_memory,
            });
        }
    }

    fn draw_setup_wizard(&mut self, ui: &mut egui::Ui) {
        let mut finished = false;
        let mut skipped = false;

        ui.vertical_centered(|ui| {
            ui.add_space(8.0);
            ui.label(
                RichText::new("Assistente de configuração")
                    .font(FontId::new(13.0, FontFamily::Proportional))
                    .color(MUTED),
            );
            ui.add_space(4.0);
            ui.label(
                RichText::new("Game Optimizer")
                    .font(FontId::new(26.0, FontFamily::Proportional))
                    .color(TEXT)
                    .strong(),
            );
        });

        ui.add_space(16.0);
        ui.horizontal(|ui| {
            let remaining = ui.available_width();
            ui.add_space(((remaining - 168.0) / 2.0).max(0.0));
            let page = self.wizard.as_ref().map(|w| w.page).unwrap_or(0);
            for i in 0..setup_wizard::PAGE_COUNT {
                let active = i == page;
                let done = i < page;
                let color = if active {
                    ACCENT
                } else if done {
                    OK
                } else {
                    STROKE
                };
                let mark = ui.allocate_response(Vec2::splat(22.0), Sense::hover());
                ui.painter()
                    .circle_filled(mark.rect.center(), 9.0, color.gamma_multiply(0.35));
                ui.painter().circle_filled(
                    mark.rect.center(),
                    5.0,
                    if active || done { color } else { MUTED },
                );
                if i + 1 < setup_wizard::PAGE_COUNT {
                    ui.add_space(8.0);
                }
            }
        });

        ui.add_space(18.0);

        let copy = self
            .wizard
            .as_ref()
            .map(SetupWizard::copy)
            .unwrap_or(setup_wizard::page_copy(0));
        let on_prefs = self.wizard.as_ref().map(|w| w.page).unwrap_or(0) == 2;

        egui::Frame::new()
            .fill(BG_RAISED)
            .stroke(Stroke::new(1.0, STROKE))
            .corner_radius(16.0)
            .inner_margin(egui::Margin::symmetric(22, 20))
            .show(ui, |ui| {
                ui.label(RichText::new(copy.title).color(TEXT).size(20.0).strong());
                ui.add_space(10.0);
                ui.label(RichText::new(copy.body).color(MUTED).size(14.5));

                if on_prefs {
                    if let Some(wizard) = self.wizard.as_mut() {
                        ui.add_space(16.0);
                        ui.checkbox(
                            &mut wizard.autostart,
                            RichText::new("Iniciar com o Windows (recomendado)")
                                .color(TEXT)
                                .size(14.0),
                        );
                        ui.add_space(6.0);
                        ui.checkbox(
                            &mut wizard.start_watching,
                            RichText::new("Começar a cuidar dos jogos agora")
                                .color(TEXT)
                                .size(14.0),
                        );
                    }
                }
            });

        ui.add_space(20.0);
        ui.horizontal(|ui| {
            let can_back = self
                .wizard
                .as_ref()
                .map(SetupWizard::can_go_back)
                .unwrap_or(false);
            let is_last = self
                .wizard
                .as_ref()
                .map(SetupWizard::is_last)
                .unwrap_or(false);

            if ui
                .add_enabled(
                    can_back,
                    egui::Button::new(RichText::new("Voltar").color(TEXT))
                        .fill(BG_ROW)
                        .corner_radius(9.0)
                        .min_size(Vec2::new(110.0, 36.0)),
                )
                .clicked()
            {
                if let Some(wizard) = self.wizard.as_mut() {
                    wizard.back();
                }
            }

            if ui
                .add(
                    egui::Button::new(RichText::new("Pular").color(MUTED).size(13.0))
                        .fill(Color32::TRANSPARENT)
                        .corner_radius(9.0)
                        .min_size(Vec2::new(90.0, 36.0)),
                )
                .clicked()
            {
                skipped = true;
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let next_label = if is_last { "Concluir" } else { "Continuar" };
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new(next_label)
                                .strong()
                                .size(15.0)
                                .color(Color32::from_rgb(32, 24, 12)),
                        )
                        .fill(ACCENT)
                        .corner_radius(9.0)
                        .min_size(Vec2::new(140.0, 36.0)),
                    )
                    .clicked()
                {
                    if is_last {
                        finished = true;
                    } else if let Some(wizard) = self.wizard.as_mut() {
                        wizard.next();
                    }
                }
            });
        });

        if skipped || finished {
            self.finish_setup_wizard();
        }
    }

    fn draw_header(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            let mark = ui.allocate_response(Vec2::splat(32.0), Sense::hover());
            ui.painter()
                .circle_filled(mark.rect.center(), 13.0, ACCENT_SOFT);
            ui.painter().circle_filled(mark.rect.center(), 8.0, ACCENT);

            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("Game Optimizer")
                        .font(FontId::new(22.0, FontFamily::Proportional))
                        .color(TEXT)
                        .strong(),
                );
                ui.label(
                    RichText::new("Seus jogos, mais fluidos — nada é fechado.")
                        .color(MUTED)
                        .size(13.0),
                );
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let primary = ui.add_enabled(
                    !self.busy,
                    egui::Button::new(
                        RichText::new("Otimizar")
                            .strong()
                            .size(15.0)
                            .color(Color32::from_rgb(32, 24, 12)),
                    )
                    .fill(ACCENT)
                    .corner_radius(10.0)
                    .min_size(Vec2::new(132.0, 38.0)),
                );
                if primary.clicked() {
                    let _ = self.ui_tx.send(UiCmd::Optimize {
                        trim_game: self.trim_game_memory,
                    });
                }

                ui.add_space(8.0);
                status_chip(ui, self.watching);
            });
        });

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            metric_pill(ui, "Jogos abertos", &self.game_count().to_string(), ACCENT);
            metric_pill(ui, "Navegadores", &self.reclaim_count().to_string(), OK);
            if ui
                .add(
                    egui::Button::new(RichText::new("Minimizar").color(MUTED).size(12.0))
                        .fill(BG_RAISED)
                        .corner_radius(8.0),
                )
                .on_hover_text("Envia para a bandeja do sistema")
                .clicked()
            {
                self.hide_to_tray(ctx);
            }
        });

        ui.add_space(8.0);
        ui.label(RichText::new(&self.status).color(MUTED).size(13.0));
    }

    fn draw_body(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) {
        let available = ui.available_height();
        ui.horizontal(|ui| {
            ui.set_min_height(available);

            ui.vertical(|ui| {
                ui.set_width(ui.available_width() * 0.64 - 8.0);
                ui.set_min_height(available);

                section_label(ui, "Seus jogos");
                egui::Frame::new()
                    .fill(BG_RAISED)
                    .stroke(Stroke::new(1.0, STROKE))
                    .corner_radius(14.0)
                    .inner_margin(12.0)
                    .show(ui, |ui| {
                        let list_h = (available - 24.0).max(180.0);
                        egui::ScrollArea::vertical()
                            .max_height(list_h)
                            .auto_shrink([false; 2])
                            .show(ui, |ui| {
                                self.draw_games(ui);
                            });
                    });
            });

            ui.add_space(14.0);

            ui.vertical(|ui| {
                ui.set_width(ui.available_width());
                ui.set_min_height(available);

                section_label(ui, "Ações");
                egui::Frame::new()
                    .fill(BG_RAISED)
                    .stroke(Stroke::new(1.0, STROKE))
                    .corner_radius(14.0)
                    .inner_margin(12.0)
                    .show(ui, |ui| {
                        let wide = ui.available_width();
                        if ui
                            .add_enabled(
                                !self.busy,
                                egui::Button::new(RichText::new("Atualizar lista").color(TEXT))
                                    .fill(BG_ROW)
                                    .corner_radius(9.0)
                                    .min_size(Vec2::new(wide, 34.0)),
                            )
                            .clicked()
                        {
                            let _ = self.ui_tx.send(UiCmd::Scan);
                        }
                        ui.add_space(6.0);
                        if self.watching {
                            if ui
                                .add_sized(
                                    [wide, 34.0],
                                    egui::Button::new(RichText::new("Parar por agora").color(TEXT))
                                        .fill(Color32::from_rgb(78, 44, 36))
                                        .corner_radius(9.0),
                                )
                                .clicked()
                            {
                                let _ = self.ui_tx.send(UiCmd::StopWatch);
                            }
                        } else if ui
                            .add_enabled(
                                !self.busy,
                                egui::Button::new(RichText::new("Manter otimizado").color(TEXT))
                                    .fill(BG_ROW)
                                    .corner_radius(9.0)
                                    .min_size(Vec2::new(wide, 34.0)),
                            )
                            .clicked()
                        {
                            let _ = self.ui_tx.send(UiCmd::StartWatch {
                                interval: self.watch_interval,
                                trim_game: self.trim_game_memory,
                            });
                        }
                    });

                ui.add_space(12.0);
                section_label(ui, "Opções");
                egui::Frame::new()
                    .fill(BG_RAISED)
                    .stroke(Stroke::new(1.0, STROKE))
                    .corner_radius(14.0)
                    .inner_margin(12.0)
                    .show(ui, |ui| {
                        if ui
                            .checkbox(
                                &mut self.autostart,
                                RichText::new("Iniciar com o Windows")
                                    .color(TEXT)
                                    .size(13.5),
                            )
                            .changed()
                        {
                            match autostart::set_enabled(self.autostart) {
                                Ok(()) => {
                                    self.status = if self.autostart {
                                        "Vai abrir junto com o Windows.".into()
                                    } else {
                                        "Não abre mais automaticamente.".into()
                                    };
                                }
                                Err(err) => {
                                    self.status =
                                        format!("Não foi possível alterar o início: {err}");
                                }
                            }
                        }

                        if self.watching {
                            ui.add_space(8.0);
                            ui.label(
                                RichText::new("Checar a cada (segundos)")
                                    .color(MUTED)
                                    .size(12.0),
                            );
                            ui.add(
                                egui::Slider::new(&mut self.watch_interval, 15..=120)
                                    .clamping(egui::SliderClamping::Always),
                            );
                        }

                        ui.add_space(8.0);
                        egui::CollapsingHeader::new(
                            RichText::new("Mais opções").color(MUTED).size(13.0),
                        )
                        .default_open(false)
                        .show(ui, |ui| {
                            ui.add_space(4.0);
                            ui.checkbox(
                                &mut self.trim_game_memory,
                                RichText::new("Liberar RAM do jogo").color(TEXT).size(13.5),
                            );
                            ui.label(
                                RichText::new(
                                    "Pode dar uma travadinha. Deixe desligado se não tiver certeza.",
                                )
                                .color(WARN)
                                .size(11.5),
                            );

                            ui.add_space(10.0);
                            ui.label(
                                RichText::new(
                                    "Limpar pasta Temp do Windows. Não apaga cache de jogos.",
                                )
                                .color(MUTED)
                                .size(11.5),
                            );
                            ui.add_space(4.0);
                            let cache_label = if self.cache_armed {
                                "Confirmar limpeza"
                            } else {
                                "Limpar arquivos temporários"
                            };
                            let cache_fill = if self.cache_armed {
                                Color32::from_rgb(96, 56, 32)
                            } else {
                                BG_ROW
                            };
                            if ui
                                .add_enabled(
                                    !self.busy,
                                    egui::Button::new(RichText::new(cache_label).color(TEXT))
                                        .fill(cache_fill)
                                        .corner_radius(9.0)
                                        .min_size(Vec2::new(ui.available_width(), 34.0)),
                                )
                                .clicked()
                            {
                                if self.cache_armed {
                                    let _ = self.ui_tx.send(UiCmd::CleanCache);
                                    self.status = "Limpando arquivos temporários…".into();
                                } else {
                                    self.cache_armed = true;
                                    self.status =
                                        "Toque de novo para confirmar a limpeza da pasta Temp."
                                            .into();
                                }
                            }
                            if self.cache_armed
                                && ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new("Cancelar").color(MUTED).size(12.0),
                                        )
                                        .fill(Color32::TRANSPARENT),
                                    )
                                    .clicked()
                            {
                                self.cache_armed = false;
                                self.status = "Limpeza cancelada.".into();
                            }
                        });

                        ui.add_space(10.0);
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("Assistente de configuração")
                                        .color(MUTED)
                                        .size(12.5),
                                )
                                .fill(BG_ROW)
                                .corner_radius(8.0)
                                .min_size(Vec2::new(ui.available_width(), 32.0)),
                            )
                            .clicked()
                        {
                            self.wizard = Some(SetupWizard::new(self.autostart, self.watching));
                        }
                    });
            });
        });
    }

    fn draw_games(&self, ui: &mut egui::Ui) {
        let Some(report) = &self.report else {
            ui.label(
                RichText::new("Procurando jogos abertos…")
                    .color(MUTED)
                    .size(14.0),
            );
            return;
        };

        if report.games.is_empty() {
            ui.add_space(12.0);
            ui.label(
                RichText::new("Nenhum jogo aberto agora")
                    .color(TEXT)
                    .size(16.0)
                    .strong(),
            );
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "Inicie um jogo e toque em Atualizar lista. O otimizador não fecha nada.",
                )
                .color(MUTED)
                .size(13.0),
            );
        } else {
            for game in &report.games {
                egui::Frame::new()
                    .fill(BG_ROW)
                    .corner_radius(10.0)
                    .inner_margin(egui::Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
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
                    });
                ui.add_space(6.0);
            }
        }

        if !report.reclaim_candidates.is_empty() {
            ui.add_space(8.0);
            ui.label(
                RichText::new(format!(
                    "Pode ceder RAM · {} navegador(es)",
                    report.reclaim_candidates.len()
                ))
                .color(MUTED)
                .size(12.5),
            );
            for proc in report.reclaim_candidates.iter().take(4) {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&proc.name).color(MUTED).size(12.5));
                    ui.label(
                        RichText::new(format_mib(proc.memory_bytes))
                            .color(MUTED)
                            .size(12.5),
                    );
                });
            }
        }
    }
}

fn section_label(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).color(MUTED).size(12.0).strong());
    ui.add_space(6.0);
}

fn status_chip(ui: &mut egui::Ui, watching: bool) {
    let (label, color) = if watching {
        ("Cuidando", OK)
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
        .inner_margin(egui::Margin::symmetric(12, 6))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let dot = ui.allocate_response(Vec2::splat(8.0), Sense::hover());
                ui.painter().circle_filled(dot.rect.center(), 3.5, color);
                ui.label(RichText::new(label).color(color).size(12.5).strong());
            });
        });
}

fn metric_pill(ui: &mut egui::Ui, label: &str, value: &str, accent: Color32) {
    egui::Frame::new()
        .fill(BG_RAISED)
        .stroke(Stroke::new(1.0, STROKE))
        .corner_radius(10.0)
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(label).color(MUTED).size(12.0));
                ui.label(RichText::new(value).color(accent).strong().size(14.0));
            });
        });
}

fn spawn_tray_pump(
    ctx: egui::Context,
    signals: Arc<TraySignals>,
    show_id: MenuId,
    optimize_id: MenuId,
    quit_id: MenuId,
) {
    thread::spawn(move || loop {
        let mut woke = false;
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            woke = true;
            if event.id == show_id {
                signals.show.store(true, Ordering::SeqCst);
            } else if event.id == optimize_id {
                signals.optimize.store(true, Ordering::SeqCst);
            } else if event.id == quit_id {
                signals.quit.store(true, Ordering::SeqCst);
            }
        }
        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            match event {
                TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                }
                | TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } => {
                    woke = true;
                    signals.show.store(true, Ordering::SeqCst);
                }
                _ => {}
            }
        }
        if woke {
            ctx.request_repaint();
        }
        thread::sleep(Duration::from_millis(80));
    });
}

fn spawn_worker(
    config_path: PathBuf,
    ui_rx: Receiver<UiCmd>,
    worker_tx: Sender<WorkerMsg>,
    watch_flag: Arc<AtomicBool>,
) {
    thread::spawn(move || {
        let mut interval = 45u64;
        let mut trim_game = false;
        let mut last_watch = Instant::now() - Duration::from_secs(10_000);
        loop {
            match ui_rx.recv_timeout(Duration::from_millis(200)) {
                Ok(UiCmd::Scan) => {
                    let _ = worker_tx.send(WorkerMsg::Busy(true));
                    let config = AppConfig::load_or_default(&config_path).unwrap_or_default();
                    let report = optimize_system(&OptimizeRequest {
                        config,
                        dry_run: true,
                    });
                    let status = if report.games.is_empty() {
                        "Nenhum jogo aberto agora.".into()
                    } else {
                        format!("{} jogo(s) pronto(s) para otimizar.", report.games.len())
                    };
                    let _ = worker_tx.send(WorkerMsg::Status(status));
                    let _ = worker_tx.send(WorkerMsg::Report(report));
                    let _ = worker_tx.send(WorkerMsg::Busy(false));
                }
                Ok(UiCmd::Optimize { trim_game: tg }) => {
                    trim_game = tg;
                    let _ = worker_tx.send(WorkerMsg::Busy(true));
                    let mut config = AppConfig::load_or_default(&config_path).unwrap_or_default();
                    config.trim_game_memory = trim_game;
                    let report = optimize_system(&OptimizeRequest {
                        config,
                        dry_run: false,
                    });
                    let ok = report.results.iter().filter(|r| r.ok).count();
                    let fail = report.results.iter().filter(|r| !r.ok).count();
                    let status = if report.games.is_empty() {
                        "Nenhum jogo aberto para otimizar.".into()
                    } else if fail == 0 {
                        format!("Tudo certo — {} jogo(s) otimizado(s).", report.games.len())
                    } else {
                        format!(
                            "Otimizou com avisos ({ok} ok, {fail} falha). Tente como administrador."
                        )
                    };
                    let _ = worker_tx.send(WorkerMsg::Status(status));
                    let _ = worker_tx.send(WorkerMsg::Report(report));
                    let _ = worker_tx.send(WorkerMsg::Busy(false));
                }
                Ok(UiCmd::StartWatch {
                    interval: secs,
                    trim_game: tg,
                }) => {
                    interval = secs.max(15);
                    trim_game = tg;
                    watch_flag.store(true, Ordering::SeqCst);
                    last_watch = Instant::now() - Duration::from_secs(10_000);
                    let _ = worker_tx.send(WorkerMsg::Watching(true));
                }
                Ok(UiCmd::StopWatch) => {
                    watch_flag.store(false, Ordering::SeqCst);
                    let _ = worker_tx.send(WorkerMsg::Watching(false));
                }
                Ok(UiCmd::CleanCache) => {
                    let _ = worker_tx.send(WorkerMsg::Busy(true));
                    let report = cache::clean_user_caches();
                    let _ = worker_tx.send(WorkerMsg::Cache(report));
                    let _ = worker_tx.send(WorkerMsg::Busy(false));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if watch_flag.load(Ordering::SeqCst)
                        && last_watch.elapsed() >= Duration::from_secs(interval.max(15))
                    {
                        last_watch = Instant::now();
                        let mut config =
                            AppConfig::load_or_default(&config_path).unwrap_or_default();
                        config.trim_game_memory = trim_game;
                        let report = optimize_system(&OptimizeRequest {
                            config,
                            dry_run: false,
                        });
                        let _ = worker_tx.send(WorkerMsg::Report(report));
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });
}
