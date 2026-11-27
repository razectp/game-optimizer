//! StatusNotifierItem tray for Linux desktop environments.

use std::sync::mpsc::{self, Receiver, Sender};

use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::StandardItem;
use ksni::{Category, Icon, MenuItem, OfflineReason, Tray};

#[derive(Clone, Copy)]
pub(super) enum Action {
    Open,
    Scan,
    Quit,
    Online,
    Offline,
}

pub(super) struct LinuxTray {
    handle: Handle<TrayItem>,
    pub(super) events: Receiver<Action>,
    pub(super) online: bool,
}

impl LinuxTray {
    pub(super) fn start(ctx: egui::Context) -> Result<Self, ksni::Error> {
        let (tx, events) = mpsc::channel();
        let icons = [16, 24, 32, 48].map(|size| {
            let mut data = super::application_icon(size).into_raw();
            for pixel in data.as_chunks_mut::<4>().0 {
                pixel.rotate_right(1);
            }
            Icon {
                width: size as i32,
                height: size as i32,
                data,
            }
        });
        let handle = TrayItem { tx, ctx, icons }.spawn()?;
        Ok(Self {
            handle,
            events,
            online: true,
        })
    }

    pub(super) fn available(&self) -> bool {
        self.online && !self.handle.is_closed()
    }
}

struct TrayItem {
    tx: Sender<Action>,
    ctx: egui::Context,
    icons: [Icon; 4],
}

impl TrayItem {
    fn dispatch(&self, action: Action) {
        if self.tx.send(action).is_ok() {
            self.ctx.request_repaint();
        }
    }
}

impl Tray for TrayItem {
    fn id(&self) -> String {
        "game-optimizer".into()
    }

    fn title(&self) -> String {
        "Game Optimizer".into()
    }

    fn category(&self) -> Category {
        Category::ApplicationStatus
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        self.icons.to_vec()
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.dispatch(Action::Open);
    }

    fn watcher_online(&self) {
        self.dispatch(Action::Online);
    }

    fn watcher_offline(&self, _reason: OfflineReason) -> bool {
        self.dispatch(Action::Offline);
        true
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        [
            ("Abrir Game Optimizer", Action::Open),
            ("Buscar jogos", Action::Scan),
            ("Sair", Action::Quit),
        ]
        .into_iter()
        .map(|(label, action)| {
            StandardItem {
                label: label.into(),
                activate: Box::new(move |item: &mut Self| item.dispatch(action)),
                ..Default::default()
            }
            .into()
        })
        .collect()
    }
}
