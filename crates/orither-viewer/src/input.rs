//! Turns what the mouse and keyboard do over the window into control messages.

use winit::{
    event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta},
    keyboard::{KeyCode, ModifiersState, PhysicalKey},
};

use crate::{
    control::{self, Position, Press, Touch},
    render::Viewport,
};

const KEYCODE_HOME: u32 = 3;
const KEYCODE_0: u32 = 7;
const KEYCODE_A: u32 = 29;
const META_SHIFT: u32 = 0x1;
const META_ALT: u32 = 0x2;
const META_CTRL: u32 = 0x1000;
/// How far one scroll wheel notch moves.
const PIXELS_PER_LINE: f64 = 40.0;

/// Android's name for a key that produces no text.
fn android_key(key: KeyCode) -> Option<u32> {
    Some(match key {
        KeyCode::Enter | KeyCode::NumpadEnter => 66,
        KeyCode::Backspace => 67,
        KeyCode::Tab => 61,
        KeyCode::Escape => 111,
        KeyCode::Delete => 112,
        KeyCode::Home => 122,
        KeyCode::End => 123,
        KeyCode::PageUp => 92,
        KeyCode::PageDown => 93,
        KeyCode::ArrowUp => 19,
        KeyCode::ArrowDown => 20,
        KeyCode::ArrowLeft => 21,
        KeyCode::ArrowRight => 22,
        _ => return None,
    })
}

/// Android's name for a letter or digit, for shortcuts such as Ctrl+C.
fn android_character(key: KeyCode) -> Option<u32> {
    const LETTERS: [KeyCode; 26] = [
        KeyCode::KeyA, KeyCode::KeyB, KeyCode::KeyC, KeyCode::KeyD, KeyCode::KeyE, KeyCode::KeyF, KeyCode::KeyG,
        KeyCode::KeyH, KeyCode::KeyI, KeyCode::KeyJ, KeyCode::KeyK, KeyCode::KeyL, KeyCode::KeyM, KeyCode::KeyN,
        KeyCode::KeyO, KeyCode::KeyP, KeyCode::KeyQ, KeyCode::KeyR, KeyCode::KeyS, KeyCode::KeyT, KeyCode::KeyU,
        KeyCode::KeyV, KeyCode::KeyW, KeyCode::KeyX, KeyCode::KeyY, KeyCode::KeyZ,
    ];
    const DIGITS: [KeyCode; 10] = [
        KeyCode::Digit0, KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4, KeyCode::Digit5,
        KeyCode::Digit6, KeyCode::Digit7, KeyCode::Digit8, KeyCode::Digit9,
    ];
    let index = |keys: &[KeyCode]| keys.iter().position(|&candidate| candidate == key).map(|index| index as u32);
    index(&LETTERS).map(|index| KEYCODE_A + index).or_else(|| index(&DIGITS).map(|index| KEYCODE_0 + index))
}

#[derive(Default)]
pub struct Input {
    cursor: Option<(f64, f64)>,
    held: bool,
    modifiers: ModifiersState,
}

impl Input {
    pub fn modifiers(&mut self, modifiers: ModifiersState) {
        self.modifiers = modifiers;
    }

    fn meta(&self) -> u32 {
        let flag = |on: bool, meta: u32| if on { meta } else { 0 };
        flag(self.modifiers.shift_key(), META_SHIFT)
            | flag(self.modifiers.alt_key(), META_ALT)
            | flag(self.modifiers.control_key(), META_CTRL)
    }

    fn position(&self, viewport: Viewport) -> Option<Position> {
        let (x, y) = self.cursor?;
        viewport.locate(x, y)
    }

    pub fn moved(&mut self, x: f64, y: f64, viewport: Viewport) -> Vec<Vec<u8>> {
        self.cursor = Some((x, y));
        match self.position(viewport) {
            Some(position) if self.held => vec![control::touch(Touch::Move, position)],
            _ => Vec::new(),
        }
    }

    /// Left is a finger, right is back and the middle button is home.
    pub fn button(&mut self, button: MouseButton, state: ElementState, viewport: Viewport) -> Vec<Vec<u8>> {
        let press = if state.is_pressed() { Press::Down } else { Press::Up };
        match button {
            MouseButton::Left => {
                let Some(position) = self.position(viewport) else { return Vec::new() };
                self.held = state.is_pressed();
                let touch = if self.held { Touch::Down } else { Touch::Up };
                vec![control::touch(touch, position)]
            }
            MouseButton::Right => vec![control::back(press)],
            MouseButton::Middle => vec![control::key(press, KEYCODE_HOME, 0)],
            _ => Vec::new(),
        }
    }

    pub fn wheel(&self, delta: MouseScrollDelta, viewport: Viewport) -> Vec<Vec<u8>> {
        let (horizontal, vertical) = match delta {
            MouseScrollDelta::LineDelta(x, y) => (f64::from(x), f64::from(y)),
            MouseScrollDelta::PixelDelta(pixels) => (pixels.x / PIXELS_PER_LINE, pixels.y / PIXELS_PER_LINE),
        };
        self.position(viewport)
            .map(|position| control::scroll(position, horizontal as f32, vertical as f32))
            .into_iter()
            .collect()
    }

    pub fn key(&self, event: &KeyEvent) -> Vec<Vec<u8>> {
        let PhysicalKey::Code(code) = event.physical_key else { return Vec::new() };
        let press = if event.state.is_pressed() { Press::Down } else { Press::Up };
        let shortcut = self.modifiers.control_key() || self.modifiers.alt_key();

        if let Some(keycode) = android_key(code).or(android_character(code).filter(|_| shortcut)) {
            return vec![control::key(press, keycode, self.meta())];
        }
        match event.text.as_deref() {
            Some(text) if event.state.is_pressed() && !shortcut => vec![control::text(text)],
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_and_digits_have_android_names() {
        assert_eq!(android_character(KeyCode::KeyA), Some(29));
        assert_eq!(android_character(KeyCode::KeyV), Some(50));
        assert_eq!(android_character(KeyCode::Digit9), Some(16));
        assert_eq!(android_character(KeyCode::Space), None);
    }
}
