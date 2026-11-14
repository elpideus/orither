//! What Windows calls the keys and buttons a phone presses, and the arithmetic that
//! turns a phone's fractions of a pixel into Windows' whole events. Nothing here
//! calls Windows, so it is tested on every system, and the Windows module next to
//! it only has to send what comes out.

use super::{Button, Key};

// Virtual-key codes, from `WinUser.h`.
pub const VK_BACK: u16 = 0x08;
pub const VK_TAB: u16 = 0x09;
pub const VK_RETURN: u16 = 0x0d;
pub const VK_SHIFT: u16 = 0x10;
pub const VK_CONTROL: u16 = 0x11;
pub const VK_MENU: u16 = 0x12;
pub const VK_ESCAPE: u16 = 0x1b;
pub const VK_SPACE: u16 = 0x20;
pub const VK_PRIOR: u16 = 0x21;
pub const VK_NEXT: u16 = 0x22;
pub const VK_END: u16 = 0x23;
pub const VK_HOME: u16 = 0x24;
pub const VK_LEFT: u16 = 0x25;
pub const VK_UP: u16 = 0x26;
pub const VK_RIGHT: u16 = 0x27;
pub const VK_DOWN: u16 = 0x28;
pub const VK_SNAPSHOT: u16 = 0x2c;
pub const VK_INSERT: u16 = 0x2d;
pub const VK_DELETE: u16 = 0x2e;
pub const VK_LWIN: u16 = 0x5b;
pub const VK_RWIN: u16 = 0x5c;
pub const VK_APPS: u16 = 0x5d;
pub const VK_DIVIDE: u16 = 0x6f;
pub const VK_F1: u16 = 0x70;
pub const VK_NUMLOCK: u16 = 0x90;
pub const VK_RCONTROL: u16 = 0xa3;
pub const VK_RMENU: u16 = 0xa5;

/// What a wheel notch is worth to Windows.
pub const WHEEL_DELTA: f64 = 120.0;
/// A notch for each 50 pixels a finger scrolls feels like a laptop's touchpad, as on Linux.
const PIXELS_PER_NOTCH: f64 = 50.0;

/// The bits of `VkKeyScanExW`'s answer that say which modifiers a character needs.
const NEEDS_SHIFT: u8 = 1;

/// The key Windows knows for one of the phone's named keys. Control, Alt and Shift go
/// by the codes that mean either side, which Windows turns into the left one, as a
/// keyboard's own would be. The Windows key has no such code, so it is the left one.
pub fn virtual_key(key: Key) -> u16 {
    match key {
        Key::Enter => VK_RETURN,
        Key::Backspace => VK_BACK,
        Key::Delete => VK_DELETE,
        Key::Tab => VK_TAB,
        Key::Escape => VK_ESCAPE,
        Key::Space => VK_SPACE,
        Key::Left => VK_LEFT,
        Key::Right => VK_RIGHT,
        Key::Up => VK_UP,
        Key::Down => VK_DOWN,
        Key::Home => VK_HOME,
        Key::End => VK_END,
        Key::PageUp => VK_PRIOR,
        Key::PageDown => VK_NEXT,
        Key::Function(number) => VK_F1 + u16::from(number.clamp(1, 12)) - 1,
        Key::Control => VK_CONTROL,
        Key::Alt => VK_MENU,
        Key::Shift => VK_SHIFT,
        Key::Meta => VK_LWIN,
    }
}

/// Keys that sit apart from the typing block on a full keyboard, whose scan codes
/// start with 0xE0. Without the flag, Windows takes the arrows for the number pad's
/// and Delete for its full stop whenever Num Lock is on.
pub fn extended(vk: u16) -> bool {
    matches!(
        vk,
        VK_PRIOR
            | VK_NEXT
            | VK_END
            | VK_HOME
            | VK_LEFT
            | VK_UP
            | VK_RIGHT
            | VK_DOWN
            | VK_SNAPSHOT
            | VK_INSERT
            | VK_DELETE
            | VK_LWIN
            | VK_RWIN
            | VK_APPS
            | VK_DIVIDE
            | VK_NUMLOCK
            | VK_RCONTROL
            | VK_RMENU
    )
}

