//! Shows the phone on this computer: its whole screen, or a single app running
//! on a display of its own. Either way the phone captures and encodes, and a
//! window in a process of its own decodes and draws, see [`orither_viewer`].
//!
//! An app can also come over from the phone while it runs. It then moves to the
//! window's display as it is, and back to the phone when the window closes. Or
//! it opens from one of its notifications, where the notification leads.

use std::{
    collections::HashMap,
    env, fs,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, Result};
use orither_link::{PeerId, Route, Session};
use orither_viewer::{Options, control};
use serde::Deserialize;
use serde_json::{Value, json};
use tauri::{Manager, async_runtime};
use tokio::{
    io::{self, AsyncReadExt, AsyncWriteExt},
    process::{ChildStdin, ChildStdout, Command},
    time,
};

use super::{Feature, Phone, helper::ensure_engine};
use crate::VIEWER;

/// The size an app's display starts at, unless its window says otherwise.
const APP_DISPLAY: (u32, u32) = (1100, 720);
/// The density of an app's display, at which an app lays itself out as on a tablet.
const APP_DPI: u32 = 200;
/// How long a new window may take to say how large it is.
const SIZED_WITHIN: Duration = Duration::from_secs(3);
/// Video encoders take sizes in multiples of this.
const ALIGNMENT: u32 = 8;
/// The bit rate and the longest side of a picture that a route can carry.
/// Bluetooth moves about 500 kbit/s, and a picture that outruns it arrives ever later.
fn quality(route: Route) -> (u32, u32) {
    match route {
        Route::Local => (16_000_000, 1920),
        Route::Internet | Route::Relay => (6_000_000, 1920),
        Route::Nearby => (350_000, 960),
    }
}

/// A display size within what the route carries, kept to the window's shape.
fn display_size((width, height): (u32, u32), max_size: u32) -> (u32, u32) {
    let longest = width.max(height).max(1);
    let (width, height) =
        if longest > max_size { (width * max_size / longest, height * max_size / longest) } else { (width, height) };
    let align = |side: u32| (side / ALIGNMENT * ALIGNMENT).max(ALIGNMENT);
    (align(width), align(height))
}

/// One phone screen on its way to one window.
struct Showing {
    options: Vec<String>,
    window: Options,
    /// Sent to the phone before the first frame and after the window closes.
    first: Vec<Vec<u8>>,
    last: Vec<Vec<u8>>,
    /// An app already running on the phone, to move onto the window's display.
    task: Option<i64>,
    /// A notification to follow on the window's display.
    lead: Option<Lead>,
}

/// The app in front on the phone, as the phone describes it.
#[derive(Deserialize)]
struct Front {
    task: i64,
    package: String,
    label: String,
}

impl Showing {
    fn common(session: &Session) -> Vec<String> {
        let (bit_rate, max_size) = quality(session.route());
        [
            "video_codec=h264",
            "audio=false",
            "control=true",
            "send_device_meta=false",
            "send_dummy_byte=false",
            "clipboard_autosync=false",
            "power_on=true",
            "keep_active=true",
        ]
        .map(str::to_owned)
        .into_iter()
        .chain([format!("video_bit_rate={bit_rate}"), format!("max_size={max_size}")])
        .collect()
    }

    fn mirror(session: &Session, title: String, screen_off: bool) -> Self {
        let (first, last) = if screen_off {
            (vec![control::display_power(false)], vec![control::display_power(true)])
        } else {
            (Vec::new(), Vec::new())
        };
        let window = Options { title, app_id: "orither-mirror".into(), icon: None, flex: false };
        Self { options: Self::common(session), window, first, last, task: None, lead: None }
    }

    /// An app on a display of its own, which follows the size of its window.
    fn own_display(session: &Session, package: &str, label: String, icon: Option<PathBuf>) -> Self {
        let mut options = Self::common(session);
        options.extend(["flex_display=true", "vd_system_decorations=false"].map(str::to_owned));
        let window = Options { title: label, app_id: app_id(package), icon, flex: true };
        Self { options, window, first: Vec::new(), last: Vec::new(), task: None, lead: None }
    }

    fn app(session: &Session, package: &str, label: String, icon: Option<PathBuf>) -> Self {
        let mut showing = Self::own_display(session, package, label, icon);
        showing.first.push(control::start_app(package));
        showing
    }

