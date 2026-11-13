//! The tray icon, which is what is left of Orither when its window is closed.

use serde_json::json;
use tauri::{
    AppHandle, Manager, Wry, async_runtime,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

use crate::link::Link;

const TRAY: &str = "main";

/// The menu, with an item that installs `update` when one is ready.
fn menu(app: &AppHandle, update: Option<&str>) -> tauri::Result<Menu<Wry>> {
    let open = MenuItem::with_id(app, "open", "Open Orither", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    match update {
        Some(version) => {
            let label = format!("Restart to update to {version}");
            let update = MenuItem::with_id(app, "update", label, true, None::<&str>)?;
            Menu::with_items(app, &[&open, &update, &quit])
        }
        None => Menu::with_items(app, &[&open, &quit]),
    }
}

pub fn install(app: &AppHandle) -> tauri::Result<()> {
    TrayIconBuilder::with_id(TRAY)
        .icon(app.default_window_icon().cloned().expect("the bundle has an icon"))
        .tooltip("Orither")
        .menu(&menu(app, None)?)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => crate::show_window(app),
            "update" => {
                let app = app.clone();
                async_runtime::spawn(async move {
                    if let Err(err) = app.state::<Link>().call("updates", "install", json!({})).await {
                        tracing::warn!("updating from the tray: {err:#}");
                    }
                });
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                crate::show_window(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

/// Offers to install the update to `version` from the tray, or stops offering with `None`.
pub fn offer_update(app: &AppHandle, version: Option<&str>) {
    let Some(tray) = app.tray_by_id(TRAY) else { return };
    match menu(app, version) {
        Ok(menu) => {
            let _ = tray.set_menu(Some(menu));
            let tooltip = version.map_or("Orither".to_owned(), |version| format!("Orither, {version} is ready to install"));
            let _ = tray.set_tooltip(Some(tooltip));
        }
        Err(err) => tracing::warn!("tray menu: {err}"),
    }
}