/// Holding one of these makes text a shortcut, which applications look for as keys
/// rather than as characters.
pub fn makes_a_shortcut(vk: u16) -> bool {
    matches!(vk, VK_CONTROL | VK_MENU | VK_LWIN)
}

/// How a character of text reaches Windows.
#[derive(Debug, PartialEq)]
pub enum Typed {
    /// A key of its own, as a line break is Enter.
    Key(u16),
    Char(char),
}

/// As on Linux, a line break is Enter, a tab is Tab, and other control characters type nothing.
pub fn typed(character: char) -> Option<Typed> {
    match character {
        '\n' => Some(Typed::Key(VK_RETURN)),
        '\t' => Some(Typed::Key(VK_TAB)),
        _ if character.is_control() => None,
        _ => Some(Typed::Char(character)),
    }
}

/// The UTF-16 units that type `character` with `KEYEVENTF_UNICODE`, each with
/// whether it is the key going up. A character beyond the first 65,536 is a surrogate
/// pair: both go down before both come up, so the application gets the two halves
/// in a row and puts them back together.
pub fn unicode_events(character: char) -> Vec<(u16, bool)> {
    let mut buffer = [0; 2];
    let units = character.encode_utf16(&mut buffer);
    units.iter().map(|unit| (*unit, false)).chain(units.iter().map(|unit| (*unit, true))).collect()
}

/// Reads what `VkKeyScanExW` says of a character: the key that types it on the
/// layout, and whether that takes Shift. A character the layout has not got, or
/// that takes Control or Alt as well (AltGr on many layouts), is `None`, and goes as
/// Unicode instead.
pub fn layout_key(scan: i16) -> Option<(u16, bool)> {
    let [vk, modifiers] = scan.to_le_bytes();
    (scan != -1 && modifiers & !NEEDS_SHIFT == 0).then_some((u16::from(vk), modifiers & NEEDS_SHIFT != 0))
}

/// The button Windows calls `button`. Windows swaps the buttons of injected input
/// too when the user made the right button the main one, and the phone's left button
/// is a tap, which should always be the main button, as it is on Linux.
pub fn physical(button: Button, swapped: bool) -> Button {
    match (button, swapped) {
        (Button::Left, true) => Button::Right,
        (Button::Right, true) => Button::Left,
        (button, _) => button,
    }
}

/// Wheel units for pixels a finger scrolled.
pub fn wheel_units(pixels: f64) -> f64 {
    pixels * WHEEL_DELTA / PIXELS_PER_NOTCH
}

/// Keeps what is left of a pixel, or of a wheel unit, for the next event, so that
/// slow motion adds up instead of rounding away.
#[derive(Default)]
pub struct Carry(f64);