    /// An app that already runs on the phone. Its display hands it back to the
    /// phone when it goes away, so closing the window loses nothing.
    fn hand_off(session: &Session, front: Front, icon: Option<PathBuf>) -> Self {
        let mut showing = Self::own_display(session, &front.package, front.label, icon);
        showing.options.push("vd_destroy_content=false".into());
        showing.task = Some(front.task);
        showing
    }

    /// An app opened from one of its notifications.
    fn led(session: &Session, lead: Lead, icon: Option<PathBuf>) -> Self {
        let mut showing = Self::own_display(session, &lead.app, lead.label.clone(), icon);
        showing.lead = Some(lead);
        showing
    }

    /// Runs until the window closes or the phone goes away. `started` learns the
    /// phone's name for the capture as soon as there is one.
    async fn run(mut self, session: Session, started: impl FnOnce(&Value) + Send) -> Result<()> {
        let mut viewer = Command::new(env::current_exe()?)
            .arg(VIEWER)
            .args(self.window.to_args())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .context("opening the window")?;
        let pictures = viewer.stdin.take().context("no way to reach the window")?;
        let mut input = viewer.stdout.take().context("no way to reach the window")?;

        // The window opens first, so that the display starts at the window's size
        // and an app adapts to the computer once rather than again a moment later.
        let mut early = Vec::new();
        if self.window.flex {
            let Some(size) = first_size(&mut input, &mut early).await else { return Ok(()) };
            let (width, height) = display_size(size, quality(session.route()).1);
            self.options.push(format!("new_display={width}x{height}/{APP_DPI}"));
        }

        let capture = session.call("screen", "start", json!({ "options": self.options, "channels": 2 })).await?;
        started(&capture);
        let shown = self.show(&session, &capture, pictures, input, &early).await;
        let _ = session.call("screen", "stop", json!({ "session": capture })).await;
        shown
    }

    async fn show(
        &self,
        session: &Session,
        capture: &Value,
        mut pictures: ChildStdin,
        mut input: ChildStdout,
        early: &[u8],
    ) -> Result<()> {
        let mut video = session.open("screen", "attach", json!({ "session": capture, "channel": 0 })).await?;
        let mut steering = session.open("screen", "attach", json!({ "session": capture, "channel": 1 })).await?;
        // The phone names its codec once it can take a resize. One sent before
        // that stops the capture, and the window sends its size as it opens.
        let mut codec = [0; 4];
        video.recv.read_exact(&mut codec).await.context("the phone's capture stopped as it started")?;
        pictures.write_all(&codec).await?;
        for message in &self.first {
            steering.send.write_all(message).await?;
        }
        steering.send.write_all(early).await?;
        if let Some(task) = self.task {
            let moved = json!({ "session": capture, "task": task });
            session.call("screen", "move", moved).await.context("moving the app to this computer")?;
        }
        if let Some(lead) = &self.lead
            && let Err(err) = session.call("notifications", "open", lead.open(capture)).await
        {
            // Without its notification, the app opens as from its launcher.
            tracing::warn!("following a notification: {err:#}");
            steering.send.write_all(&control::start_app(&lead.app)).await?;
        }

        let mut ignored = io::sink();
        tokio::select! {
            _ = io::copy(&mut video.recv, &mut pictures) => {}
            _ = io::copy(&mut input, &mut steering.send) => {}
            _ = io::copy(&mut steering.recv, &mut ignored) => {}
        }
        for message in &self.last {
            steering.send.write_all(message).await?;
        }
        steering.send.flush().await?;
        Ok(())
    }
}

/// Reads the window's first message, which in a window that follows its size is
/// that size, and keeps it with anything else read for the phone. None when the
/// window closed before saying.
async fn first_size(input: &mut ChildStdout, early: &mut Vec<u8>) -> Option<(u32, u32)> {
    let reading = async {
        let mut buffer = [0; 256];
        // A message can come in pieces, as stdout writes up to a newline byte first.
        while control::partial_resize(early) {
            let read = input.read(&mut buffer).await.ok().filter(|&read| read > 0)?;
            early.extend_from_slice(&buffer[..read]);
        }
        Some(())
    };
    if let Ok(None) = time::timeout(SIZED_WITHIN, reading).await {
        return None;
    }
    let size = control::split_resize(early).map(|((width, height), _)| (width.into(), height.into()));
    Some(size.unwrap_or(APP_DISPLAY))
}

fn app_id(package: &str) -> String {
    format!("orither-{package}")
}

#[derive(Deserialize)]
struct App {
    package: String,
    label: Option<String>,
}

