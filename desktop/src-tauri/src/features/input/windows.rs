//! Input goes in with `SendInput`, as if from a mouse and keyboard plugged into this
//! computer. Windows asks nobody about that, so `open` only puts the phone in
//! control, and control ends after 10 minutes without input, as on Linux, to start
//! again with the next input.
//!
//! Windows keeps injected input away from programs that run with more rights than
//! Orither (User Interface Privilege Isolation): a window run as administrator, and
//! the UAC prompt or the sign-in screen, ignore it. `SendInput` cannot tell when that
//! happens, so nothing says so.
//!
//! Orders reach a thread of their own, which keeps them in order, ends control when
//! it has been idle, and lets go of whatever a phone held down.

use std::{
    collections::HashSet,
    mem::size_of,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering::Relaxed},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    time::{Duration, Instant},
};

use orither_link::PeerId;
use serde_json::{Value, json};
use windows_sys::Win32::UI::{
    Input::KeyboardAndMouse::{
        GetKeyboardLayout, HKL, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP,
        KEYEVENTF_UNICODE, MAPVK_VK_TO_VSC, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN,
        MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL, MOUSEINPUT,
        MapVirtualKeyW, SendInput, VkKeyScanExW,
    },
    WindowsAndMessaging::{GetForegroundWindow, GetSystemMetrics, GetWindowThreadProcessId, SM_SWAPBUTTON},
};

use super::{
    Button, Input,
    windows_keys::{self as keys, Carry, Typed},
};
use crate::features::Phone;

/// How long the computer stays controlled without any input.
const IDLE_FOR: Duration = Duration::from_secs(10 * 60);
/// How often the thread looks for idleness while a phone is in control.
const WHILE_ACTIVE: Duration = Duration::from_secs(1);
/// Stopping happens as Orither quits, which should not wait long.
const STOP_WITHIN: Duration = Duration::from_secs(1);
const NO_THREAD: &str = "Your computer could not start remote control.";

#[derive(Default)]
struct Shared {
    active: AtomicBool,
}

impl Shared {
    /// `asks` tells the UI that nobody is asked here, unlike on Linux.
    fn state(&self) -> Value {
        json!({ "supported": true, "active": self.active.load(Relaxed), "asks": false })
    }
}

enum Order {
    Open { ask: bool },
    Input(PeerId, Input),
    Release(PeerId),
    Stop(Sender<()>),
}

/// The thread that sends input to Windows.
pub struct Remote {
    orders: Sender<Order>,
    shared: Arc<Shared>,
}

impl Remote {
    pub fn start(phone: Phone) -> Self {
        let (orders, taken) = mpsc::channel();
        let shared = Arc::<Shared>::default();
        let theirs = shared.clone();
        let thread = std::thread::Builder::new().name("remote input".into());
        if let Err(err) = thread.spawn(move || Desktop::new(phone, theirs).run(&taken)) {
            tracing::warn!("no remote input: {err}");
        }
        Self { orders, shared }
    }

    pub fn state(&self) -> Value {
        self.shared.state()
    }

    /// A phone that asks gets control at once. One that only checks hears that it
    /// can have it, and gets it with its first input.
    pub async fn open(&self, ask: bool) -> Result<(), String> {
        self.orders.send(Order::Open { ask }).map_err(|_| NO_THREAD.to_owned())
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
            let _ = stopped.recv_timeout(STOP_WITHIN);
        }
    }
}

struct Desktop {
    phone: Phone,
    shared: Arc<Shared>,
    /// The phone that sent input last.
    controller: Option<PeerId>,
    last_input: Instant,
    /// Virtual keys held down, to let go of once nobody is there to.
    keys: HashSet<u16>,
    /// The flags that let go of each button held down.
    buttons: HashSet<u32>,
    motion: (Carry, Carry),
    wheel: (Carry, Carry),
    /// Whether Windows refused the last input, so that a locked screen is logged
    /// once rather than for every motion.
    refused: bool,
}

impl Desktop {
    fn new(phone: Phone, shared: Arc<Shared>) -> Self {
        Self {
            phone,
            shared,
            controller: None,
            last_input: Instant::now(),
            keys: HashSet::new(),
            buttons: HashSet::new(),
            motion: Default::default(),
            wheel: Default::default(),
            refused: false,
        }
    }

    fn active(&self) -> bool {
        self.shared.active.load(Relaxed)
    }

