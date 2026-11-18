//! The messages scrcpy's server takes on its control socket.

const KEYCODE: u8 = 0;
const TEXT: u8 = 1;
const TOUCH: u8 = 2;
const SCROLL: u8 = 3;
const BACK: u8 = 4;
const DISPLAY_POWER: u8 = 10;
const START_APP: u8 = 16;
const RESET_VIDEO: u8 = 17;
const RESIZE_DISPLAY: u8 = 21;
/// Orither's own, added to the scrcpy server it carries (android/scrcpy).
const SET_VIDEO_BIT_RATE: u8 = 100;

const MOUSE_POINTER: i64 = -1;
const PRIMARY_BUTTON: u32 = 1;
const FULL_PRESSURE: u16 = u16::MAX;
const MAX_TEXT: usize = 300;
/// Scroll amounts travel as a fraction of this.
const SCROLL_RANGE: f32 = 16.0;

/// Where on a video of the given size something happened. The server ignores
/// positions meant for a size other than the one it is sending.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Position {
    pub x: i32,
    pub y: i32,
    pub width: u16,
    pub height: u16,
}

impl Position {
    fn write(self, message: &mut Vec<u8>) {
        message.extend_from_slice(&self.x.to_be_bytes());
        message.extend_from_slice(&self.y.to_be_bytes());
        message.extend_from_slice(&self.width.to_be_bytes());
        message.extend_from_slice(&self.height.to_be_bytes());
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    Down = 0,
    Up = 1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Touch {
    Down = 0,
    Up = 1,
    Move = 2,
}

pub fn key(press: Press, keycode: u32, meta: u32) -> Vec<u8> {
    let mut message = vec![KEYCODE, press as u8];
    message.extend_from_slice(&keycode.to_be_bytes());
    message.extend_from_slice(&0u32.to_be_bytes());
    message.extend_from_slice(&meta.to_be_bytes());
    message
}

/// Types text, of which only the first few hundred bytes fit in one message.
pub fn text(text: &str) -> Vec<u8> {
    let end = (0..=MAX_TEXT.min(text.len())).rev().find(|&end| text.is_char_boundary(end)).unwrap_or(0);
    let mut message = vec![TEXT];
    message.extend_from_slice(&(end as u32).to_be_bytes());
    message.extend_from_slice(&text.as_bytes()[..end]);
    message
}

pub fn touch(touch: Touch, position: Position) -> Vec<u8> {
    let held = touch != Touch::Up;
    let mut message = vec![TOUCH, touch as u8];
    message.extend_from_slice(&MOUSE_POINTER.to_be_bytes());
    position.write(&mut message);
    message.extend_from_slice(&(if held { FULL_PRESSURE } else { 0 }).to_be_bytes());
    message.extend_from_slice(&PRIMARY_BUTTON.to_be_bytes());
    message.extend_from_slice(&(if held { PRIMARY_BUTTON } else { 0 }).to_be_bytes());
    message
}

fn scroll_amount(amount: f32) -> i16 {
    ((amount / SCROLL_RANGE).clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16
}

pub fn scroll(position: Position, horizontal: f32, vertical: f32) -> Vec<u8> {
    let mut message = vec![SCROLL];
    position.write(&mut message);
    message.extend_from_slice(&scroll_amount(horizontal).to_be_bytes());
    message.extend_from_slice(&scroll_amount(vertical).to_be_bytes());
    message.extend_from_slice(&0u32.to_be_bytes());
    message
}

pub fn back(press: Press) -> Vec<u8> {
    vec![BACK, press as u8]
}

/// Turns the phone's own display on or off, without stopping what runs on it.
pub fn display_power(on: bool) -> Vec<u8> {
    vec![DISPLAY_POWER, u8::from(on)]
}

/// Sets the bit rate of the phone's video while it plays, in bits a second.
pub fn set_video_bit_rate(bit_rate: u32) -> Vec<u8> {
    let mut message = vec![SET_VIDEO_BIT_RATE];
    message.extend_from_slice(&bit_rate.to_be_bytes());
    message
}

/// Asks the phone to start its video afresh, which begins with a key frame.
pub fn reset_video() -> Vec<u8> {
    vec![RESET_VIDEO]
}

pub fn start_app(package: &str) -> Vec<u8> {
    let name = package.as_bytes();
    let length = name.len().min(usize::from(u8::MAX));
    let mut message = vec![START_APP, length as u8];
    message.extend_from_slice(&name[..length]);
    message
}

pub fn resize_display(width: u16, height: u16) -> Vec<u8> {
    let mut message = vec![RESIZE_DISPLAY];
    message.extend_from_slice(&width.to_be_bytes());
    message.extend_from_slice(&height.to_be_bytes());
    message
}

/// Whether more bytes could still make `bytes` a whole resize message.
pub fn partial_resize(bytes: &[u8]) -> bool {
    bytes.len() < resize_display(0, 0).len() && bytes.first().is_none_or(|&kind| kind == RESIZE_DISPLAY)
}

/// Splits a resize message off the front of `bytes`, if one is there, from what follows it.
pub fn split_resize(bytes: &[u8]) -> Option<((u16, u16), &[u8])> {
    match bytes {
        [RESIZE_DISPLAY, w1, w0, h1, h0, rest @ ..] => {
            Some(((u16::from_be_bytes([*w1, *w0]), u16::from_be_bytes([*h1, *h0])), rest))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHERE: Position = Position { x: 10, y: 20, width: 720, height: 1560 };

    #[test]
    fn a_touch_has_the_layout_the_server_reads() {
        let message = touch(Touch::Down, WHERE);
        assert_eq!(message.len(), 32);
        assert_eq!(&message[..2], [TOUCH, 0]);
        assert_eq!(&message[2..10], [0xff; 8]);
        assert_eq!(&message[10..14], 10i32.to_be_bytes());
        assert_eq!(&message[14..18], 20i32.to_be_bytes());
        assert_eq!(&message[18..20], 720u16.to_be_bytes());
        assert_eq!(&message[20..22], 1560u16.to_be_bytes());
        assert_eq!(&message[22..24], [0xff, 0xff]);
        assert_eq!(&message[24..28], 1u32.to_be_bytes());
        assert_eq!(&message[28..32], 1u32.to_be_bytes());

        let lifted = touch(Touch::Up, WHERE);
        assert_eq!(&lifted[22..24], [0, 0]);
        assert_eq!(&lifted[28..32], 0u32.to_be_bytes());
    }

    #[test]
    fn a_scroll_is_a_fraction_of_its_range() {
        let message = scroll(WHERE, 0.0, 16.0);
        assert_eq!(message.len(), 21);
        assert_eq!(&message[13..15], [0, 0]);
        assert_eq!(&message[15..17], i16::MAX.to_be_bytes());
    }

    #[test]
    fn text_stops_at_a_whole_character() {
        let message = text(&"è".repeat(200));
        assert_eq!(&message[1..5], 300u32.to_be_bytes());
        assert_eq!(message.len(), 305);
        assert_eq!(text(&format!("a{}", "è".repeat(200))).len(), 304);
    }

    #[test]
    fn key_and_app_messages_are_framed() {
        assert_eq!(key(Press::Up, 66, 0).len(), 14);
        assert_eq!(start_app("a.b"), [START_APP, 3, b'a', b'.', b'b']);
        assert_eq!(resize_display(1280, 800), [RESIZE_DISPLAY, 5, 0, 3, 32]);
    }

    #[test]
    fn a_resize_splits_from_what_follows() {
        let mut bytes = resize_display(1280, 800);
        bytes.extend(back(Press::Down));
        assert_eq!(split_resize(&bytes), Some(((1280, 800), &back(Press::Down)[..])));
        assert_eq!(split_resize(&back(Press::Down)), None);
        assert_eq!(split_resize(&bytes[..3]), None);
        assert!(partial_resize(&[]) && partial_resize(&bytes[..3]));
        assert!(!partial_resize(&bytes) && !partial_resize(&back(Press::Down)));
    }
}
