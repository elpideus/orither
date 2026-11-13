//! Input goes through the XDG desktop portal's RemoteDesktop interface, which the
//! Wayland desktops that allow remote control offer, and which asks the user in its
//! own dialog. The desktop hands back a token for persist mode 2, and with it the
//! next session starts without asking. It is kept in [`TOKEN`].
//!
//! The bus is blocking, so it lives on a thread of its own and orders reach it over a
//! channel. Input goes out without waiting for replies, so motion never queues up
//! behind round trips. The portal answers its requests with `Response` signals,
//! which the thread looks for while it waits on them.

use std::{
    collections::{HashSet, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering::Relaxed},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow};
use dbus::{
    Message, Path,
    arg::{AppendAll, PropMap, RefArg, Variant},
    blocking::{Connection, stdintf::org_freedesktop_dbus::Properties},
    channel::Sender as _,
    message::MatchRule,
};
use orither_link::PeerId;
use serde_json::{Value, json};
use tokio::{sync::oneshot, time::timeout};
use xkeysym::Keysym;

use super::{Button, Input, Key};
use crate::{features::Phone, preferences};

const DESKTOP: &str = "org.freedesktop.portal.Desktop";
const DESKTOP_PATH: &str = "/org/freedesktop/portal/desktop";
const REMOTE_DESKTOP: &str = "org.freedesktop.portal.RemoteDesktop";
const REQUEST: &str = "org.freedesktop.portal.Request";
const SESSION: &str = "org.freedesktop.portal.Session";
const CALL_TIMEOUT: Duration = Duration::from_secs(10);
/// Closing happens as Orither quits, which should not wait long on the portal.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(1);
const KEYBOARD: u32 = 1;
const POINTER: u32 = 2;
/// Asks the desktop to remember the user's answer until they take it back.
const PERSIST_UNTIL_REVOKED: u32 = 2;
/// The portal answers 0 for yes and 1 when the user said no. Anything else went wrong.
const ALLOWED: u32 = 0;
const REFUSED_BY_USER: u32 = 1;
const RELEASED: u32 = 0;
const PRESSED: u32 = 1;
const TOKEN: &str = "input-restore-token";
/// How long the desktop stays controlled without any input.
const IDLE_FOR: Duration = Duration::from_secs(10 * 60);
/// How long a phone's `open` waits before it hears what the user has to do.
const ANSWER_WITHIN: Duration = Duration::from_secs(2);
/// How often the thread looks for the portal's answers while it waits for them.
const WHILE_STARTING: Duration = Duration::from_millis(50);
/// How often it looks for the desktop ending a session, and for idleness.
const WHILE_RUNNING: Duration = Duration::from_secs(1);
/// Input held back while the desktop gets ready, at most.
const HOLD_AT_MOST: usize = 256;
/// A wheel notch scrolls 15 axis units. A notch for each 50 pixels a finger
/// scrolls feels like a laptop's touchpad.
const AXIS_PER_PIXEL: f64 = 15.0 / 50.0;
const LEFT: i32 = 0x110;
const RIGHT: i32 = 0x111;
const MIDDLE: i32 = 0x112;
const RETURN: u32 = 0xff0d;
const TAB: u32 = 0xff09;

const ASKING: &str = "Allow Orither on your computer to control it";
const REFUSED: &str = "Your computer did not allow it. Tap to ask again.";
const STOPPED: &str = "Control was stopped on your computer. Tap to start it again.";
const FAILED: &str = "Your computer could not start remote control. Tap to try again.";
const NO_PORTAL: &str = "This computer's desktop does not let other devices control it.";

#[derive(Default)]
struct Shared {
    supported: AtomicBool,
    active: AtomicBool,
}

impl Shared {
    fn state(&self) -> Value {
        json!({ "supported": self.supported.load(Relaxed), "active": self.active.load(Relaxed) })
    }
}

type Reply = oneshot::Sender<Result<(), String>>;

enum Order {
    Open { ask: bool, reply: Reply },
    Input(PeerId, Input),
    Release(PeerId),
    Stop(Sender<()>),
}

