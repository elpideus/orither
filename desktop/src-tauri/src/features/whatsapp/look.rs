//! How WhatsApp Web looks in Orither: in Orither's colours, which follow the
//! desktop's, and in the desktop's type. It is only a style sheet, laid over
//! WhatsApp's own as a user's would be, and leaves the page itself as it is.
//!
//! WhatsApp Web keeps its colours in CSS variables, most of them its design
//! system's `--WDS-…`, each with an `-RGB` twin for see-through shades. It sets
//! them at several depths of the page, so the sheet sets them on every element.

use crate::theme::Theme;

/// What WhatsApp Web's design system calls each colour, in Orither's.
const SYSTEM: [(&str, Shade); 47] = [
    ("accent", Shade::Solid(Role::Accent)),
    ("accent-emphasized", Shade::Mix(Role::Accent, Role::Text, 0.2)),
    ("accent-deemphasized", Shade::Mix(Role::View, Role::Accent, 0.16)),
    ("app-wash", Shade::Solid(Role::Window)),
    ("background-wash-plain", Shade::Solid(Role::View)),
    ("background-wash-inset", Shade::Solid(Role::View)),
    ("background-elevated-wash-plain", Shade::Solid(Role::Button)),
    ("background-elevated-wash-inset", Shade::Solid(Role::Button)),
    ("surface-default", Shade::Solid(Role::View)),
    ("surface-emphasized", Shade::Solid(Role::Button)),
    ("surface-elevated-default", Shade::Solid(Role::Button)),
    ("surface-elevated-emphasized", Shade::Mix(Role::Button, Role::Text, 0.06)),
    ("surface-highlight", Shade::Clear(Role::Text, 0.06)),
    ("surface-pressed", Shade::Clear(Role::Text, 0.12)),
    ("surface-inverse", Shade::Solid(Role::Text)),
    ("components-surface-nav-bar", Shade::Solid(Role::View)),
    ("components-active-list-row", Shade::Clear(Role::Accent, 0.16)),
    ("components-filter-surface-selected", Shade::Clear(Role::Accent, 0.16)),
    ("components-badge-muted", Shade::Clear(Role::Text, 0.1)),
    ("components-badge-muted-dot", Shade::Clear(Role::Text, 0.1)),
    ("components-outline-profile-photo", Shade::Clear(Role::Text, 0.08)),
    ("content-default", Shade::Solid(Role::Text)),
    ("content-action-default", Shade::Solid(Role::Text)),
    ("content-deemphasized", Shade::Solid(Role::Dim)),
    ("content-disabled", Shade::Mix(Role::Dim, Role::View, 0.45)),
    ("content-action-emphasized", Shade::Solid(Role::Accent)),
    ("content-external-link", Shade::Solid(Role::Accent)),
    ("content-inverse", Shade::Solid(Role::Window)),
    ("content-on-accent", Shade::Solid(Role::OnAccent)),
    ("lines-divider", Shade::Clear(Role::Text, 0.08)),
    ("lines-outline-deemphasized", Shade::Clear(Role::Text, 0.08)),
    ("lines-outline-default", Shade::Mix(Role::Dim, Role::View, 0.35)),
    ("persistent-activity-indicator", Shade::Solid(Role::Accent)),
    ("persistent-always-branded", Shade::Solid(Role::Accent)),
    ("systems-bubble-surface-incoming", Shade::Solid(Role::Button)),
    ("systems-bubble-surface-outgoing", Shade::Mix(Role::Button, Role::Accent, 0.28)),
    ("systems-bubble-surface-system", Shade::Solid(Role::View)),
    ("systems-bubble-surface-e2e", Shade::Solid(Role::View)),
    ("systems-bubble-surface-business", Shade::Solid(Role::View)),
    ("systems-bubble-content-deemphasized", Shade::Solid(Role::Dim)),
    ("systems-chat-background-wallpaper", Shade::Solid(Role::Window)),
    ("systems-chat-foreground-wallpaper", Shade::Clear(Role::Text, 0.05)),
    ("systems-chat-surface-composer", Shade::Solid(Role::Button)),
    ("systems-chat-surface-tray", Shade::Solid(Role::Window)),
    ("systems-status-seen", Shade::Solid(Role::Dim)),
    ("secondary-positive", Shade::Solid(Role::Accent)),
    ("secondary-positive-deemphasized", Shade::Mix(Role::View, Role::Accent, 0.16)),
];