impl Carry {
    /// The whole part of what has built up, which leaves only the fraction behind.
    pub fn take(&mut self, amount: f64) -> i32 {
        let total = self.0 + amount;
        if !total.is_finite() {
            self.0 = 0.0;
            return 0;
        }
        self.0 = total.fract();
        // Saturates, though no finger moves that far.
        total.trunc() as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEYS: [Key; 18] = [
        Key::Enter, Key::Backspace, Key::Delete, Key::Tab, Key::Escape, Key::Space, Key::Left, Key::Right, Key::Up, Key::Down,
        Key::Home, Key::End, Key::PageUp, Key::PageDown, Key::Control, Key::Alt, Key::Shift, Key::Meta,
    ];

    #[test]
    fn named_keys_are_virtual_keys() {
        assert_eq!(virtual_key(Key::Enter), 0x0d);
        assert_eq!(virtual_key(Key::Backspace), 0x08);
        assert_eq!(virtual_key(Key::PageUp), 0x21);
        assert_eq!(virtual_key(Key::Left), 0x25);
        assert_eq!(virtual_key(Key::Down), 0x28);
        assert_eq!(virtual_key(Key::Delete), 0x2e);
        assert_eq!(virtual_key(Key::Meta), 0x5b);
        assert_eq!(virtual_key(Key::Function(1)), 0x70);
        assert_eq!(virtual_key(Key::Function(12)), 0x7b);
        let codes: std::collections::HashSet<u16> = KEYS.into_iter().chain((1..=12).map(Key::Function)).map(virtual_key).collect();
        assert_eq!(codes.len(), KEYS.len() + 12);
    }

    #[test]
    fn keys_apart_from_the_typing_block_are_extended() {
        let apart = [Key::Delete, Key::Left, Key::Right, Key::Up, Key::Down, Key::Home, Key::End, Key::PageUp, Key::PageDown, Key::Meta];
        for key in KEYS {
            assert_eq!(extended(virtual_key(key)), apart.contains(&key), "{key:?}");
        }
        assert!((1..=12).all(|number| !extended(virtual_key(Key::Function(number)))));
        assert!(extended(VK_RCONTROL) && extended(VK_RMENU) && extended(VK_INSERT));
        assert!(!extended(VK_CONTROL) && !extended(VK_MENU) && !extended(VK_SHIFT));
    }

    #[test]
    fn control_alt_and_windows_make_shortcuts() {
        assert!([Key::Control, Key::Alt, Key::Meta].map(virtual_key).into_iter().all(makes_a_shortcut));
        assert!(!makes_a_shortcut(virtual_key(Key::Shift)));
    }

    #[test]
    fn text_types_as_on_linux() {
        assert_eq!(typed('a'), Some(Typed::Char('a')));
        assert_eq!(typed('é'), Some(Typed::Char('é')));
        assert_eq!(typed('\n'), Some(Typed::Key(VK_RETURN)));
        assert_eq!(typed('\t'), Some(Typed::Key(VK_TAB)));
        assert_eq!(typed('\r'), None);
        assert_eq!(typed('\u{7}'), None);
    }

    #[test]
    fn a_surrogate_pair_goes_down_whole_before_it_comes_up() {
        assert_eq!(unicode_events('a'), [(0x61, false), (0x61, true)]);
        assert_eq!(unicode_events('€'), [(0x20ac, false), (0x20ac, true)]);
        assert_eq!(unicode_events('😀'), [(0xd83d, false), (0xde00, false), (0xd83d, true), (0xde00, true)]);
    }

    #[test]
    fn reads_the_layout() {
        // 'c' on a US layout, 'C' with Shift, '@' with AltGr on a German one, and nothing.
        assert_eq!(layout_key(0x0043), Some((0x43, false)));
        assert_eq!(layout_key(0x0143), Some((0x43, true)));
        assert_eq!(layout_key(0x0651), None);
        assert_eq!(layout_key(-1), None);
    }

    #[test]
    fn the_left_button_is_always_the_main_one() {
        assert_eq!(physical(Button::Left, false), Button::Left);
        assert_eq!(physical(Button::Left, true), Button::Right);
        assert_eq!(physical(Button::Right, true), Button::Left);
        assert_eq!(physical(Button::Middle, true), Button::Middle);
    }

    #[test]
    fn a_finger_scrolls_a_notch_every_fifty_pixels() {
        assert_eq!(wheel_units(50.0), 120.0);
        assert_eq!(wheel_units(-25.0), -60.0);
    }

    #[test]
    fn fractions_add_up() {
        let mut carry = Carry::default();
        assert_eq!([0.75, 0.75, 0.75, -2.5].map(|amount| carry.take(amount)), [0, 1, 1, -2]);
        assert_eq!(carry.take(0.25), 0);
        assert_eq!(carry.take(f64::INFINITY), 0);
        assert_eq!(carry.take(0.5), 0);
        assert_eq!(carry.take(0.5), 1);
    }
}