/// The thread that speaks to the portal.
pub struct Remote {
    orders: Sender<Order>,
    shared: Arc<Shared>,
}

impl Remote {
    pub fn start(phone: Phone) -> Self {
        let (orders, taken) = mpsc::channel();
        let shared = Arc::<Shared>::default();
        let theirs = shared.clone();
        std::thread::spawn(move || {
            if let Err(err) = Desktop::run(&taken, phone, &theirs) {
                tracing::warn!("no remote input: {err:#}");
            }
            theirs.supported.store(false, Relaxed);
            theirs.active.store(false, Relaxed);
        });
        Self { orders, shared }
    }

    pub fn state(&self) -> Value {
        self.shared.state()
    }

    pub async fn open(&self, ask: bool) -> Result<(), String> {
        let (reply, answer) = oneshot::channel();
        if self.orders.send(Order::Open { ask, reply }).is_err() {
            return Err(NO_PORTAL.into());
        }
        match timeout(ANSWER_WITHIN, answer).await {
            Ok(Ok(answer)) => answer,
            Ok(Err(_)) => Err(NO_PORTAL.into()),
            // Only the user's answer takes this long.
            Err(_) => Err(ASKING.into()),
        }
    }

    pub fn send(&self, peer: PeerId, input: Input) {
        let _ = self.orders.send(Order::Input(peer, input));
    }

    pub fn release(&self, peer: &PeerId) {
        let _ = self.orders.send(Order::Release(*peer));
    }

    pub fn stop(&self) {
        let (done, stopped) = mpsc::channel();
        if self.orders.send(Order::Stop(done)).is_ok() {
            let _ = stopped.recv_timeout(CLOSE_TIMEOUT * 2);
        }
    }
}

/// What the portal said about one of its requests.
struct Response {
    request: String,
    code: u32,
    session: Option<String>,
    token: Option<String>,
}

enum Heard {
    Response(Response),
    Closed(String),
}

#[derive(Clone, Copy)]
enum Step {
    Create,
    Select,
    Start,
}