/// The colours of WhatsApp Web's older parts, which name them on their own.
const OLDER: [(&str, Shade); 21] = [
    ("panel-background", Shade::Solid(Role::View)),
    ("panel-background-deep", Shade::Solid(Role::Window)),
    ("panel-input-background", Shade::Solid(Role::Button)),
    ("app-background-deeper", Shade::Solid(Role::Window)),
    ("modal-backdrop-solid", Shade::Solid(Role::Window)),
    ("picker-background", Shade::Solid(Role::Button)),
    ("empty-state-background", Shade::Solid(Role::View)),
    ("overlay", Shade::Solid(Role::Window)),
    ("text-primary-strong", Shade::Solid(Role::Text)),
    ("text-muted", Shade::Solid(Role::Dim)),
    ("text-action", Shade::Solid(Role::Accent)),
    ("icon-high-emphasis", Shade::Solid(Role::Accent)),
    ("badge-pending", Shade::Solid(Role::Accent)),
    ("status-ring-unread", Shade::Solid(Role::Accent)),
    ("link-alt", Shade::Solid(Role::Accent)),
    ("splashscreen-startup-background", Shade::Solid(Role::Window)),
    ("splashscreen-startup-background-plain", Shade::Solid(Role::Window)),
    ("splashscreen-progress-primary", Shade::Solid(Role::Accent)),
    ("splashscreen-progress-background", Shade::Solid(Role::Button)),
    ("splashscreen-primary-title", Shade::Solid(Role::Text)),
    ("splashscreen-secondary-lighter", Shade::Solid(Role::Dim)),
];

/// The older parts' colours that see-through shades are made of, as `r, g, b`.
const OLDER_RGB: [(&str, Role); 8] = [
    ("panel-background-rgb", Role::View),
    ("background-default-rgb", Role::View),
    ("background-default-hover-rgb", Role::Button),
    ("overlay-rgb", Role::Window),
    ("primary-rgb", Role::Text),
    ("primary-strong-rgb", Role::Text),
    ("primary-stronger-rgb", Role::Text),
    ("secondary-rgb", Role::Dim),
];

/// The type Orither's own pages are in.
const FONT: &str = "system-ui, sans-serif";

/// The style sheet that dresses WhatsApp Web in this theme.
pub fn stylesheet(theme: &Theme) -> String {
    let Some(colours) = Colours::of(theme) else {
        return String::new();
    };
    let mut variables = vec![("--font-family-default".to_owned(), FONT.to_owned())];
    for (name, shade) in SYSTEM {
        let (colour, alpha) = colours.shade(shade);
        variables.push((format!("--WDS-{name}"), colour.css(alpha)));
        variables.push((format!("--WDS-{name}-RGB"), colour.rgb()));
    }
    for (name, shade) in OLDER {
        let (colour, alpha) = colours.shade(shade);
        variables.push((format!("--{name}"), colour.css(alpha)));
    }
    for (name, role) in OLDER_RGB {
        variables.push((format!("--{name}"), colours.role(role).rgb()));
    }
    let (accent, text) = (colours.role(Role::Accent), colours.role(Role::Text));
    let one = |name: &str, value: String| vec![(name.to_owned(), value)];
    [
        ("*", variables),
        ("body, input, textarea, button, select", one("font-family", FONT.to_owned())),
        ("[contenteditable], input, textarea", one("caret-color", accent.css(1.0))),
        ("::selection", one("background-color", accent.css(0.32))),
        ("::-webkit-scrollbar-thumb", one("background-color", text.css(0.16))),
    ]
    .iter()
    .map(|(selector, declarations)| rule(selector, declarations))
    .collect()
}