/// One of the phone's notifications, and the app that posted it.
pub struct Lead {
    pub key: String,
    pub app: String,
    pub label: String,
}

impl Lead {
    /// What the phone needs to open the notification on a capture's display.
    fn open(&self, capture: &Value) -> Value {
        json!({ "session": capture, "key": self.key, "app": self.app })
    }
}

/// The app's icon, fetched from the phone the first time it is needed.
pub async fn app_icon(phone: &Phone, session: &Session, package: &str) -> Result<PathBuf> {
    let folder = phone.app.path().app_data_dir()?.join("apps");
    let icon = folder.join(format!("{package}.png"));
    if !icon.exists() {
        let mut stream = session.open("apps", "icon", json!({ "package": package })).await?;
        let mut png = Vec::new();
        AsyncReadExt::read_to_end(&mut stream.recv, &mut png).await?;
        // Written whole or not at all, as the desktop may read it any moment.
        fs::create_dir_all(&folder)?;
        let part = icon.with_extension("part");
        fs::write(&part, png)?;
        fs::rename(part, &icon)?;
    }
    Ok(icon)
}

#[derive(Clone)]
pub struct Screen {
    phone: Phone,
    /// What is on the screen, and the phone's capture behind each once it runs.
    open: Arc<Mutex<HashMap<String, Option<Value>>>>,
}

impl Screen {
    pub fn new(phone: Phone) -> Self {
        Self { phone, open: Arc::default() }
    }

    /// Shows something unless it is on the screen already.
    fn present(&self, key: String, session: Session, showing: Showing) {
        if self.open.lock().unwrap().insert(key.clone(), None).is_some() {
            return;
        }
        let (open, phone) = (self.open.clone(), self.phone.clone());
        async_runtime::spawn(async move {
            let started = |capture: &Value| {
                if let Some(slot) = open.lock().unwrap().get_mut(&key) {
                    *slot = Some(capture.clone());
                }
            };
            if let Err(err) = showing.run(session, started).await {
                tracing::warn!("{key}: {err:#}");
                phone.tell_ui("screen", "failed", json!({ "error": format!("{err:#}") }));
            }
            open.lock().unwrap().remove(&key);
        });
    }

    async fn view(&self, body: &Value) -> Result<Value> {
        let session = self.phone.session()?;
        ensure_engine(&self.phone, &session).await?;
        let title = self.phone.node.peers().into_iter().find(|peer| peer.online).map(|peer| peer.peer.device.name);
        let title = title.unwrap_or_else(|| "Your phone".into());
        let showing = Showing::mirror(&session, title, body["screenOff"].as_bool().unwrap_or(false));
        self.present("mirror".into(), session, showing);
        Ok(Value::Null)
    }

    async fn app(&self, app: App) -> Result<Value> {
        let session = self.phone.session()?;
        ensure_engine(&self.phone, &session).await?;
        let label = match app.label {
            Some(label) => label,
            None => self.label_of(&session, &app.package).await?,
        };
        let icon = self.identity(&session, &app.package, &label).await;
        if let Err(err) = &icon {
            tracing::warn!("giving {} its icon: {err:#}", app.package);
        }
        let showing = Showing::app(&session, &app.package, label, icon.ok());
        self.present(app.package, session, showing);
        Ok(Value::Null)
    }

    /// Takes over the app in front on the phone, which the user sent from there.
    async fn hand_off(&self, peer: PeerId) -> Result<()> {
        let session = self.phone.node.session(&peer).context("your phone is not connected")?;
        ensure_engine(&self.phone, &session).await?;
        let front = session.call("screen", "foreground", Value::Null).await?;
        // With nothing to send, the phone has said so itself.
        let Some(front) = serde_json::from_value::<Option<Front>>(front)? else { return Ok(()) };
        let capture = self.open.lock().unwrap().get(&front.package).cloned();
        match capture {
            // The app has a window already, which may have lost it to the phone.
            Some(Some(capture)) => {
                session.call("screen", "move", json!({ "session": capture, "task": front.task })).await?;
            }
            Some(None) => {}
            None => {
                let icon = self.identity(&session, &front.package, &front.label).await;
                if let Err(err) = &icon {
                    tracing::warn!("giving {} its icon: {err:#}", front.package);
                }
                let key = front.package.clone();
                let showing = Showing::hand_off(&session, front, icon.ok());
                self.present(key, session, showing);
            }
        }
        Ok(())
    }