    fn run(mut self, orders: &Receiver<Order>) {
        loop {
            let order = if self.active() {
                orders.recv_timeout(WHILE_ACTIVE)
            } else {
                orders.recv().map_err(|_| RecvTimeoutError::Disconnected)
            };
            match order {
                Ok(Order::Stop(done)) => {
                    self.close();
                    let _ = done.send(());
                    return;
                }
                Ok(order) => self.follow(order),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    self.close();
                    return;
                }
            }
            if self.active() && self.last_input.elapsed() > IDLE_FOR {
                self.close();
            }
        }
    }

    fn follow(&mut self, order: Order) {
        match order {
            Order::Open { ask: true } => {
                self.last_input = Instant::now();
                self.set_active(true);
            }
            Order::Input(peer, input) => {
                self.controller = Some(peer);
                self.last_input = Instant::now();
                self.set_active(true);
                self.inject(input);
            }
            Order::Release(peer) if self.controller == Some(peer) => self.release_all(),
            Order::Open { ask: false } | Order::Release(_) | Order::Stop(_) => {}
        }
    }

    /// Ends control, as Linux closes its session.
    fn close(&mut self) {
        self.release_all();
        self.motion = Default::default();
        self.wheel = Default::default();
        self.set_active(false);
    }

    fn set_active(&self, active: bool) {
        if self.shared.active.swap(active, Relaxed) != active {
            self.phone.tell_ui("input", "state", self.shared.state());
        }
    }

    fn inject(&mut self, input: Input) {
        match input {
            // Relative motion, as a mouse's, so it also reaches programs that read the
            // mouse itself, such as games. Windows applies its pointer speed to it, and
            // "Enhance pointer precision" too, on top of the phone's own acceleration.
            Input::Move { dx, dy } => {
                let (dx, dy) = (self.motion.0.take(dx), self.motion.1.take(dy));
                if dx != 0 || dy != 0 {
                    self.send(&[mouse(dx, dy, 0, MOUSEEVENTF_MOVE)]);
                }
            }
            Input::Button { button, down } => self.button(button, down),
            Input::Click(button) => {
                self.button(button, true);
                self.button(button, false);
            }
            Input::Scroll { dx, dy, end } => {
                let horizontal = self.wheel.0.take(keys::wheel_units(dx));
                let vertical = self.wheel.1.take(keys::wheel_units(dy));
                let mut events = Vec::new();
                // The wheel's numbers are signed, in an unsigned field. Windows
                // scrolls up for a positive wheel, where the phone scrolls down.
                if vertical != 0 {
                    events.push(mouse(0, 0, vertical.saturating_neg() as u32, MOUSEEVENTF_WHEEL));
                }
                if horizontal != 0 {
                    events.push(mouse(0, 0, horizontal as u32, MOUSEEVENTF_HWHEEL));
                }
                self.send(&events);
                // What is left of a notch would only nudge the next scroll.
                if end {
                    self.wheel = Default::default();
                }
            }
            Input::Text(text) => {
                let events = self.text(&text);
                self.send(&events);
            }
            Input::Key { key, down } => {
                let vk = keys::virtual_key(key);
                if down != Some(false) {
                    self.key(vk, true);
                }
                if down != Some(true) {
                    self.key(vk, false);
                }
            }
        }
    }

    /// Text goes as Unicode, which types any character on any layout and past any
    /// input method. While Control, Alt or Windows is held it is a shortcut, which
    /// applications look for as a key, so then each character is the key that types
    /// it on the layout of the window in front, wherever that layout has one.
    fn text(&self, text: &str) -> Vec<INPUT> {
        let shortcut = self.keys.iter().any(|vk| keys::makes_a_shortcut(*vk));
        let layout = shortcut.then(foreground_layout);
        let shift_held = self.keys.contains(&keys::VK_SHIFT);
        let mut events = Vec::new();
        for character in text.chars() {
            match keys::typed(character) {
                Some(Typed::Key(vk)) => events.extend([keyboard(vk, false), keyboard(vk, true)]),
                Some(Typed::Char(character)) => match layout.and_then(|layout| on_layout(character, layout)) {
                    Some((vk, shift)) => {
                        let shift = shift && !shift_held;
                        if shift {
                            events.push(keyboard(keys::VK_SHIFT, false));
                        }
                        events.extend([keyboard(vk, false), keyboard(vk, true)]);
                        if shift {
                            events.push(keyboard(keys::VK_SHIFT, true));
                        }
                    }
                    None => events.extend(keys::unicode_events(character).into_iter().map(|(unit, up)| unicode(unit, up))),
                },
                None => {}
            }
        }
        events
    }

    fn button(&mut self, button: Button, down: bool) {
        // SAFETY: GetSystemMetrics only reads a setting.
        let swapped = unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0;
        let (press, release) = match keys::physical(button, swapped) {
            Button::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
            Button::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
            Button::Middle => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
        };
        if down {
            self.buttons.insert(release);
        } else {
            self.buttons.remove(&release);
        }
        self.send(&[mouse(0, 0, 0, if down { press } else { release })]);
    }

    fn key(&mut self, vk: u16, down: bool) {
        if down {
            self.keys.insert(vk);
        } else {
            self.keys.remove(&vk);
        }
        self.send(&[keyboard(vk, !down)]);
    }

    fn release_all(&mut self) {
        let keys = std::mem::take(&mut self.keys).into_iter().map(|vk| keyboard(vk, true));
        let buttons = std::mem::take(&mut self.buttons).into_iter().map(|release| mouse(0, 0, 0, release));
        let events: Vec<INPUT> = keys.chain(buttons).collect();
        self.send(&events);
    }

    /// Sends events in one go, so nothing from a real mouse or keyboard lands among them.
    fn send(&mut self, events: &[INPUT]) {
        let Ok(count) = u32::try_from(events.len()) else { return };
        if count == 0 {
            return;
        }
        // SAFETY: the events are initialized, and `count` and the size describe them.
        let sent = unsafe { SendInput(count, events.as_ptr(), size_of::<INPUT>() as i32) };
        if sent == count {
            self.refused = false;
        } else if !std::mem::replace(&mut self.refused, true) {
            // Usually the sign-in screen or a UAC prompt, which no program may type into.
            tracing::warn!("Windows took {sent} of {count} input events: {}", std::io::Error::last_os_error());
        }
    }
}

