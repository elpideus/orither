//! Starts Orither with the computer once a phone is linked, because a link that
//! only works while the app happens to be open would be no link at all.

use tauri::AppHandle;

const ASKED: &str = "autostart-set";

/// Only ever does this once, so that turning it off in Settings stays off.
pub fn enable_once(app: &AppHandle) {
    use tauri_plugin_autostart::ManagerExt;

    if cfg!(debug_assertions) || crate::preferences::read::<bool>(app, ASKED).is_some() {
        return;
    }
    crate::preferences::write(app, ASKED, &true);
    if let Err(err) = app.autolaunch().enable() {
        tracing::warn!("could not start with the computer: {err}");
    }
}