/// A rule of the sheet. A user's sheet wins over the page's only where it
/// insists.
fn rule(selector: &str, declarations: &[(String, String)]) -> String {
    let body: String = declarations.iter().map(|(name, value)| format!("{name}: {value} !important; ")).collect();
    format!("{selector} {{ {body}}}\n")
}

/// A colour of Orither's theme.
#[derive(Clone, Copy)]
enum Role {
    Window,
    View,
    Button,
    Text,
    Dim,
    Accent,
    OnAccent,
}

/// How a colour of WhatsApp Web's comes out of the theme's.
#[derive(Clone, Copy)]
enum Shade {
    Solid(Role),
    /// The first, with this share of the second mixed in.
    Mix(Role, Role, f64),
    /// See-through, this opaque.
    Clear(Role, f64),
}

struct Colours([Colour; 7]);

impl Colours {
    fn of(theme: &Theme) -> Option<Self> {
        let roles = [&theme.window, &theme.view, &theme.button, &theme.text, &theme.dim, &theme.accent, &theme.on_accent];
        let mut colours = [Colour([0; 3]); 7];
        for (colour, css) in colours.iter_mut().zip(roles) {
            *colour = Colour::parse(css)?;
        }
        Some(Self(colours))
    }

    fn role(&self, role: Role) -> Colour {
        self.0[role as usize]
    }

    /// The colour, and how opaque it is.
    fn shade(&self, shade: Shade) -> (Colour, f64) {
        match shade {
            Shade::Solid(role) => (self.role(role), 1.0),
            Shade::Mix(base, other, share) => (self.role(base).mix(self.role(other), share), 1.0),
            Shade::Clear(role, alpha) => (self.role(role), alpha),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Colour([u8; 3]);

impl Colour {
    /// A colour as `#rrggbb`.
    fn parse(css: &str) -> Option<Self> {
        let hex = css.strip_prefix('#').filter(|hex| hex.len() == 6)?;
        let part = |at: usize| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok();
        Some(Self([part(0)?, part(2)?, part(4)?]))
    }

    fn mix(self, other: Self, share: f64) -> Self {
        let blend = |index: usize| f64::from(self.0[index]) * (1.0 - share) + f64::from(other.0[index]) * share;
        Self(std::array::from_fn(|index| blend(index).round() as u8))
    }

    fn css(self, alpha: f64) -> String {
        let [red, green, blue] = self.0;
        if alpha < 1.0 { format!("rgba({}, {alpha})", self.rgb()) } else { format!("#{red:02x}{green:02x}{blue:02x}") }
    }

    fn rgb(self) -> String {
        let [red, green, blue] = self.0;
        format!("{red}, {green}, {blue}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme;

    #[test]
    fn mixes_colours() {
        let (black, white) = (Colour([0, 0, 0]), Colour([255, 255, 255]));
        assert_eq!(black.mix(white, 0.5), Colour([128, 128, 128]));
        assert_eq!(Colour::parse("#3ddc84"), Some(Colour([0x3d, 0xdc, 0x84])));
        assert_eq!(Colour::parse("3ddc84"), None);
        assert_eq!(white.css(0.16), "rgba(255, 255, 255, 0.16)");
    }

    #[test]
    fn every_colour_wins_over_whatsapps() {
        let sheet = stylesheet(&theme::current());
        assert!(sheet.contains("--WDS-accent: #"));
        assert!(sheet.contains("--WDS-accent-RGB: "));
        let declarations = sheet.split(['{', '}']).skip(1).step_by(2).flat_map(|block| block.split(';'));
        for declaration in declarations.map(str::trim).filter(|declaration| !declaration.is_empty()) {
            assert!(declaration.ends_with("!important"), "{declaration}");
        }
    }
}