fn mouse(dx: i32, dy: i32, data: u32, flags: u32) -> INPUT {
    let mi = MOUSEINPUT { dx, dy, mouseData: data, dwFlags: flags, time: 0, dwExtraInfo: 0 };
    INPUT { r#type: INPUT_MOUSE, Anonymous: INPUT_0 { mi } }
}

/// A key by its virtual-key code, with the scan code an application may read as well,
/// as the browser does for a key's `code`.
fn keyboard(vk: u16, up: bool) -> INPUT {
    // SAFETY: MapVirtualKeyW only reads the keyboard layout.
    let scan = unsafe { MapVirtualKeyW(u32::from(vk), MAPVK_VK_TO_VSC) };
    let mut flags = if up { KEYEVENTF_KEYUP } else { 0 };
    if keys::extended(vk) {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    let ki = KEYBDINPUT { wVk: vk, wScan: scan as u16, dwFlags: flags, time: 0, dwExtraInfo: 0 };
    INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki } }
}

/// One UTF-16 unit of text, which reaches the window as a character of its own.
fn unicode(unit: u16, up: bool) -> INPUT {
    let flags = KEYEVENTF_UNICODE | if up { KEYEVENTF_KEYUP } else { 0 };
    let ki = KEYBDINPUT { wVk: 0, wScan: unit, dwFlags: flags, time: 0, dwExtraInfo: 0 };
    INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki } }
}

/// The keyboard layout of the window that gets the keys, which may not be this thread's.
fn foreground_layout() -> HKL {
    // SAFETY: these only read; a missing window gives thread 0, which is this thread's layout.
    unsafe { GetKeyboardLayout(GetWindowThreadProcessId(GetForegroundWindow(), std::ptr::null_mut())) }
}

/// The key that types `character` on `layout`, and whether it takes Shift.
fn on_layout(character: char, layout: HKL) -> Option<(u16, bool)> {
    let unit = u16::try_from(u32::from(character)).ok()?;
    // SAFETY: VkKeyScanExW only reads the layout.
    keys::layout_key(unsafe { VkKeyScanExW(unit, layout) })
}

#[cfg(test)]
mod tests {
    use windows_sys::Win32::UI::{Input::KeyboardAndMouse as windows, WindowsAndMessaging::WHEEL_DELTA};

    use super::keys;

    /// The codes in `windows_keys` are written out so that they are tested on Linux too.
    #[test]
    fn the_codes_written_out_are_windows_own() {
        let pairs = [
            (keys::VK_BACK, windows::VK_BACK),
            (keys::VK_TAB, windows::VK_TAB),
            (keys::VK_RETURN, windows::VK_RETURN),
            (keys::VK_SHIFT, windows::VK_SHIFT),
            (keys::VK_CONTROL, windows::VK_CONTROL),
            (keys::VK_MENU, windows::VK_MENU),
            (keys::VK_ESCAPE, windows::VK_ESCAPE),
            (keys::VK_SPACE, windows::VK_SPACE),
            (keys::VK_PRIOR, windows::VK_PRIOR),
            (keys::VK_NEXT, windows::VK_NEXT),
            (keys::VK_END, windows::VK_END),
            (keys::VK_HOME, windows::VK_HOME),
            (keys::VK_LEFT, windows::VK_LEFT),
            (keys::VK_UP, windows::VK_UP),
            (keys::VK_RIGHT, windows::VK_RIGHT),
            (keys::VK_DOWN, windows::VK_DOWN),
            (keys::VK_SNAPSHOT, windows::VK_SNAPSHOT),
            (keys::VK_INSERT, windows::VK_INSERT),
            (keys::VK_DELETE, windows::VK_DELETE),
            (keys::VK_LWIN, windows::VK_LWIN),
            (keys::VK_RWIN, windows::VK_RWIN),
            (keys::VK_APPS, windows::VK_APPS),
            (keys::VK_DIVIDE, windows::VK_DIVIDE),
            (keys::VK_F1, windows::VK_F1),
            (keys::VK_NUMLOCK, windows::VK_NUMLOCK),
            (keys::VK_RCONTROL, windows::VK_RCONTROL),
            (keys::VK_RMENU, windows::VK_RMENU),
        ];
        assert!(pairs.iter().all(|(ours, theirs)| ours == theirs));
        assert_eq!(keys::WHEEL_DELTA, f64::from(WHEEL_DELTA));
    }
}