enum State {
    Closed,
    /// Waiting for the portal to answer `request`. Every step after the first has a session.
    Starting { step: Step, request: String, session: Option<Path<'static>> },
    Running { session: Path<'static> },
    /// Reported to phones until one asks again.
    Failed(&'static str),
}

/// What is held down on the desktop, to let go of once nobody is there to.
#[derive(Default)]
struct Pressed {
    keys: HashSet<i32>,
    buttons: HashSet<i32>,
}

struct Desktop {
    bus: Connection,
    heard: Receiver<Heard>,
    phone: Phone,
    shared: Arc<Shared>,
    version: u32,
    devices: u32,
    state: State,
    waiting: Vec<Reply>,
    held_back: VecDeque<Input>,
    pressed: Pressed,
    /// The phone that sent input last.
    controller: Option<PeerId>,
    last_input: Instant,
    tokens: u32,
}

fn option(options: &mut PropMap, key: &str, value: impl RefArg + 'static) {
    options.insert(key.into(), Variant(Box::new(value)));
}

fn object(path: String) -> Result<Path<'static>> {
    Path::new(path).map_err(|err| anyhow!("odd session: {err}"))
}

impl Desktop {
    fn run(orders: &Receiver<Order>, phone: Phone, shared: &Arc<Shared>) -> Result<()> {
        let bus = Connection::new_session()?;
        let (tell, heard) = mpsc::channel();
        let responded = tell.clone();
        let on_response = move |(code, results): (u32, PropMap), _: &Connection, message: &Message| {
            let text = |key: &str| results.get(key).and_then(|value| value.0.as_str()).map(str::to_owned);
            let request = message.path().map(|path| path.to_string()).unwrap_or_default();
            let response = Response { request, code, session: text("session_handle"), token: text("restore_token") };
            responded.send(Heard::Response(response)).is_ok()
        };
        bus.add_match(MatchRule::new_signal(REQUEST, "Response"), on_response)?;
        let on_closed = move |(): (), _: &Connection, message: &Message| {
            let session = message.path().map(|path| path.to_string()).unwrap_or_default();
            tell.send(Heard::Closed(session)).is_ok()
        };
        bus.add_match(MatchRule::new_signal(SESSION, "Closed"), on_closed)?;

        let mut desktop = Self {
            bus,
            heard,
            phone,
            shared: shared.clone(),
            version: 1,
            devices: 0,
            state: State::Closed,
            waiting: Vec::new(),
            held_back: VecDeque::new(),
            pressed: Pressed::default(),
            controller: None,
            last_input: Instant::now(),
            tokens: 0,
        };
        desktop.check();
        loop {
            let order = match desktop.patience() {
                Some(patience) => orders.recv_timeout(patience),
                None => orders.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            match order {
                Ok(Order::Stop(done)) => {
                    desktop.close();
                    let _ = done.send(());
                    return Ok(());
                }
                Ok(order) => desktop.follow(order),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    desktop.close();
                    return Ok(());
                }
            }
            desktop.listen()?;
            if matches!(desktop.state, State::Running { .. }) && desktop.last_input.elapsed() > IDLE_FOR {
                desktop.close();
            }
        }
    }

    /// Reads what the desktop offers. Asking costs nothing and shows the user nothing.
    fn check(&mut self) -> bool {
        let portal = self.bus.with_proxy(DESKTOP, DESKTOP_PATH, CALL_TIMEOUT);
        let devices: Result<u32, _> = portal.get(REMOTE_DESKTOP, "AvailableDeviceTypes");
        self.version = portal.get(REMOTE_DESKTOP, "version").unwrap_or(1);
        self.devices = devices.unwrap_or(0) & (KEYBOARD | POINTER);
        let supported = self.devices & POINTER != 0;
        self.shared.supported.store(supported, Relaxed);
        supported
    }

    fn patience(&self) -> Option<Duration> {
        match self.state {
            State::Starting { .. } => Some(WHILE_STARTING),
            State::Running { .. } => Some(WHILE_RUNNING),
            State::Closed | State::Failed(_) => None,
        }
    }

    fn follow(&mut self, order: Order) {
        match order {
            Order::Open { ask, reply } => self.open(ask, reply),
            Order::Input(peer, input) => self.input(peer, input),
            Order::Release(peer) if self.controller == Some(peer) => self.release_all(),
            Order::Release(_) | Order::Stop(_) => {}
        }
    }

    /// Sends what is waiting to go, and takes in what the portal said meanwhile.
    fn listen(&mut self) -> Result<()> {
        self.bus.channel().flush();
        while self.bus.process(Duration::ZERO)? {}
        while let Ok(heard) = self.heard.try_recv() {
            match heard {
                Heard::Response(response) => self.answered(response),
                Heard::Closed(session) => self.closed(&session),
            }
        }
        Ok(())
    }

    fn open(&mut self, ask: bool, reply: Reply) {
        if !self.shared.supported.load(Relaxed) && !self.check() {
            let _ = reply.send(Err(NO_PORTAL.into()));
            return;
        }
        match &self.state {
            State::Running { .. } => {
                let _ = reply.send(Ok(()));
            }
            // Closed for being unused, which input undoes by itself.
            State::Closed if !ask => {
                let _ = reply.send(Ok(()));
            }
            State::Failed(why) if !ask => {
                let _ = reply.send(Err((*why).into()));
            }
            State::Starting { .. } => self.waiting.push(reply),
            State::Closed | State::Failed(_) => {
                self.waiting.push(reply);
                self.begin();
            }
        }
    }

    fn input(&mut self, peer: PeerId, input: Input) {
        self.controller = Some(peer);
        self.last_input = Instant::now();
        match &self.state {
            State::Running { session } => {
                let session = session.clone();
                self.inject(&session, input);
            }
            State::Starting { .. } => self.hold(input),
            // Input after the desktop closed for idleness opens it again.
            State::Closed if self.shared.supported.load(Relaxed) => {
                self.hold(input);
                self.begin();
            }
            State::Closed | State::Failed(_) => {}
        }
    }

    /// Keeps what matters until the desktop is ready. Motion that old would only jump.
    fn hold(&mut self, input: Input) {
        if !matches!(input, Input::Move { .. } | Input::Scroll { .. }) && self.held_back.len() < HOLD_AT_MOST {
            self.held_back.push_back(input);
        }
    }

    fn token(&mut self) -> String {
        self.tokens += 1;
        format!("orither{}_{}", std::process::id(), self.tokens)
    }

    fn request(&self, method: &str, arguments: impl AppendAll) -> Result<String> {
        let portal = self.bus.with_proxy(DESKTOP, DESKTOP_PATH, CALL_TIMEOUT);
        let (request,): (Path<'static>,) = portal.method_call(REMOTE_DESKTOP, method, arguments)?;
        Ok(request.to_string())
    }

    fn begin(&mut self) {
        let mut options = PropMap::new();
        option(&mut options, "handle_token", self.token());
        option(&mut options, "session_handle_token", self.token());
        match self.request("CreateSession", (options,)) {
            Ok(request) => self.state = State::Starting { step: Step::Create, request, session: None },
            Err(err) => {
                tracing::warn!("creating a remote desktop session: {err:#}");
                self.fail(FAILED);
            }
        }
    }

    fn select(&mut self, session: Path<'static>) -> Result<()> {
        let mut options = PropMap::new();
        option(&mut options, "handle_token", self.token());
        option(&mut options, "types", self.devices);
        if self.version >= 2 {
            option(&mut options, "persist_mode", PERSIST_UNTIL_REVOKED);
            if let Some(token) = preferences::read::<String>(&self.phone.app, TOKEN) {
                option(&mut options, "restore_token", token);
            }
        }
        let request = self.request("SelectDevices", (session.clone(), options))?;
        self.state = State::Starting { step: Step::Select, request, session: Some(session) };
        Ok(())
    }

    /// Without a token that still holds, this is when the desktop asks the user.
    fn start(&mut self, session: Path<'static>) -> Result<()> {
        let mut options = PropMap::new();
        option(&mut options, "handle_token", self.token());
        let request = self.request("Start", (session.clone(), "", options))?;
        self.state = State::Starting { step: Step::Start, request, session: Some(session) };
        Ok(())
    }

    fn answered(&mut self, response: Response) {
        let State::Starting { step, request, session } = &self.state else { return };
        if *request != response.request {
            return;
        }
        let (step, session) = (*step, session.clone());
        let next = match (response.code, step, session.clone()) {
            (ALLOWED, Step::Create, _) => response.session.ok_or_else(|| anyhow!("no session")).and_then(object).and_then(|session| self.select(session)),
            (ALLOWED, Step::Select, Some(session)) => self.start(session),
            (ALLOWED, Step::Start, Some(session)) => {
                self.started(session, response.token);
                Ok(())
            }
            (REFUSED_BY_USER, ..) => Err(anyhow!(REFUSED)),
            (code, ..) => Err(anyhow!("the portal answered {code}")),
        };
        if let Err(err) = next {
            if let Some(session) = &session {
                self.close_session(session);
            }
            if response.code == REFUSED_BY_USER {
                self.fail(REFUSED);
            } else {
                tracing::warn!("starting remote input: {err:#}");
                self.fail(FAILED);
            }
        }
    }

    fn started(&mut self, session: Path<'static>, token: Option<String>) {
        // A token is good for one start, so each start leaves the next one.
        if let Some(token) = token {
            preferences::write(&self.phone.app, TOKEN, &token);
        }
        self.state = State::Running { session: session.clone() };
        self.last_input = Instant::now();
        self.set_active(true);
        self.waiting.drain(..).for_each(|reply| drop(reply.send(Ok(()))));
        for input in std::mem::take(&mut self.held_back) {
            self.inject(&session, input);
        }
    }

    fn fail(&mut self, why: &'static str) {
        self.state = State::Failed(why);
        self.held_back.clear();
        self.pressed = Pressed::default();
        self.waiting.drain(..).for_each(|reply| drop(reply.send(Err(why.into()))));
        self.set_active(false);
    }

    /// The desktop ended a session, usually because the user stopped it there.
    fn closed(&mut self, closed: &str) {
        match &self.state {
            State::Running { session } if &**session == closed => self.fail(STOPPED),
            State::Starting { session: Some(session), .. } if &**session == closed => self.fail(FAILED),
            _ => {}
        }
    }

    fn close(&mut self) {
        if let State::Running { session } = &self.state {
            let session = session.clone();
            self.release_all();
            self.close_session(&session);
        } else if let State::Starting { session: Some(session), .. } = &self.state {
            let session = session.clone();
            self.close_session(&session);
        }
        self.state = State::Closed;
        self.held_back.clear();
        self.waiting.drain(..).for_each(|reply| drop(reply.send(Err(FAILED.into()))));
        self.set_active(false);
    }

    fn close_session(&self, session: &Path<'static>) {
        let session = self.bus.with_proxy(DESKTOP, session.clone(), CLOSE_TIMEOUT);
        if let Err(err) = session.method_call::<(), _, _, _>(SESSION, "Close", ()) {
            tracing::debug!("closing the remote desktop session: {err}");
        }
    }

    fn set_active(&self, active: bool) {
        if self.shared.active.swap(active, Relaxed) != active {
            self.phone.tell_ui("input", "state", self.shared.state());
        }
    }

    /// Sends without waiting for an answer. A session that went away says so with `Closed`.
    fn notify(&self, method: &str, arguments: impl AppendAll) {
        let mut message = Message::new_method_call(DESKTOP, DESKTOP_PATH, REMOTE_DESKTOP, method).expect("valid names");
        message.append_all(arguments);
        message.set_no_reply(true);
        let _ = self.bus.send(message);
    }

    fn inject(&mut self, session: &Path<'static>, input: Input) {
        let session = session.clone();
        match input {
            Input::Move { dx, dy } => self.notify("NotifyPointerMotion", (session, PropMap::new(), dx, dy)),
            Input::Button { button, down } => self.button(session, button_code(button), down),
            Input::Click(button) => {
                self.button(session.clone(), button_code(button), true);
                self.button(session, button_code(button), false);
            }
            Input::Scroll { dx, dy, end } => {
                let mut options = PropMap::new();
                if end {
                    option(&mut options, "finish", true);
                }
                self.notify("NotifyPointerAxis", (session, options, axis(dx), axis(dy)));
            }
            Input::Text(text) => {
                for keysym in text.chars().filter_map(keysym) {
                    let keysym = keysym as i32;
                    self.notify("NotifyKeyboardKeysym", (session.clone(), PropMap::new(), keysym, PRESSED));
                    self.notify("NotifyKeyboardKeysym", (session.clone(), PropMap::new(), keysym, RELEASED));
                }
            }
            Input::Key { key, down } => {
                let code = keycode(key);
                if down != Some(false) {
                    self.key(session.clone(), code, true);
                }
                if down != Some(true) {
                    self.key(session, code, false);
                }
            }
        }
    }

    fn button(&mut self, session: Path<'static>, code: i32, down: bool) {
        if down {
            self.pressed.buttons.insert(code);
        } else {
            self.pressed.buttons.remove(&code);
        }
        self.notify("NotifyPointerButton", (session, PropMap::new(), code, if down { PRESSED } else { RELEASED }));
    }

    fn key(&mut self, session: Path<'static>, code: i32, down: bool) {
        if down {
            self.pressed.keys.insert(code);
        } else {
            self.pressed.keys.remove(&code);
        }
        self.notify("NotifyKeyboardKeycode", (session, PropMap::new(), code, if down { PRESSED } else { RELEASED }));
    }

    fn release_all(&mut self) {
        let State::Running { session } = &self.state else { return };
        let session = session.clone();
        let Pressed { keys, buttons } = std::mem::take(&mut self.pressed);
        keys.into_iter().for_each(|code| self.key(session.clone(), code, false));
        buttons.into_iter().for_each(|code| self.button(session.clone(), code, false));
    }
}

fn axis(pixels: f64) -> f64 {
    pixels * AXIS_PER_PIXEL
}

/// Linux evdev button codes, which the portal takes.
fn button_code(button: Button) -> i32 {
    match button {
        Button::Left => LEFT,
        Button::Right => RIGHT,
        Button::Middle => MIDDLE,
    }
}

/// Linux evdev key codes, from `input-event-codes.h`. Named keys go by code rather
/// than by symbol, so they are the same key on every layout.
fn keycode(key: Key) -> i32 {
    match key {
        Key::Escape => 1,
        Key::Backspace => 14,
        Key::Tab => 15,
        Key::Enter => 28,
        Key::Control => 29,
        Key::Shift => 42,
        Key::Alt => 56,
        Key::Space => 57,
        Key::Function(number @ 1..=10) => 58 + i32::from(number),
        Key::Function(11) => 87,
        Key::Function(_) => 88,
        Key::Home => 102,
        Key::Up => 103,
        Key::PageUp => 104,
        Key::Left => 105,
        Key::Right => 106,
        Key::End => 107,
        Key::Down => 108,
        Key::PageDown => 109,
        Key::Delete => 111,
        Key::Meta => 125,
    }
}

/// The XKB keysym that types `character`, by xkbcommon's own rules: Latin-1 as it
/// is, the older keysyms where a script has them, and Unicode keysyms otherwise. The
/// desktop looks the keysym up in the keyboard layout, whose keys carry the older
/// ones, so `€` has to be `EuroSign` and not its Unicode keysym.
fn keysym(character: char) -> Option<u32> {
    match character {
        '\n' => Some(RETURN),
        '\t' => Some(TAB),
        _ if character.is_control() => None,
        _ => Some(Keysym::from_char(character)).filter(|keysym| *keysym != Keysym::NoSymbol).map(Keysym::raw),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_characters_as_the_layout_has_them() {
        assert_eq!(keysym('a'), Some(0x61));
        assert_eq!(keysym('A'), Some(0x41));
        assert_eq!(keysym(' '), Some(0x20));
        assert_eq!(keysym('é'), Some(0xe9));
        assert_eq!(keysym('€'), Some(0x20ac));
        assert_eq!(keysym('ж'), Some(0x6d6));
        assert_eq!(keysym('😀'), Some(0x0101_f600));
        assert_eq!(keysym('\n'), Some(RETURN));
        assert_eq!(keysym('\t'), Some(TAB));
        assert_eq!(keysym('\r'), None);
        assert_eq!(keysym('\u{7}'), None);
    }

    #[test]
    fn named_keys_are_evdev_codes() {
        assert_eq!(keycode(Key::Enter), 28);
        assert_eq!(keycode(Key::Function(1)), 59);
        assert_eq!(keycode(Key::Function(10)), 68);
        assert_eq!(keycode(Key::Function(11)), 87);
        assert_eq!(keycode(Key::Function(12)), 88);
        assert_eq!(keycode(Key::Left), 105);
        assert_eq!(keycode(Key::Meta), 125);
        let keys = [
            Key::Enter, Key::Backspace, Key::Delete, Key::Tab, Key::Escape, Key::Space, Key::Left, Key::Right, Key::Up,
            Key::Down, Key::Home, Key::End, Key::PageUp, Key::PageDown, Key::Control, Key::Alt, Key::Shift, Key::Meta,
        ];
        let codes: HashSet<i32> = keys.into_iter().chain((1..=12).map(Key::Function)).map(keycode).collect();
        assert_eq!(codes.len(), keys.len() + 12);
    }

    #[test]
    fn a_finger_scrolls_a_notch_every_fifty_pixels() {
        assert_eq!(axis(50.0), 15.0);
        assert_eq!(axis(-100.0), -30.0);
    }

    #[test]
    fn buttons_are_evdev_codes() {
        assert_eq!([Button::Left, Button::Right, Button::Middle].map(button_code), [0x110, 0x111, 0x112]);
    }
}
