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
use crate::optimize::{optimize_system, OptimizeReport, OptimizeRequest};
use crate::report::format_mib;

/// Warm amber — primary actions / brand energy.
const ACCENT: Color32 = Color32::from_rgb(232, 168, 56);
const ACCENT_SOFT: Color32 = Color32::from_rgb(64, 48, 22);
/// Calm sage for “all good” status.
const OK: Color32 = Color32::from_rgb(110, 186, 140);
const BG: Color32 = Color32::from_rgb(22, 24, 28);
const BG_RAISED: Color32 = Color32::from_rgb(32, 35, 41);
const BG_ROW: Color32 = Color32::from_rgb(40, 44, 52);
const STROKE: Color32 = Color32::from_rgb(58, 62, 72);
const TEXT: Color32 = Color32::from_rgb(240, 236, 228);
const MUTED: Color32 = Color32::from_rgb(148, 152, 160);
const WARN: Color32 = Color32::from_rgb(220, 140, 70);

enum WorkerMsg {
    Report(OptimizeReport),
    Log(String),
    Watching(bool),
}

enum UiCmd {
    Optimize { trim_game: bool },
    Scan,
    StartWatch { interval: u64, trim_game: bool },
    StopWatch,
}

/// Launch the native GUI (blocks until quit).
pub fn run(config_path: PathBuf) -> eframe::Result<()> {
    let icon = tray_rgba_icon();
    let options = NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([720.0, 460.0])
            .with_min_inner_size([640.0, 400.0])
            .with_title("Game Optimizer")
            .with_icon(eframe_icon()),
        ..Default::default()
    };

    eframe::run_native(
        "Game Optimizer",
        options,
        Box::new(move |cc| {
            install_fonts(&cc.egui_ctx);
            style_visuals(&cc.egui_ctx);
            Ok(Box::new(GuiApp::new(config_path, icon)) as Box<dyn App>)
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
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(52, 56, 66);
    visuals.widgets.active.bg_fill = ACCENT_SOFT;
    visuals.selection.bg_fill = ACCENT_SOFT;
    visuals.extreme_bg_color = BG_RAISED;
    visuals.widgets.inactive.corner_radius = CornerRadius::same(6);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(6);
    visuals.widgets.active.corner_radius = CornerRadius::same(6);
    ctx.set_visuals(visuals);

    ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.spacing.item_spacing = Vec2::new(8.0, 6.0);
        style.spacing.button_padding = Vec2::new(12.0, 6.0);
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
    config_path: PathBuf,
    report: Option<OptimizeReport>,
    logs: Vec<String>,
    watching: bool,
    watch_interval: u64,
    trim_game_memory: bool,
    autostart: bool,
    start_minimized_hint: bool,
    /// Must stay owned so the tray icon is not destroyed.
    #[allow(dead_code)]
    tray: Option<TrayIcon>,
    tray_show: MenuItem,
    tray_optimize: MenuItem,
    tray_quit: MenuItem,
    ui_tx: Sender<UiCmd>,
    worker_rx: Receiver<WorkerMsg>,
    watch_flag: Arc<AtomicBool>,
    last_pulse: Instant,
    started: Instant,
}

impl GuiApp {
    fn new(config_path: PathBuf, icon: Icon) -> Self {
        let autostart =
            autostart::ensure_default_enabled().unwrap_or_else(|_| autostart::is_enabled());

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
            config_path,
            report: None,
            logs: vec!["Pronto. O X ou Minimizar enviam para a bandeja.".into()],
            watching: false,
            watch_interval: 45,
            trim_game_memory: false,
            autostart,
            start_minimized_hint: false,
            tray,
            tray_show,
            tray_optimize,
            tray_quit,
            ui_tx,
            worker_rx,
            watch_flag,
            last_pulse: Instant::now(),
            started: Instant::now(),
        };
        let _ = app.ui_tx.send(UiCmd::Scan);
        app
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
            }
        }
    }

    fn poll_tray(&mut self, ctx: &egui::Context) {
        if let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == self.tray_show.id() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            } else if event.id == self.tray_optimize.id() {
                let _ = self.ui_tx.send(UiCmd::Optimize {
                    trim_game: self.trim_game_memory,
                });
            } else if event.id == self.tray_quit.id() {
                self.watch_flag.store(false, Ordering::SeqCst);
                std::process::exit(0);
            }
        }

        if let Ok(TrayIconEvent::DoubleClick { .. }) = TrayIconEvent::receiver().try_recv() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
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
        self.poll_worker();
        self.poll_tray(ctx);

        let close_requested = ctx.input(|i| i.viewport().close_requested());
        if close_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            if !self.start_minimized_hint {
                self.push_log("Na bandeja. Clique duplo no ícone para reabrir.");
                self.start_minimized_hint = true;
            }
        }

        if self.last_pulse.elapsed() > Duration::from_millis(250) {
            ctx.request_repaint();
            self.last_pulse = Instant::now();
        }
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
    }
}

