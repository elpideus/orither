//! Lets a phone be this computer's trackpad and keyboard. The phone sends what its
//! fingers and keyboard do as notices, and [`platform`] turns them into the
//! computer's own input. A Linux desktop asks the user before anything controls it,
//! so the phone first calls `open`, which says what the user still has to do.

#[cfg(target_os = "linux")]
#[path = "input/linux.rs"]
mod platform;

#[cfg(windows)]
#[path = "input/windows.rs"]
mod platform;

#[cfg(not(any(target_os = "linux", windows)))]
#[path = "input/unsupported.rs"]
mod platform;

/// Windows' names for keys, apart from the Windows module so that they are tested everywhere.
#[cfg(any(windows, test))]
#[path = "input/windows_keys.rs"]
mod windows_keys;

use std::sync::Arc;

use anyhow::{Result, anyhow};
use orither_link::PeerId;
use serde_json::{Value, json};

use super::{Feature, Phone};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Button {
    Left,
    Right,
    Middle,
}

impl Button {
    fn named(name: &str) -> Option<Self> {
        Some(match name {
            "left" => Self::Left,
            "right" => Self::Right,
            "middle" => Self::Middle,
            _ => return None,
        })
    }
}

/// The keys a phone keyboard has no character for, named as the web names them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Key {
    Enter,
    Backspace,
    Delete,
    Tab,
    Escape,
    Space,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    /// F1 to F12.
    Function(u8),
    Control,
    Alt,
    Shift,
    Meta,
}

impl Key {
    fn named(name: &str) -> Option<Self> {
        Some(match name {
            "Enter" => Self::Enter,
            "Backspace" => Self::Backspace,
            "Delete" => Self::Delete,
            "Tab" => Self::Tab,
            "Escape" => Self::Escape,
            "Space" | " " => Self::Space,
            "ArrowLeft" => Self::Left,
            "ArrowRight" => Self::Right,
            "ArrowUp" => Self::Up,
            "ArrowDown" => Self::Down,
            "Home" => Self::Home,
            "End" => Self::End,
            "PageUp" => Self::PageUp,
            "PageDown" => Self::PageDown,
            "Control" => Self::Control,
            "Alt" => Self::Alt,
            "Shift" => Self::Shift,
            "Meta" => Self::Meta,
            _ => {
                let number = name.strip_prefix('F')?.parse().ok()?;
                return (1..=12).contains(&number).then_some(Self::Function(number));
            }
        })
    }
}

/// One thing a phone did, in the order it did it.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    /// Pointer motion in logical pixels, already accelerated by the phone.
    Move { dx: f64, dy: f64 },
    Button { button: Button, down: bool },
    Click(Button),
    /// Smooth scrolling in pixels, positive down and right. `end` comes as the fingers lift.
    Scroll { dx: f64, dy: f64, end: bool },
    Text(String),
    /// Pressed and released when `down` is `None`, held or let go otherwise.
    Key { key: Key, down: Option<bool> },
}

impl Input {
    /// Reads a notice, or `None` when it is not one of input's.
    fn read(method: &str, body: &Value) -> Option<Self> {
        let number = |name: &str| body[name].as_f64().filter(|value| value.is_finite()).unwrap_or(0.0);
        let button = || Button::named(body["button"].as_str()?);
        Some(match method {
            "move" => Self::Move { dx: number("dx"), dy: number("dy") },
            "button" => Self::Button { button: button()?, down: body["down"].as_bool()? },
            "click" => Self::Click(button()?),
            "scroll" => Self::Scroll { dx: number("dx"), dy: number("dy"), end: body["end"].as_bool().unwrap_or(false) },
            "text" => Self::Text(body["text"].as_str()?.to_owned()),
            "key" => Self::Key { key: Key::named(body["key"].as_str()?)?, down: body["down"].as_bool() },
            _ => return None,
        })
    }
}

const METHODS: [&str; 6] = ["move", "button", "click", "scroll", "text", "key"];

