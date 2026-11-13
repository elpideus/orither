//! Follows the desktop's colors. KDE keeps them in `kdeglobals`, which is also
//! what makes the theme apply here the moment it changes in System Settings.
//! Other Linux desktops state a light or dark preference and an accent through
//! the XDG portal. Anywhere else the UI keeps its own default: neutrals close
//! to Breeze Dark, with a green accent.

use std::{collections::HashMap, fs, path::PathBuf, time::Duration};

use serde::Serialize;
use tauri::{AppHandle, Emitter, async_runtime};

/// What the app hears whenever the colors change.
pub const CHANGED: &str = "theme";
const CHECK_EVERY: Duration = Duration::from_secs(2);
#[cfg(target_os = "linux")]
const PORTAL_TIMEOUT: Duration = Duration::from_millis(400);

#[derive(Clone, PartialEq, Serialize)]
pub struct Theme {
    pub window: String,
    pub view: String,
    pub button: String,
    pub text: String,
    pub dim: String,
    pub accent: String,
    pub on_accent: String,
}

#[tauri::command]
pub fn theme() -> Option<Theme> {
    read()
}

/// The desktop's colors, or else Orither's own, which the UI keeps too.
pub fn current() -> Theme {
    read().unwrap_or_else(|| palette(false, None))
}

fn read() -> Option<Theme> {
    read_kde().or_else(read_portal)
}

/// Tells the UI whenever the colors change.
pub fn watch(app: AppHandle) {
    async_runtime::spawn(async move {
        let mut last = read();
        loop {
            tokio::time::sleep(CHECK_EVERY).await;
            let current = read();
            if current != last {
                let _ = app.emit(CHANGED, &current);
                last = current;
            }
        }
    });
}

fn kdeglobals() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let config = config.or_else(|| Some(PathBuf::from(std::env::var_os("HOME")?).join(".config")))?;
    Some(config.join("kdeglobals"))
}

fn read_kde() -> Option<Theme> {
    let is_kde = std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|desktop| desktop.contains("KDE"));
    if !is_kde {
        return None;
    }
    parse(&fs::read_to_string(kdeglobals()?).ok()?)
}

fn parse(text: &str) -> Option<Theme> {
    let mut sections: HashMap<&str, HashMap<&str, &str>> = HashMap::new();
    let mut current = "";
    for line in text.lines() {
        if let Some(name) = line.strip_prefix('[').and_then(|line| line.strip_suffix(']')) {
            current = name;
        } else if let Some((key, value)) = line.split_once('=') {
            sections.entry(current).or_default().insert(key, value);
        }
    }

    let color = |section: &str, key: &str| {
        let value = sections.get(section)?.get(key)?;
        let parts: Vec<u8> = value.split(',').map(|part| part.trim().parse()).collect::<Result<_, _>>().ok()?;
        let [red, green, blue] = parts[..] else { return None };
        Some(format!("#{red:02x}{green:02x}{blue:02x}"))
    };
    let accent = color("General", "AccentColor").or_else(|| color("Colors:Selection", "BackgroundNormal"))?;
    Some(Theme {
        window: color("Colors:Window", "BackgroundNormal")?,
        view: color("Colors:View", "BackgroundNormal")?,
        button: color("Colors:Button", "BackgroundNormal")?,
        text: color("Colors:Window", "ForegroundNormal")?,
        dim: color("Colors:Window", "ForegroundInactive")?,
        on_accent: color("Colors:Selection", "ForegroundNormal")?,
        accent,
    })
}

#[cfg(target_os = "linux")]
fn read_portal() -> Option<Theme> {
    use dbus::{arg::Variant, blocking::Connection};

    const PREFERS_LIGHT: u32 = 2;
    let session = Connection::new_session().ok()?;
    let portal = session.with_proxy("org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop", PORTAL_TIMEOUT);
    let read = "ReadOne";
    let settings = "org.freedesktop.portal.Settings";
    let namespace = "org.freedesktop.appearance";
    let (Variant(scheme),): (Variant<u32>,) = portal.method_call(settings, read, (namespace, "color-scheme")).ok()?;
    let accent: Result<(Variant<(f64, f64, f64)>,), _> = portal.method_call(settings, read, (namespace, "accent-color"));
    Some(palette(scheme == PREFERS_LIGHT, accent.ok().and_then(|(Variant(accent),)| accent_color(accent))))
}

#[cfg(not(target_os = "linux"))]
fn read_portal() -> Option<Theme> {
    None
}

/// Portal colors run from 0 to 1, and anything outside that means none was chosen.
fn accent_color((red, green, blue): (f64, f64, f64)) -> Option<[u8; 3]> {
    [red, green, blue].iter().all(|part| (0.0..=1.0).contains(part)).then(|| [red, green, blue].map(|part| (part * 255.0).round() as u8))
}

/// Orither's own neutrals, close to Breeze, in the wanted brightness, around the wanted accent.
fn palette(light: bool, accent: Option<[u8; 3]>) -> Theme {
    let [red, green, blue] = accent.unwrap_or([0x3d, 0xdc, 0x84]);
    let brightness = 0.299 * f64::from(red) + 0.587 * f64::from(green) + 0.114 * f64::from(blue);
    let [window, view, button, text, dim] = if light {
        ["#f3f3f3", "#fbfbfb", "#ffffff", "#1b1b1b", "#5c5c5c"]
    } else {
        ["#171717", "#1c1c1c", "#252525", "#ebebeb", "#a0a0a0"]
    };
    Theme {
        window: window.into(),
        view: view.into(),
        button: button.into(),
        text: text.into(),
        dim: dim.into(),
        accent: format!("#{red:02x}{green:02x}{blue:02x}"),
        on_accent: if brightness > 140.0 { "#05230f" } else { "#ffffff" }.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_portal_accent_is_used_only_when_one_was_chosen() {
        assert_eq!(accent_color((1.0, 0.0, 0.2)), Some([255, 0, 51]));
        assert_eq!(accent_color((-1.0, -1.0, -1.0)), None);
    }

    #[test]
    fn the_palette_keeps_text_readable_on_the_accent() {
        assert_eq!(palette(false, None).on_accent, "#05230f");
        assert_eq!(palette(true, Some([0, 0, 120])).on_accent, "#ffffff");
        assert_eq!(palette(true, None).window, "#f3f3f3");
    }

    #[test]
    fn reads_a_color_scheme() {
        let theme = parse(
            "[Colors:Button]\nBackgroundNormal=37,37,37\n\
             [Colors:Selection]\nBackgroundNormal=230,165,217\nForegroundNormal=52,30,48\n\
             [Colors:View]\nBackgroundNormal=28,28,28\n\
             [Colors:Window]\nBackgroundNormal=23,23,23\nForegroundNormal=235,235,235\nForegroundInactive=160,160,160\n",
        )
        .unwrap();
        assert_eq!(theme.window, "#171717");
        assert_eq!(theme.accent, "#e6a5d9");
    }

    #[test]
    fn the_accent_setting_wins_over_the_selection() {
        let theme = parse(
            "[General]\nAccentColor=61,220,132\n[Colors:Button]\nBackgroundNormal=1,1,1\n\
             [Colors:Selection]\nBackgroundNormal=2,2,2\nForegroundNormal=3,3,3\n\
             [Colors:View]\nBackgroundNormal=4,4,4\n\
             [Colors:Window]\nBackgroundNormal=5,5,5\nForegroundNormal=6,6,6\nForegroundInactive=7,7,7\n",
        )
        .unwrap();
        assert_eq!(theme.accent, "#3ddc84");
    }
}