impl GuiApp {
    fn draw_header(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let pulse = ((self.started.elapsed().as_secs_f32() * 1.6).sin() + 1.0) * 0.5;
        let brand_glow = Color32::from_rgba_unmultiplied(
            ACCENT.r(),
            ACCENT.g(),
            ACCENT.b(),
            40 + (pulse * 50.0) as u8,
        );

        ui.horizontal(|ui| {
            // Brand mark
            let mark = ui.allocate_response(Vec2::splat(28.0), Sense::hover());
            ui.painter()
                .circle_filled(mark.rect.center(), 11.0, brand_glow);
            ui.painter().circle_filled(mark.rect.center(), 7.5, ACCENT);

            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("Game Optimizer")
                        .font(FontId::new(22.0, FontFamily::Proportional))
                        .color(TEXT)
                        .strong(),
                );
                ui.label(
                    RichText::new("Jogos abertos · sem fechar nada")
                        .color(MUTED)
                        .size(12.0),
                );
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let primary = ui.add_sized(
                    [148.0, 34.0],
                    egui::Button::new(
                        RichText::new("Otimizar")
                            .strong()
                            .color(Color32::from_rgb(28, 22, 12)),
                    )
                    .fill(ACCENT)
                    .corner_radius(8.0),
                );
                if primary.clicked() {
                    let _ = self.ui_tx.send(UiCmd::Optimize {
                        trim_game: self.trim_game_memory,
                    });
                }

                ui.add_space(8.0);
                status_chip(ui, self.watching);

                if ui
                    .add(
                        egui::Button::new(RichText::new("Bandeja").color(MUTED).size(12.0))
                            .fill(BG_RAISED)
                            .corner_radius(6.0),
                    )
                    .on_hover_text("Minimizar para a bandeja do sistema")
                    .clicked()
                {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                }
            });
        });

        ui.add_space(8.0);
        // Compact stats strip — fills the “empty” feel with useful density.
        ui.horizontal(|ui| {
            metric_pill(ui, "Jogos", &self.game_count().to_string(), ACCENT);
            metric_pill(ui, "RAM livre", &self.reclaim_count().to_string(), OK);
            metric_pill(
                ui,
                "Vigilância",
                &format!("{}s", self.watch_interval),
                MUTED,
            );
        });
    }

    fn draw_body(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) {
        let available = ui.available_height();
        ui.horizontal(|ui| {
            ui.set_min_height(available);

            // Main column ~68%
            ui.vertical(|ui| {
                ui.set_width(ui.available_width() * 0.68 - 6.0);
                ui.set_min_height(available);

                section_label(ui, "Jogos detectados");
                egui::Frame::new()
                    .fill(BG_RAISED)
                    .stroke(Stroke::new(1.0, STROKE))
                    .corner_radius(10.0)
                    .inner_margin(10.0)
                    .show(ui, |ui| {
                        let list_h = (available * 0.42).clamp(88.0, 160.0);
                        egui::ScrollArea::vertical()
                            .max_height(list_h)
                            .auto_shrink([false; 2])
                            .show(ui, |ui| {
                                self.draw_games(ui);
                            });
                    });

                ui.add_space(8.0);
                section_label(ui, "Atividade");
                egui::Frame::new()
                    .fill(BG_RAISED)
                    .stroke(Stroke::new(1.0, STROKE))
                    .corner_radius(10.0)
                    .inner_margin(8.0)
                    .show(ui, |ui| {
                        // Fill remaining height so no dead zone.
                        let rest = ui.available_height().max(72.0);
                        egui::ScrollArea::vertical()
                            .max_height(rest)
                            .auto_shrink([false; 2])
                            .stick_to_bottom(true)
                            .show(ui, |ui| {
                                ui.set_min_width(ui.available_width());
                                for line in self.logs.iter().rev().take(40).rev() {
                                    ui.label(RichText::new(line).color(MUTED).size(12.0));
                                }
                            });
                    });
            });

            ui.add_space(10.0);

            // Side column
            ui.vertical(|ui| {
                ui.set_width(ui.available_width());
                ui.set_min_height(available);

                section_label(ui, "Ações");
                egui::Frame::new()
                    .fill(BG_RAISED)
                    .stroke(Stroke::new(1.0, STROKE))
                    .corner_radius(10.0)
                    .inner_margin(10.0)
                    .show(ui, |ui| {
                        if ui
                            .add_sized(
                                [ui.available_width(), 32.0],
                                egui::Button::new(RichText::new("Atualizar").color(TEXT))
                                    .fill(BG_ROW)
                                    .corner_radius(7.0),
                            )
                            .clicked()
                        {
                            let _ = self.ui_tx.send(UiCmd::Scan);
                        }
                        ui.add_space(4.0);
                        if self.watching {
                            if ui
                                .add_sized(
                                    [ui.available_width(), 32.0],
                                    egui::Button::new(
                                        RichText::new("Parar vigilância").color(TEXT),
                                    )
                                    .fill(Color32::from_rgb(72, 42, 36))
                                    .corner_radius(7.0),
                                )
                                .clicked()
                            {
                                let _ = self.ui_tx.send(UiCmd::StopWatch);
                            }
                        } else if ui
                            .add_sized(
                                [ui.available_width(), 32.0],
                                egui::Button::new(
                                    RichText::new("Vigiar automaticamente").color(TEXT),
                                )
                                .fill(BG_ROW)
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

                ui.add_space(8.0);
                section_label(ui, "Ajustes");
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

                        ui.checkbox(
                            &mut self.trim_game_memory,
                            RichText::new("Trim memória do jogo").color(TEXT).size(13.0),
                        );
                        ui.label(
                            RichText::new("Pode engasgar brevemente ao paginar.")
                                .color(WARN)
                                .size(11.0),
                        );

                        ui.add_space(6.0);
                        ui.label(
                            RichText::new("Intervalo (segundos)")
                                .color(MUTED)
                                .size(12.0),
                        );
                        ui.add(
                            egui::Slider::new(&mut self.watch_interval, 15..=120)
                                .clamping(egui::SliderClamping::Always),
                        );

                        ui.add_space(8.0);
                        ui.label(
                            RichText::new(self.config_path.display().to_string())
                                .color(MUTED)
                                .size(10.0)
                                .monospace(),
                        );
                    });
            });
        });
    }

    fn draw_games(&self, ui: &mut egui::Ui) {
        let Some(report) = &self.report else {
            ui.label(RichText::new("Carregando…").color(MUTED));
            return;
        };

        if report.games.is_empty() {
            ui.label(
                RichText::new("Nenhum jogo aberto. Inicie um jogo e toque em Atualizar.")
                    .color(MUTED)
                    .size(13.0),
            );
        } else {
            for game in &report.games {
                egui::Frame::new()
                    .fill(BG_ROW)
                    .corner_radius(8.0)
                    .inner_margin(egui::Margin::symmetric(10, 8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&game.name).color(TEXT).strong().size(14.0));
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
                        ui.label(
                            RichText::new(format!("PID {}", game.pid))
                                .color(MUTED)
                                .size(11.0)
                                .monospace(),
                        );
                    });
                ui.add_space(4.0);
            }
        }

        if !report.reclaim_candidates.is_empty() {
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!(
                    "Pode liberar RAM · {} app(s)",
                    report.reclaim_candidates.len()
                ))
                .color(MUTED)
                .size(12.0),
            );
            for proc in report.reclaim_candidates.iter().take(4) {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&proc.name).color(MUTED).size(12.0));
                    ui.label(
                        RichText::new(format_mib(proc.memory_bytes))
                            .color(MUTED)
                            .size(12.0),
                    );
                });
            }
        }
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
) {
    thread::spawn(move || {
        let mut interval = 45u64;
        let mut trim_game = false;
        loop {
            match ui_rx.recv_timeout(Duration::from_millis(200)) {
                Ok(UiCmd::Scan) => {
                    let config = AppConfig::load_or_default(&config_path).unwrap_or_default();
                    let report = optimize_system(&OptimizeRequest {
                        config,
                        dry_run: true,
                    });
                    let _ = worker_tx.send(WorkerMsg::Log(format!(
                        "Scan: {} jogo(s), {} candidato(s).",
                        report.games.len(),
                        report.reclaim_candidates.len()
                    )));
                    let _ = worker_tx.send(WorkerMsg::Report(report));
                }
                Ok(UiCmd::Optimize { trim_game: tg }) => {
                    trim_game = tg;
                    let mut config = AppConfig::load_or_default(&config_path).unwrap_or_default();
                    config.trim_game_memory = trim_game;
                    let report = optimize_system(&OptimizeRequest {
                        config,
                        dry_run: false,
                    });
                    let ok = report.results.iter().filter(|r| r.ok).count();
                    let fail = report.results.iter().filter(|r| !r.ok).count();
                    let _ = worker_tx.send(WorkerMsg::Log(format!(
                        "Otimização: {ok} ok, {fail} falha(s), {} jogo(s).",
                        report.games.len()
                    )));
                    let _ = worker_tx.send(WorkerMsg::Report(report));
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
                }
                Ok(UiCmd::StopWatch) => {
                    watch_flag.store(false, Ordering::SeqCst);
                    let _ = worker_tx.send(WorkerMsg::Watching(false));
                    let _ = worker_tx.send(WorkerMsg::Log("Vigilância parada.".into()));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if watch_flag.load(Ordering::SeqCst) {
                        let mut config =
                            AppConfig::load_or_default(&config_path).unwrap_or_default();
                        config.trim_game_memory = trim_game;
                        let report = optimize_system(&OptimizeRequest {
                            config,
                            dry_run: false,
                        });
                        let _ = worker_tx.send(WorkerMsg::Report(report));
                        thread::sleep(Duration::from_secs(interval.max(15)));
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });
}