pub struct RemoteInput {
    remote: platform::Remote,
}

impl RemoteInput {
    pub fn start(phone: Phone) -> Arc<dyn Feature> {
        Arc::new(Self { remote: platform::Remote::start(phone) })
    }
}

#[async_trait::async_trait]
impl Feature for RemoteInput {
    fn id(&self) -> &'static str {
        "input"
    }

    async fn command(&self, method: &str, _body: &Value) -> Option<Result<Value>> {
        match method {
            "state" => Some(Ok(self.remote.state())),
            _ => None,
        }
    }

    /// `open` gets the desktop ready, asking the user if it has to. With `ask` false
    /// it only reports, so a phone can check now and then without asking again
    /// someone who just said no.
    async fn answer(&self, _peer: &PeerId, method: &str, body: &Value) -> Option<Result<Value>> {
        if method != "open" {
            return None;
        }
        let ask = body["ask"].as_bool().unwrap_or(true);
        Some(self.remote.open(ask).await.map(|()| json!({ "ready": true })).map_err(|error| anyhow!(error)))
    }

    fn notice(&self, peer: &PeerId, method: &str, body: &Value) -> bool {
        match Input::read(method, body) {
            Some(input) => self.remote.send(*peer, input),
            None if METHODS.contains(&method) => tracing::debug!("odd input {method}: {body}"),
            None => return false,
        }
        true
    }

    /// Nothing stays held for a phone that went away mid-drag or mid-chord.
    fn disconnected(&self, peer: &PeerId) {
        self.remote.release(peer);
    }

    fn stopping(&self) {
        self.remote.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_every_notice() {
        assert_eq!(Input::read("move", &json!({ "dx": 1.5, "dy": -2 })), Some(Input::Move { dx: 1.5, dy: -2.0 }));
        let held = Input::Button { button: Button::Left, down: true };
        assert_eq!(Input::read("button", &json!({ "button": "left", "down": true })), Some(held));
        assert_eq!(Input::read("click", &json!({ "button": "middle" })), Some(Input::Click(Button::Middle)));
        let scroll = Input::Scroll { dx: 0.0, dy: 12.0, end: false };
        assert_eq!(Input::read("scroll", &json!({ "dy": 12 })), Some(scroll));
        assert_eq!(Input::read("text", &json!({ "text": "hé" })), Some(Input::Text("hé".into())));
        let chord = Input::Key { key: Key::Control, down: Some(true) };
        assert_eq!(Input::read("key", &json!({ "key": "Control", "down": true })), Some(chord));
        assert_eq!(Input::read("key", &json!({ "key": "F12" })), Some(Input::Key { key: Key::Function(12), down: None }));
    }

    #[test]
    fn rejects_what_it_cannot_do() {
        assert_eq!(Input::read("click", &json!({ "button": "fourth" })), None);
        assert_eq!(Input::read("button", &json!({ "button": "left" })), None);
        assert_eq!(Input::read("key", &json!({ "key": "F13" })), None);
        assert_eq!(Input::read("key", &json!({ "key": "Hyper" })), None);
        assert_eq!(Input::read("wave", &json!({})), None);
    }

    #[test]
    fn knows_every_named_key() {
        let names = [
            "Enter", "Backspace", "Delete", "Tab", "Escape", "Space", "ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown",
            "Home", "End", "PageUp", "PageDown", "Control", "Alt", "Shift", "Meta",
        ];
        assert!(names.iter().all(|name| Key::named(name).is_some()));
        assert!((1..=12).all(|number| Key::named(&format!("F{number}")) == Some(Key::Function(number))));
        assert_eq!(Key::named("F0"), None);
    }

    #[test]
    fn a_motion_that_is_not_a_number_does_not_move() {
        assert_eq!(Input::read("move", &json!({ "dx": "far" })), Some(Input::Move { dx: 0.0, dy: 0.0 }));
    }
}