    /// Opens where a notification leads, in the window of the app that posted it.
    pub async fn notification(&self, lead: Lead) -> Result<()> {
        let session = self.phone.session()?;
        ensure_engine(&self.phone, &session).await?;
        let capture = self.open.lock().unwrap().get(&lead.app).cloned();
        match capture {
            Some(Some(capture)) => {
                session.call("notifications", "open", lead.open(&capture)).await?;
            }
            // Its window is on the way.
            Some(None) => {}
            None => {
                let icon = self.identity(&session, &lead.app, &lead.label).await;
                if let Err(err) = &icon {
                    tracing::warn!("giving {} its icon: {err:#}", lead.app);
                }
                let key = lead.app.clone();
                let showing = Showing::led(&session, lead, icon.ok());
                self.present(key, session, showing);
            }
        }
        Ok(())
    }

    async fn label_of(&self, session: &Session, package: &str) -> Result<String> {
        let apps = session.call("apps", "list", json!({})).await?;
        let listed = apps.as_array().and_then(|apps| apps.iter().find(|app| app["package"] == package));
        Ok(listed.and_then(|app| app["label"].as_str()).unwrap_or(package).to_owned())
    }

    /// Fetches the app's icon and, where the desktop reads them, writes the entry
    /// that shows it in the taskbar and the launcher as if it were installed.
    async fn identity(&self, session: &Session, package: &str, label: &str) -> Result<PathBuf> {
        let icon = app_icon(&self.phone, session, package).await?;
        #[cfg(any(target_os = "linux", windows))]
        write_entry(package, label, &icon)?;
        #[cfg(not(any(target_os = "linux", windows)))]
        let _ = label;
        Ok(icon)
    }
}

#[cfg(target_os = "linux")]
fn write_entry(package: &str, label: &str, icon: &Path) -> Result<()> {
    let data = env::var_os("XDG_DATA_HOME").map(PathBuf::from);
    let data = data.or_else(|| Some(Path::new(&env::var_os("HOME")?).join(".local/share")));
    let folder = data.context("no home folder")?.join("applications");
    fs::create_dir_all(&folder)?;
    let id = app_id(package);
    // An AppImage runs from a mount that goes away with it.
    let program = env::var_os("APPIMAGE").map_or_else(env::current_exe, |path| Ok(path.into()))?;
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName={label}\nComment=On your phone, through Orither\n\
         Exec=\"{program}\" {OPEN_APP} {package}\nIcon={icon}\nStartupWMClass={id}\nTerminal=false\n\
         Categories=Utility;\n",
        label = label.replace('\n', " "),
        program = program.display(),
        icon = icon.display(),
    );
    fs::write(folder.join(format!("{id}.desktop")), entry)?;
    Ok(())
}

#[cfg(any(windows, test))]
mod shortcut;

/// Puts the app in the Start menu, under the id its window takes, which gives the
/// window the shortcut's name and icon in the taskbar.
#[cfg(windows)]
fn write_entry(package: &str, label: &str, icon: &Path) -> Result<()> {
    shortcut::write(package, label, icon)
}

/// Opens an app from outside, as its launcher entry does.
pub const OPEN_APP: &str = "--open-app";

#[async_trait::async_trait]
impl Feature for Screen {
    fn id(&self) -> &'static str {
        "screen"
    }

    async fn command(&self, method: &str, body: &Value) -> Option<Result<Value>> {
        Some(match method {
            "view" => self.view(body).await,
            "app" => match serde_json::from_value(body.clone()) {
                Ok(app) => self.app(app).await,
                Err(err) => Err(err.into()),
            },
            _ => return None,
        })
    }

    fn notice(&self, peer: &PeerId, method: &str, _body: &Value) -> bool {
        if method != "handoff" {
            return false;
        }
        let (screen, peer) = (self.clone(), *peer);
        async_runtime::spawn(async move {
            if let Err(err) = screen.hand_off(peer).await {
                tracing::warn!("taking an app over: {err:#}");
                screen.phone.tell_ui("screen", "failed", json!({ "error": format!("{err:#}") }));
            }
        });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_display_fits_the_route_and_the_encoder() {
        assert_eq!(display_size((1650, 1080), 1920), (1648, 1080));
        assert_eq!(display_size((3300, 2160), 1920), (1920, 1256));
        assert_eq!(display_size((2160, 3300), 960), (624, 960));
        assert_eq!(display_size((3, 0), 1920), (8, 8));
    }
}
