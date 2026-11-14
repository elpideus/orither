//! The Orither desktop app. Everything specific to one feature lives in
//! [`features`]; a feature that only shows the phone's data needs nothing here
//! at all, because the UI's calls pass straight through to the phone.

mod adb;
mod autostart;
mod features;
#[cfg(target_os = "linux")]
mod gstreamer;
mod link;
mod media;
mod preferences;
mod theme;
mod tray;

use std::time::{Duration, Instant};

use anyhow::Context;
use orither_link::{Device, DeviceKind, NodeConfig};
use serde_json::json;
use tauri::{AppHandle, Manager, RunEvent, WindowEvent, async_runtime};
use tauri_plugin_autostart::MacosLauncher;

use crate::{features::screen::OPEN_APP, link::Link};

/// Passed when the session starts Orither at login, which stays in the tray.
pub(crate) const BACKGROUND: &str = "--background";
/// Runs this program as a window onto the phone, see [`orither_viewer`].
pub(crate) const VIEWER: &str = "--viewer";
/// How long an app opened from the launcher waits for the phone.
const PHONE_APPEARS_IN: Duration = Duration::from_secs(20);

/// Decides what this process is: a window onto the phone, or Orither itself.
pub fn start() {
    #[cfg(target_os = "linux")]
    gstreamer::prepare();
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some(VIEWER) {
        return run();
    }
    let Some(options) = orither_viewer::Options::from_args(args) else {
        return eprintln!("the viewer needs a title, an app id, an icon and a mode");
    };
    if let Err(err) = orither_viewer::run(options) {
        eprintln!("{err:#}");
        std::process::exit(1);
    }
}

/// The app that `args` asks to open, if they ask for one.
fn requested_app(args: &[String]) -> Option<String> {
    args.iter().position(|arg| arg == OPEN_APP).and_then(|index| args.get(index + 1)).cloned()
}

/// Opens an app for its launcher entry, waiting for the phone if it has to.
fn open_app(app: &AppHandle, package: String) {
    let app = app.clone();
    async_runtime::spawn(async move {
        let link = app.state::<Link>();
        let waited = Instant::now();
        while link.phone().session().is_err() && waited.elapsed() < PHONE_APPEARS_IN {
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        if let Err(err) = link.call("screen", "app", json!({ "package": package })).await {
            tracing::warn!("opening {package}: {err:#}");
            show_window(&app);
        }
    });
}

fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| match requested_app(&args) {
            Some(package) => open_app(app, package),
            None => show_window(app),
        }))
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec![BACKGROUND])))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .register_asynchronous_uri_scheme_protocol("orither", media::serve)
        .invoke_handler(tauri::generate_handler![
            link::device,
            link::peers,
            link::invite,
            link::cancel_invite,
            link::unpair,
            link::call,
            link::selected,
            link::select,
            theme::theme,
        ])
        .setup(|app| {
            adb::locate(app.handle());
            let data_dir = app.path().app_data_dir().context("no data directory")?;
            let name = gethostname::gethostname().to_string_lossy().into_owned();
            let device = Device { name, kind: DeviceKind::Desktop };
            let config = NodeConfig { data_dir, device, offline: false };
            let link = async_runtime::block_on(Link::start(app.handle().clone(), config))?;
            app.manage(link);
            tray::install(app.handle())?;
            theme::watch(app.handle().clone());
            #[cfg(unix)]
            quit_on_terminate(app.handle().clone());
            let args: Vec<String> = std::env::args().collect();
            match requested_app(&args) {
                Some(package) => open_app(app.handle(), package),
                None if !args.iter().any(|arg| arg == BACKGROUND) => show_window(app.handle()),
                None => {}
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the window keeps the phone linked, as the tray icon shows.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("starting Orither")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                async_runtime::block_on(app.state::<Link>().shutdown());
            }
        });
}

/// A logout or a `kill` asks with SIGTERM. Quitting the usual way tells the phone, and
/// takes away the devices Orither added.
#[cfg(unix)]
fn quit_on_terminate(app: AppHandle) {
    use tokio::signal::unix::{SignalKind, signal};

    async_runtime::spawn(async move {
        if let Ok(mut terminate) = signal(SignalKind::terminate()) {
            terminate.recv().await;
            app.exit(0);
        }
    });
}

pub(crate) fn show_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
