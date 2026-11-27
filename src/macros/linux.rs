//! X11 input and window discovery for Linux macro instances.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{self, Atom, AtomEnum, ConnectionExt as _, Window};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;
use xkeysym::key;

use super::WindowInspectResult;

#[cfg(target_arch = "x86_64")]
#[path = "wine.rs"]
mod wine;

thread_local! {
    static X11: RefCell<Option<Backend>> = RefCell::new(Backend::open());
}

struct Backend {
    conn: RustConnection,
    root: Window,
    pid_atom: Atom,
    clients_atom: Atom,
    title_atom: Atom,
    wine_atom: Atom,
    window_cache: HashMap<u32, WindowInspectResult>,
    refreshed: Option<Instant>,
    targets: HashMap<u32, Window>,
    held: HashMap<(u32, String), Held>,
    #[cfg(target_arch = "x86_64")]
    wine_launches: HashMap<u32, Option<wine::WineLaunch>>,
    #[cfg(target_arch = "x86_64")]
    wine_bridges: wine::WineBridges,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Input {
    Key(u8),
    Button(u8),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Delivery {
    XTest,
    Window(Window),
    #[cfg(target_arch = "x86_64")]
    Wine(Window),
}

#[derive(Clone, Copy)]
struct Held {
    input: Input,
    delivery: Delivery,
}

impl Backend {
    fn open() -> Option<Self> {
        if std::env::var("XDG_SESSION_TYPE").as_deref() == Ok("wayland") {
            return None;
        }

        let (conn, screen) = x11rb::connect(None).ok()?;
        conn.xtest_get_version(2, 2).ok()?.reply().ok()?;
        let root = conn.setup().roots.get(screen)?.root;
        let atom = |name| {
            conn.intern_atom(false, name)
                .ok()?
                .reply()
                .ok()
                .map(|reply| reply.atom)
        };
        let pid_atom = atom(b"_NET_WM_PID")?;
        let clients_atom = atom(b"_NET_CLIENT_LIST")?;
        let title_atom = atom(b"_NET_WM_NAME")?;
        let wine_atom = atom(b"_WINE_HWND_STYLE")?;
        Some(Self {
            conn,
            root,
            pid_atom,
            clients_atom,
            title_atom,
            wine_atom,
            window_cache: HashMap::new(),
            refreshed: None,
            targets: HashMap::new(),
            held: HashMap::new(),
            #[cfg(target_arch = "x86_64")]
            wine_launches: HashMap::new(),
            #[cfg(target_arch = "x86_64")]
            wine_bridges: wine::WineBridges::default(),
        })
    }

    fn pid(&self, window: Window) -> Option<u32> {
        self.conn
            .get_property(false, window, self.pid_atom, AtomEnum::CARDINAL, 0, 1)
            .ok()?
            .reply()
            .ok()?
            .value32()?
            .next()
            .filter(|&pid| pid != 0)
    }

    fn client_window(&self, mut window: Window) -> Option<Window> {
        for _ in 0..16 {
            if window <= 1 || window == self.root {
                return None;
            }
            if self.pid(window).is_some() {
                return Some(window);
            }
            window = self.conn.query_tree(window).ok()?.reply().ok()?.parent;
        }
        None
    }

    fn focused_window(&self) -> Option<Window> {
        let focus = self.conn.get_input_focus().ok()?.reply().ok()?.focus;
        self.client_window(focus)
    }

    fn focused_pid(&self) -> Option<u32> {
        self.pid(self.focused_window()?)
    }

    fn title(&self, window: Window) -> Option<String> {
        [self.title_atom, AtomEnum::WM_NAME.into()]
            .into_iter()
            .find_map(|atom| {
                let reply = self
                    .conn
                    .get_property(false, window, atom, AtomEnum::ANY, 0, 1024)
                    .ok()?
                    .reply()
                    .ok()?;
                let title = String::from_utf8_lossy(&reply.value).trim().to_string();
                (!title.is_empty()).then_some(title)
            })
    }

    fn inspect(&mut self, pid: u32) -> WindowInspectResult {
        if self
            .refreshed
            .is_none_or(|last| last.elapsed() >= Duration::from_millis(300))
        {
            self.refreshed = Some(Instant::now());
            self.window_cache.clear();
            self.targets.clear();
            let clients = self
                .conn
                .get_property(
                    false,
                    self.root,
                    self.clients_atom,
                    AtomEnum::WINDOW,
                    0,
                    4096,
                )
                .ok()
                .and_then(|cookie| cookie.reply().ok());
            let listed: Vec<_> = clients
                .as_ref()
                .and_then(|reply| reply.value32())
                .map(Iterator::collect)
                .unwrap_or_default();
            let windows = if listed.is_empty() {
                let mut children = self
                    .conn
                    .query_tree(self.root)
                    .ok()
                    .and_then(|cookie| cookie.reply().ok())
                    .map_or_else(Vec::new, |reply| reply.children);
                // QueryTree is ordered by stacking. Keep the selected client
                // stable even when another window of the process takes focus.
                children.sort_unstable();
                children
            } else {
                listed
            };
            for window in windows {
                let Some(pid) = self.pid(window) else {
                    continue;
                };
                let title = self.title(window);
                let status = self.window_cache.entry(pid).or_default();
                status.count += 1;
                self.targets.entry(pid).or_insert(window);
                if status.primary_title.is_none() && title.is_some() {
                    status.primary_title = title;
                    self.targets.insert(pid, window);
                }
            }
        }
        self.window_cache.get(&pid).cloned().unwrap_or_default()
    }

    fn input(&self, code: &str) -> Option<Input> {
        match code {
            "MOUSE_LEFT" => return Some(Input::Button(1)),
            "MOUSE_RIGHT" => return Some(Input::Button(3)),
            _ => {}
        }
        let symbol = keysym(code)?;
        let min = self.conn.setup().min_keycode;
        let max = self.conn.setup().max_keycode;
        let mapping = self
            .conn
            .get_keyboard_mapping(min, max.checked_sub(min)?.checked_add(1)?)
            .ok()?
            .reply()
            .ok()?;
        let width = usize::from(mapping.keysyms_per_keycode);
        if width == 0 {
            return None;
        }
        let index = mapping
            .keysyms
            .chunks_exact(width)
            .position(|key| key.contains(&symbol))?;
        Some(Input::Key(min.checked_add(u8::try_from(index).ok()?)?))
    }

    fn is_down(&self, code: &str) -> bool {
        let Some(input @ Input::Key(key)) = self.input(code) else {
            return false;
        };
        if self
            .held
            .values()
            .any(|held| held.delivery == Delivery::XTest && input == held.input)
        {
            return false;
        }
        self.conn
            .query_keymap()
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .is_some_and(|state| state.keys[usize::from(key / 8)] & (1 << (key % 8)) != 0)
    }

    fn emit(&self, input: Input, down: bool) -> bool {
        let (event, code) = match (input, down) {
            (Input::Key(code), true) => (xproto::KEY_PRESS_EVENT, code),
            (Input::Key(code), false) => (xproto::KEY_RELEASE_EVENT, code),
            (Input::Button(code), true) => (xproto::BUTTON_PRESS_EVENT, code),
            (Input::Button(code), false) => (xproto::BUTTON_RELEASE_EVENT, code),
        };
        self.conn
            .xtest_fake_input(event, code, 0, self.root, 0, 0, 0)
            .is_ok_and(|cookie| cookie.check().is_ok())
    }

    fn pointer_window(&self) -> Option<Window> {
        let mut window = self.root;
        for _ in 0..16 {
            window = self.conn.query_pointer(window).ok()?.reply().ok()?.child;
            if window == x11rb::NONE {
                return None;
            }
            if self.pid(window).is_some() {
                return Some(window);
            }
        }
        None
    }

    fn background(&mut self, pid: u32) -> Option<Delivery> {
        self.inspect(pid);
        let window = *self.targets.get(&pid)?;
        if self.pid(window) != Some(pid) {
            return None;
        }

        #[cfg(target_arch = "x86_64")]
        if let Some(launch) = self
            .wine_launches
            .entry(pid)
            .or_insert_with(|| wine::WineLaunch::for_pid(pid))
        {
            return self
                .wine_bridges
                .ready(pid, launch)
                .then_some(Delivery::Wine(window));
        }
        let wine_window = self
            .conn
            .get_property(false, window, self.wine_atom, AtomEnum::CARDINAL, 0, 1)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .is_some_and(|reply| reply.value_len != 0);
        (!wine_window).then_some(Delivery::Window(window))
    }

    fn emit_to_window(&self, window: Window, input: Input, down: bool) -> bool {
        use xproto::EventMask;
        let (response_type, detail, mask) = match (input, down) {
            (Input::Key(code), true) => (xproto::KEY_PRESS_EVENT, code, EventMask::KEY_PRESS),
            (Input::Key(code), false) => (xproto::KEY_RELEASE_EVENT, code, EventMask::KEY_RELEASE),
            (Input::Button(code), true) => {
                (xproto::BUTTON_PRESS_EVENT, code, EventMask::BUTTON_PRESS)
            }
            (Input::Button(code), false) => (
                xproto::BUTTON_RELEASE_EVENT,
                code,
                EventMask::BUTTON_RELEASE,
            ),
        };
        let Some(geometry) = self
            .conn
            .get_geometry(window)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
        else {
            return false;
        };
        let event = xproto::KeyPressEvent {
            response_type,
            detail,
            sequence: 0,
            time: x11rb::CURRENT_TIME,
            root: self.root,
            event: window,
            child: x11rb::NONE,
            root_x: 0,
            root_y: 0,
            event_x: (geometry.width / 2) as i16,
            event_y: (geometry.height / 2) as i16,
            state: xproto::KeyButMask::default(),
            same_screen: true,
        };
        self.conn
            .send_event(false, window, mask, event)
            .is_ok_and(|cookie| cookie.check().is_ok())
    }

    fn dispatch(&mut self, pid: u32, code: &str, held: Held, down: bool) -> bool {
        match held.delivery {
            Delivery::XTest => self.emit(held.input, down),
            Delivery::Window(window) => {
                self.pid(window) == Some(pid) && self.emit_to_window(window, held.input, down)
            }
            #[cfg(target_arch = "x86_64")]
            Delivery::Wine(window) => {
                self.pid(window) == Some(pid) && self.wine_bridges.send(pid, window, code, down)
            }
        }
    }

    fn send_down(&mut self, pid: u32, code: &str) -> bool {
        let Some(input) = self.input(code) else {
            return false;
        };
        let target = (pid, code.to_owned());
        if self.held.contains_key(&target) {
            return false;
        }
        self.inspect(pid);
        let Some(&window) = self.targets.get(&pid) else {
            return false;
        };
        let is_focused = |backend: &Self| {
            backend.pid(window) == Some(pid)
                && backend.focused_window() == Some(window)
                && (!matches!(input, Input::Button(_)) || backend.pointer_window() == Some(window))
        };
        let mut focused = false;
        let mut xtest_sent = false;
        // Keep the focus check and global XTEST injection together on the X server.
        // Targeted background messages never need a server grab.
        if is_focused(self)
            && self
                .conn
                .grab_server()
                .is_ok_and(|cookie| cookie.check().is_ok())
        {
            focused = is_focused(self);
            let duplicate = self
                .held
                .values()
                .any(|held| held.delivery == Delivery::XTest && input == held.input);
            xtest_sent = focused && !duplicate && self.emit(input, true);
            let _ = self.conn.ungrab_server();
            let _ = self.conn.flush();
        }

        let delivery = if xtest_sent {
            Delivery::XTest
        } else if !focused {
            let Some(delivery) = self.background(pid) else {
                return false;
            };
            if !self.dispatch(pid, code, Held { input, delivery }, true) {
                return false;
            }
            delivery
        } else {
            return false;
        };
        self.held.insert(target, Held { input, delivery });
        true
    }

    fn release_unfocused(&mut self, focus: Option<Window>) {
        let pointer = self
            .held
            .values()
            .any(|held| held.delivery == Delivery::XTest && matches!(held.input, Input::Button(_)))
            .then(|| self.pointer_window())
            .flatten();
        let stale: Vec<_> = self
            .held
            .iter()
            .filter(|((pid, _), held)| {
                let target = self.targets.get(pid).copied();
                held.delivery == Delivery::XTest
                    && (target.is_none()
                        || focus != target
                        || (matches!(held.input, Input::Button(_)) && pointer != target))
            })
            .map(|(target, _)| target.clone())
            .collect();
        for (pid, code) in stale {
            self.send_up(pid, &code);
        }
    }

    fn send_up(&mut self, pid: u32, code: &str) -> bool {
        self.held
            .remove(&(pid, code.to_owned()))
            .is_some_and(|held| self.dispatch(pid, code, held, false))
    }
}

fn keysym(code: &str) -> Option<u32> {
    let bytes = code.as_bytes();
    if let [c] = bytes {
        return c
            .is_ascii_alphanumeric()
            .then_some(u32::from(c.to_ascii_lowercase()));
    }
    if let Some(number) = code
        .strip_prefix('F')
        .and_then(|key| key.parse::<u32>().ok())
    {
        return (1..=12).contains(&number).then(|| key::F1 + number - 1);
    }
    if let Some(number) = code
        .strip_prefix("NUM")
        .and_then(|key| key.parse::<u32>().ok())
    {
        return (number <= 9).then(|| key::KP_0 + number);
    }
    Some(match code {
        "ALT" => key::Alt_L,
        "BACKSPACE" => key::BackSpace,
        "CTRL" => key::Control_L,
        "DELETE" => key::Delete,
        "DOWN" => key::Down,
        "END" => key::End,
        "ENTER" => key::Return,
        "ESC" => key::Escape,
        "HOME" => key::Home,
        "INSERT" => key::Insert,
        "LEFT" => key::Left,
        "PAGEDOWN" => key::Page_Down,
        "PAGEUP" => key::Page_Up,
        "RIGHT" => key::Right,
        "SHIFT" => key::Shift_L,
        "SPACE" => key::space,
        "TAB" => key::Tab,
        "UP" => key::Up,
        _ => return None,
    })
}

pub(super) fn available() -> bool {
    X11.with(|backend| backend.borrow().is_some())
}

pub(super) fn sync_focus() -> Option<u32> {
    X11.with(|backend| {
        let mut backend = backend.borrow_mut();
        let backend = backend.as_mut()?;
        let focus = backend.focused_window();
        backend.release_unfocused(focus);
        let focus = focus?;
        let pid = backend.pid(focus)?;
        backend.inspect(pid);
        (backend.targets.get(&pid) == Some(&focus)).then_some(pid)
    })
}

pub(super) fn prepare_background(pid: u32) -> bool {
    X11.with(|backend| {
        backend
            .borrow_mut()
            .as_mut()
            .is_some_and(|backend| backend.background(pid).is_some())
    })
}

#[cfg(target_arch = "x86_64")]
pub(super) fn retain_games(live: &std::collections::HashSet<u32>) {
    X11.with(|backend| {
        if let Some(backend) = backend.borrow_mut().as_mut() {
            backend.wine_launches.retain(|pid, _| live.contains(pid));
            backend.wine_bridges.retain(|pid| live.contains(&pid));
        }
    });
}

pub(super) fn foreground_window_pid() -> Option<u32> {
    X11.with(|backend| backend.borrow().as_ref()?.focused_pid())
}

pub(super) fn is_key_physically_down(code: &str) -> bool {
    X11.with(|backend| {
        backend
            .borrow()
            .as_ref()
            .is_some_and(|backend| backend.is_down(code))
    })
}

pub(super) fn query_process_windows(pid: u32) -> WindowInspectResult {
    X11.with(|backend| {
        backend
            .borrow_mut()
            .as_mut()
            .map(|backend| backend.inspect(pid))
            .unwrap_or_default()
    })
}

pub(super) fn send_key_down(pid: u32, code: &str) -> usize {
    X11.with(|backend| {
        usize::from(
            backend
                .borrow_mut()
                .as_mut()
                .is_some_and(|backend| backend.send_down(pid, code)),
        )
    })
}

pub(super) fn send_key_up(pid: u32, code: &str) -> usize {
    X11.with(|backend| {
        usize::from(
            backend
                .borrow_mut()
                .as_mut()
                .is_some_and(|backend| backend.send_up(pid, code)),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires an isolated X11 server with XTEST"]
    fn foreground_and_background_events_target_only_the_game() {
        use x11rb::protocol::Event;
        use x11rb::wrapper::ConnectionExt as _;
        let backend = Backend::open().expect("X11 with XTEST");
        let conn = &backend.conn;
        let create_window = |pid| {
            let window = conn.generate_id().unwrap();
            conn.create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                window,
                backend.root,
                0,
                0,
                120,
                100,
                0,
                xproto::WindowClass::INPUT_OUTPUT,
                0,
                &xproto::CreateWindowAux::new()
                    .event_mask(xproto::EventMask::KEY_PRESS | xproto::EventMask::KEY_RELEASE),
            )
            .unwrap()
            .check()
            .unwrap();
            conn.change_property32(
                xproto::PropMode::REPLACE,
                window,
                backend.pid_atom,
                AtomEnum::CARDINAL,
                &[pid],
            )
            .unwrap()
            .check()
            .unwrap();
            conn.map_window(window).unwrap().check().unwrap();
            window
        };
        let focus = |window| {
            conn.set_input_focus(xproto::InputFocus::PARENT, window, x11rb::CURRENT_TIME)
                .unwrap()
                .check()
                .unwrap();
        };
        let game = create_window(42);
        let other = create_window(43);
        focus(game);
        assert_eq!(foreground_window_pid(), Some(42));
        assert_eq!(query_process_windows(42).count, 1);
        assert_eq!(send_key_down(42, "A"), 1);
        assert_eq!(send_key_up(42, "A"), 1);
        focus(other);
        assert_eq!(send_key_down(42, "A"), 1);
        assert_eq!(send_key_up(42, "A"), 1);
        assert_eq!(foreground_window_pid(), Some(43));
        let mut events = Vec::new();
        while let Some(event) = conn.poll_for_event().unwrap() {
            match event {
                Event::KeyPress(event) => events.push((true, event.event)),
                Event::KeyRelease(event) => events.push((false, event.event)),
                _ => {}
            }
        }
        assert_eq!(
            events,
            [(true, game), (false, game), (true, game), (false, game)]
        );
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    #[ignore = "requires the Wine receiver fixture on an isolated X server"]
    fn proton_background_dispatch_uses_the_wine_bridge() {
        let xid: u32 = std::env::var("MACRO_TEST_GAME_XID")
            .unwrap()
            .parse()
            .unwrap();
        let probe = Backend::open().expect("X11 with XTEST");
        let pid = probe.pid(xid).expect("Wine window PID");
        let focused = probe.focused_window().expect("focused fixture");
        assert_ne!(focused, xid);

        let deadline = Instant::now() + Duration::from_secs(8);
        while !prepare_background(pid) {
            assert!(Instant::now() < deadline, "the Wine bridge did not start");
            std::thread::sleep(Duration::from_millis(25));
        }
        X11.with(|backend| {
            let backend = backend.borrow();
            let backend = backend.as_ref().unwrap();
            assert_eq!(backend.targets.get(&pid), Some(&xid));
        });
        use crate::macros::{GameMacroProfile, MacroExecutionMode, MacroManager, MacroRule};
        let mut manager = MacroManager::new(std::collections::BTreeMap::from([(
            "fixture".into(),
            GameMacroProfile {
                mode: MacroExecutionMode::Parallel,
                auto_start: true,
                rules: vec![MacroRule {
                    key: "A".into(),
                    label: "Fixture".into(),
                    enabled: true,
                    interval_ms: 1_000,
                    hold_ms: 20,
                }],
                ..GameMacroProfile::default()
            },
        )]));
        let now = Instant::now();
        let games = [(pid, "fixture".into())];
        let status = manager.tick(&games, now);
        assert_eq!(
            status[0].total_triggers, 1,
            "scheduler state: {:?}",
            status[0]
        );
        assert_eq!(
            manager.tick(&games, now + Duration::from_millis(25))[0].total_triggers,
            1
        );
        manager.shutdown();
        assert_eq!(probe.focused_window(), Some(focused));
    }

    #[test]
    fn all_keyboard_rules_have_x11_keysyms() {
        for spec in super::super::SUPPORTED_KEYS
            .iter()
            .filter(|spec| spec.vk != 0)
        {
            assert!(keysym(spec.code).is_some(), "{}", spec.code);
        }
    }
}
