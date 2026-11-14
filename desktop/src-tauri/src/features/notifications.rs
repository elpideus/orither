//! The phone's notifications, shown as this computer's own. A click opens the
//! app in a window, where the notification leads on the phone, see
//! [`Screen::notification`], except WhatsApp's, which open in the WhatsApp page.
//! Closing one here takes it off the phone too.
// On macOS there are no clicks to hear and no pictures to show.
#![cfg_attr(not(any(target_os = "linux", windows)), allow(dead_code))]

use std::{
    collections::{HashMap, HashSet, VecDeque},
    io::Cursor,
    path::PathBuf,
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use orither_link::{PeerId, Session};
use serde::Deserialize;
use serde_json::{Value, json};
use tauri::async_runtime;
use tokio::{
    io::AsyncReadExt,
    sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel},
};

use super::{
    Feature, Phone,
    screen::{self, Lead, Screen},
    whatsapp::{self, Chat},
};
use crate::preferences;

const ENABLED: &str = "notifications-enabled";
/// More than a notification's picture needs, which the phone sends small.
const MAX_PICTURE: u64 = 1024 * 1024;
/// How long Orither waits, as it quits, for its notifications to go.
const CLOSED_WITHIN: Duration = Duration::from_secs(1);
/// How many notifications may pop up within [`BURST`]. Those past it, as when the phone
/// comes back with many at once, are listed without popping up, as quiet ones are.
const POPUPS: usize = 3;
const BURST: Duration = Duration::from_secs(10);

/// A notification as the phone describes it.
#[derive(Deserialize)]
struct Shown {
    key: String,
    app: String,
    label: String,
    title: String,
    text: String,
    /// Whether to call attention to it, which an update may not.
    alert: bool,
    quiet: bool,
    picture: bool,
    /// The app's own id for the conversation it is from, if it is from one.
    #[serde(default)]
    conversation: Option<String>,
}

/// A notification with all this computer shows of it.
struct Note {
    shown: Shown,
    /// The phone it came from.
    origin: String,
    icon: Option<PathBuf>,
    picture: Option<Pixels>,
}

/// A picture as a notification service takes it, in rows of red, green, blue and alpha.
struct Pixels {
    width: i32,
    height: i32,
    row: i32,
    data: Vec<u8>,
}

/// Reads a picture the phone sent, which it sends as PNG.
fn pixels(png: &[u8]) -> Option<Pixels> {
    let mut decoder = png::Decoder::new(Cursor::new(png));
    decoder.set_transformations(png::Transformations::normalize_to_color8() | png::Transformations::ALPHA);
    let mut reader = decoder.read_info().ok()?;
    let mut data = vec![0; reader.output_buffer_size()?];
    let frame = reader.next_frame(&mut data).ok()?;
    if frame.color_type != png::ColorType::Rgba {
        return None;
    }
    data.truncate(frame.buffer_size());
    Some(Pixels {
        width: frame.width.try_into().ok()?,
        height: frame.height.try_into().ok()?,
        row: frame.line_size.try_into().ok()?,
        data,
    })
}

/// A picture as a PNG file holds it, for a desktop that shows pictures from files only.
#[cfg(any(windows, test))]
fn png(picture: &Pixels) -> Option<Vec<u8>> {
    if picture.row != picture.width.checked_mul(4)? {
        return None;
    }
    let mut png = Vec::new();
    let mut encoder = png::Encoder::new(&mut png, picture.width.try_into().ok()?, picture.height.try_into().ok()?);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().ok()?;
    writer.write_image_data(&picture.data).ok()?;
    writer.finish().ok()?;
    Some(png)
}

/// What a Windows toast shows of a notification. Its title and text are bound to the
/// toast's data, so that an update changes them where they show without popping up again.
/// The logo is the notification's own picture, round as the phone shows a person's, or
/// else the app's icon.
#[cfg(any(windows, test))]
fn toast(logo: Option<(&std::path::Path, bool)>, attribution: &str, silent: bool) -> String {
    let escape = |text: &str| text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
    let logo = logo.map_or_else(String::new, |(path, round)| {
        let crop = if round { r#" hint-crop="circle""# } else { "" };
        let source = escape(&format!("file:///{}", path.display()));
        format!(r#"<image placement="appLogoOverride"{crop} src="{source}"/>"#)
    });
    let attribution = escape(attribution);
    let audio = if silent { r#"<audio silent="true"/>"# } else { "" };
    format!(
        r#"<toast><visual><binding template="ToastGeneric">{logo}<text>{{title}}</text><text>{{text}}</text><text placement="attribution">{attribution}</text></binding></visual>{audio}</toast>"#
    )
}

/// What the desktop's notifications are asked to do.
enum Order {
    Show(Note),
    Close(String),
    /// Closes all but these.
    Keep(HashSet<String>),
    CloseAll,
    /// Closes all and says when that is done, as Orither quits.
    Stop(mpsc::Sender<()>),
}

/// What the user did with a notification here.
enum Answer {
    /// With the title it showed, and the conversation it is from.
    Clicked { lead: Lead, title: String, conversation: Option<String> },
    Dismissed(String),
}

/// What the phone said, kept in order.
enum Change {
    Posted(PeerId, Shown),
    Removed(String),
    Connected,
}

/// What a notification says, as this computer shows it.
#[derive(PartialEq)]
struct Words {
    label: String,
    title: String,
    text: String,
    picture: bool,
}

impl Words {
    fn of(shown: &Shown) -> Self {
        Self { label: shown.label.clone(), title: shown.title.clone(), text: shown.text.clone(), picture: shown.picture }
    }
}

/// How a notification the phone posted shows here.
#[derive(Debug, PartialEq)]
enum Heed {
    /// As the phone asks.
    Show,
    /// Without popping up, as too many popped up just now.
    Hush,
    /// Not at all, as it says what it said already.
    Skip,
}

/// What the phone's notifications said here, so that each pops up once. Apps post theirs
/// again unchanged, many at once as the phone comes back online, and the phone passes each
/// on as it comes, calling for attention unless the app asked to alert only once.
#[derive(Default)]
struct Memory {
    said: HashMap<String, Words>,
    /// The notifications that popped up lately, and when.
    popped: VecDeque<(Instant, String)>,
}

impl Memory {
    /// How to show a notification the phone posted, which it remembers.
    fn heed(&mut self, shown: &Shown, now: Instant) -> Heed {
        let words = Words::of(shown);
        if self.said.get(&shown.key) == Some(&words) {
            return Heed::Skip;
        }
        self.said.insert(shown.key.clone(), words);
        if !shown.alert || shown.quiet {
            return Heed::Show;
        }
        self.popped.retain(|(at, _)| now.saturating_duration_since(*at) < BURST);
        // One that changes as it shows, as a chat does with each new line, takes no more room.
        if self.popped.iter().any(|(_, key)| *key == shown.key) {
            return Heed::Show;
        }
        if self.popped.len() >= POPUPS {
            return Heed::Hush;
        }
        self.popped.push_back((now, shown.key.clone()));
        Heed::Show
    }

    /// Takes in what the phone has as it connects: what shows here already, and what came
    /// meanwhile, which it is late to call attention to.
    fn connected(&mut self, keys: &HashSet<String>, listed: &[Shown]) {
        self.said.retain(|key, _| keys.contains(key));
        self.said.extend(listed.iter().map(|shown| (shown.key.clone(), Words::of(shown))));
    }

    fn removed(&mut self, key: &str) {
        self.said.remove(key);
    }
}

struct Shared {
    phone: Phone,
    enabled: AtomicBool,
    shade: shade::Shade,
    memory: Mutex<Memory>,
}

impl Shared {
    fn memory(&self) -> MutexGuard<'_, Memory> {
        self.memory.lock().unwrap_or_else(PoisonError::into_inner)
    }

    async fn posted(&self, peer: PeerId, mut shown: Shown) -> Result<()> {
        if !self.enabled.load(Ordering::Relaxed) {
            return Ok(());
        }
        let session = self.phone.node.session(&peer).context("your phone is not connected")?;
        // Before fetching anything, as the same one can come many times a second.
        let heed = self.memory().heed(&shown, Instant::now());
        match heed {
            Heed::Skip => return Ok(()),
            Heed::Hush => shown.quiet = true,
            Heed::Show => {}
        }
        let phone = self.phone.node.peers().into_iter().find(|status| status.peer.id == peer);
        let origin = phone.map_or_else(|| "Your phone".into(), |status| status.peer.device.name);
        let icon = screen::app_icon(&self.phone, &session, &shown.app).await;
        if let Err(err) = &icon {
            tracing::debug!("fetching {}'s icon: {err:#}", shown.app);
        }
        let picture = if shown.picture { self.picture(&session, &shown.key).await } else { None };
        self.shade.order(Order::Show(Note { shown, origin, icon: icon.ok(), picture }));
        Ok(())
    }

    async fn picture(&self, session: &Session, key: &str) -> Option<Pixels> {
        let fetched = async {
            let stream = session.open("notifications", "picture", json!({ "key": key })).await?;
            let mut png = Vec::new();
            stream.recv.take(MAX_PICTURE).read_to_end(&mut png).await?;
            anyhow::Ok(png)
        };
        match fetched.await {
            Ok(png) => pixels(&png),
            Err(err) => {
                tracing::debug!("fetching a notification's picture: {err:#}");
                None
            }
        }
    }

    /// Closes those that went from the phone while it was away. Any that came
    /// meanwhile stay on the phone, as it is late to call attention to them, and
    /// so do they when their apps post them again as the phone comes back online.
    async fn connected(&self) -> Result<()> {
        let listed = self.phone.session()?.call("notifications", "list", Value::Null).await?;
        let listed = listed.as_array().map(Vec::as_slice).unwrap_or_default();
        let keys: HashSet<String> = listed.iter().filter_map(|shown| shown["key"].as_str()).map(str::to_owned).collect();
        let known: Vec<Shown> = listed.iter().filter_map(|shown| Shown::deserialize(shown).ok()).collect();
        self.memory().connected(&keys, &known);
        self.shade.order(Order::Keep(keys));
        Ok(())
    }

    async fn dismiss(&self, key: String) -> Result<()> {
        self.phone.session()?.call("notifications", "dismiss", json!({ "key": key })).await?;
        Ok(())
    }
}

/// Takes the phone's changes one after another, so that none overtakes another.
async fn follow(shared: Arc<Shared>, mut changes: UnboundedReceiver<Change>) {
    while let Some(change) = changes.recv().await {
        let done = match change {
            Change::Posted(peer, shown) => shared.posted(peer, shown).await,
            Change::Removed(key) => {
                shared.memory().removed(&key);
                shared.shade.order(Order::Close(key));
                Ok(())
            }
            Change::Connected => shared.connected().await,
        };
        if let Err(err) = done {
            tracing::warn!("following the phone's notifications: {err:#}");
        }
    }
}

async fn answer(shared: Arc<Shared>, screen: Screen, mut answers: UnboundedReceiver<Answer>) {
    while let Some(answer) = answers.recv().await {
        match answer {
            Answer::Clicked { lead, title, conversation } if whatsapp::owns(&lead.app) => {
                whatsapp::open(&shared.phone, Chat { title, id: conversation });
            }
            Answer::Clicked { lead, .. } => {
                let (screen, phone) = (screen.clone(), shared.phone.clone());
                async_runtime::spawn(async move {
                    if let Err(err) = screen.notification(lead).await {
                        tracing::warn!("opening a notification: {err:#}");
                        phone.tell_ui("screen", "failed", json!({ "error": format!("{err:#}") }));
                        crate::show_window(&phone.app);
                    }
                });
            }
            Answer::Dismissed(key) => {
                if let Err(err) = shared.dismiss(key).await {
                    tracing::debug!("dismissing a notification on the phone: {err:#}");
                }
            }
        }
    }
}

/// Whether the phone's notifications show here now.
// WhatsApp Web asks, which is built in on Linux only so far.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn mirrored(phone: &Phone) -> bool {
    phone.session().is_ok() && preferences::read(&phone.app, ENABLED).unwrap_or(true)
}

pub struct Notifications {
    shared: Arc<Shared>,
    changes: UnboundedSender<Change>,
    screen: Screen,
}

impl Notifications {
    pub fn start(phone: Phone, screen: Screen) -> Arc<dyn Feature> {
        let (answered, answers) = unbounded_channel();
        let (changed, changes) = unbounded_channel();
        let enabled = preferences::read(&phone.app, ENABLED).unwrap_or(true);
        let shade = shade::Shade::start(&phone, answered);
        let shared = Arc::new(Shared { phone, enabled: AtomicBool::new(enabled), shade, memory: Mutex::default() });
        async_runtime::spawn(follow(shared.clone(), changes));
        async_runtime::spawn(answer(shared.clone(), screen.clone(), answers));
        Arc::new(Self { shared, changes: changed, screen })
    }

    /// Opens a notification the UI lists, where a click on the desktop's own copy would.
    fn open(&self, body: &Value) -> Result<Value> {
        let text = |name: &str| body[name].as_str().map(str::to_owned);
        let lead = Lead {
            key: text("key").context("no notification")?,
            app: text("app").context("no app")?,
            label: text("label").unwrap_or_default(),
        };
        if whatsapp::owns(&lead.app) {
            whatsapp::open(&self.shared.phone, Chat { title: text("title").unwrap_or_default(), id: text("conversation") });
            return Ok(Value::Null);
        }
        let (screen, phone) = (self.screen.clone(), self.shared.phone.clone());
        async_runtime::spawn(async move {
            if let Err(err) = screen.notification(lead).await {
                phone.tell_ui("screen", "failed", json!({ "error": format!("{err:#}") }));
            }
        });
        Ok(Value::Null)
    }

    fn enable(&self, on: bool) -> Value {
        self.shared.enabled.store(on, Ordering::Relaxed);
        preferences::write(&self.shared.phone.app, ENABLED, &on);
        if !on {
            self.shared.shade.order(Order::CloseAll);
        }
        Value::Null
    }
}

#[async_trait::async_trait]
impl Feature for Notifications {
    fn id(&self) -> &'static str {
        "notifications"
    }

    async fn command(&self, method: &str, body: &Value) -> Option<Result<Value>> {
        Some(match method {
            "state" => Ok(json!({ "enabled": self.shared.enabled.load(Ordering::Relaxed) })),
            "enable" => Ok(self.enable(body["on"].as_bool().unwrap_or(true))),
            "open" => self.open(body),
            _ => return None,
        })
    }

    fn connected(&self, _session: &Session) {
        let _ = self.changes.send(Change::Connected);
    }

    /// Nothing that a notification here leads to works without Orither.
    fn stopping(&self) {
        let (done, closed) = mpsc::channel();
        self.shared.shade.order(Order::Stop(done));
        let _ = closed.recv_timeout(CLOSED_WITHIN);
    }

    fn notice(&self, peer: &PeerId, method: &str, body: &Value) -> bool {
        let change = match method {
            "posted" => match serde_json::from_value(body.clone()) {
                Ok(shown) => Change::Posted(*peer, shown),
                Err(err) => {
                    tracing::warn!("odd notification: {err}");
                    return true;
                }
            },
            "removed" => match body["key"].as_str() {
                Some(key) => Change::Removed(key.into()),
                None => return true,
            },
            _ => return false,
        };
        // The UI lists them too, on the home page.
        self.shared.phone.tell_ui_about(peer, "notifications", method, body.clone());
        let _ = self.changes.send(change);
        true
    }
}

/// The desktop's own notifications, through the freedesktop.org service that
/// Linux desktops run. It says when one is clicked or closed.
#[cfg(target_os = "linux")]
mod shade {
    use std::{
        collections::HashMap,
        sync::mpsc::{self, Receiver, Sender, TryRecvError},
        time::Duration,
    };

    use anyhow::Result;
    use dbus::{
        arg::{PropMap, RefArg, Variant},
        blocking::{Connection, Proxy},
        message::MatchRule,
    };
    use tokio::sync::mpsc::UnboundedSender;

    use super::{Answer, Lead, Note, Order, Phone};

    const SERVICE: &str = "org.freedesktop.Notifications";
    const PATH: &str = "/org/freedesktop/Notifications";
    const CALL_TIMEOUT: Duration = Duration::from_secs(2);
    /// How long the thread listens to the service before it looks for orders.
    const LISTEN_FOR: Duration = Duration::from_millis(200);
    /// Why the service closed a notification.
    const EXPIRED: u32 = 1;
    const DISMISSED: u32 = 2;
    const LOW: u8 = 0;
    const NORMAL: u8 = 1;
    /// The action a click on the notification itself invokes.
    const CLICK: &str = "default";

    pub struct Shade {
        orders: Sender<Order>,
    }

    impl Shade {
        pub fn start(_phone: &Phone, answers: UnboundedSender<Answer>) -> Self {
            let (orders, taken) = mpsc::channel();
            std::thread::spawn(move || {
                if let Err(err) = Desktop::run(&taken, answers) {
                    tracing::warn!("no notifications on this desktop: {err:#}");
                }
            });
            Self { orders }
        }

        pub fn order(&self, order: Order) {
            let _ = self.orders.send(order);
        }
    }

    #[derive(Clone, Copy)]
    enum Signal {
        Clicked(u32),
        Closed(u32, u32),
    }

    /// One of the phone's notifications, as the service knows it.
    struct Entry {
        id: u32,
        app: String,
        label: String,
        title: String,
        conversation: Option<String>,
        /// Gone from the screen, though it may still be in the desktop's history.
        expired: bool,
    }

    struct Desktop {
        bus: Connection,
        /// Whether the service reads a little HTML in a notification's text.
        markup: bool,
        shown: HashMap<String, Entry>,
        answers: UnboundedSender<Answer>,
    }

    fn hint(value: impl RefArg + 'static) -> Variant<Box<dyn RefArg>> {
        Variant(Box::new(value))
    }

    fn escape(text: &str) -> String {
        text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    }

    impl Desktop {
        fn run(orders: &Receiver<Order>, answers: UnboundedSender<Answer>) -> Result<()> {
            let bus = Connection::new_session()?;
            let (heard, signals) = mpsc::channel();
            let clicked = heard.clone();
            let on_click = move |(id, _action): (u32, String), _: &Connection, _: &_| clicked.send(Signal::Clicked(id)).is_ok();
            bus.add_match(MatchRule::new_signal(SERVICE, "ActionInvoked"), on_click)?;
            let on_close = move |(id, reason): (u32, u32), _: &Connection, _: &_| heard.send(Signal::Closed(id, reason)).is_ok();
            bus.add_match(MatchRule::new_signal(SERVICE, "NotificationClosed"), on_close)?;
            let service = bus.with_proxy(SERVICE, PATH, CALL_TIMEOUT);
            let (capabilities,): (Vec<String>,) = service.method_call(SERVICE, "GetCapabilities", ())?;
            let markup = capabilities.iter().any(|capability| capability == "body-markup");
            let mut desktop = Self { bus, markup, shown: HashMap::new(), answers };
            loop {
                desktop.bus.process(LISTEN_FOR)?;
                while let Ok(signal) = signals.try_recv() {
                    desktop.heard(signal);
                }
                loop {
                    match orders.try_recv() {
                        Ok(Order::Stop(done)) => {
                            desktop.close_all();
                            let _ = done.send(());
                            return Ok(());
                        }
                        Ok(order) => desktop.follow(order),
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => return Ok(()),
                    }
                }
            }
        }

        fn service(&self) -> Proxy<'_, &Connection> {
            self.bus.with_proxy(SERVICE, PATH, CALL_TIMEOUT)
        }

        fn follow(&mut self, order: Order) {
            let done = match order {
                Order::Show(note) => self.show(note),
                Order::Close(key) => self.close(&key),
                Order::Keep(keys) => {
                    let gone: Vec<String> = self.shown.keys().filter(|key| !keys.contains(*key)).cloned().collect();
                    gone.iter().try_for_each(|key| self.close(key))
                }
                Order::CloseAll | Order::Stop(_) => {
                    self.close_all();
                    Ok(())
                }
            };
            if let Err(err) = done {
                tracing::warn!("showing a notification: {err:#}");
            }
        }

        fn show(&mut self, note: Note) -> Result<()> {
            let Note { shown, origin, icon, picture } = note;
            let before = self.shown.get(&shown.key).map(|entry| (entry.id, entry.expired));
            // An update that asks for no attention brings nothing back.
            if !shown.alert && before.is_none_or(|(_, expired)| expired) {
                return Ok(());
            }
            let replaces = match before {
                Some((id, false)) => id,
                // A new one takes the place of the one left in the history.
                Some((id, true)) => {
                    let _: Result<(), _> = self.service().method_call(SERVICE, "CloseNotification", (id,));
                    0
                }
                None => 0,
            };
            let mut hints = PropMap::new();
            hints.insert("urgency".into(), hint(if shown.quiet { LOW } else { NORMAL }));
            hints.insert("x-kde-origin-name".into(), hint(origin));
            if !shown.alert {
                hints.insert("suppress-sound".into(), hint(true));
            }
            if let Some(picture) = picture {
                let data = (picture.width, picture.height, picture.row, true, 8, 4, picture.data);
                hints.insert("image-data".into(), hint(data));
            }
            let icon = icon.map(|icon| icon.to_string_lossy().into_owned()).unwrap_or_default();
            let summary = if shown.title.is_empty() { &shown.label } else { &shown.title };
            let body = if self.markup { escape(&shown.text) } else { shown.text.clone() };
            let actions = vec![CLICK, "Open"];
            let arguments = (shown.label.as_str(), replaces, icon, summary.as_str(), body, actions, hints, -1);
            let (id,): (u32,) = self.service().method_call(SERVICE, "Notify", arguments)?;
            let title = summary.clone();
            let (app, label, conversation) = (shown.app, shown.label, shown.conversation);
            let entry = Entry { id, app, label, title, conversation, expired: false };
            self.shown.insert(shown.key, entry);
            Ok(())
        }

        fn close(&mut self, key: &str) -> Result<()> {
            if let Some(entry) = self.shown.remove(key) {
                self.service().method_call::<(), _, _, _>(SERVICE, "CloseNotification", (entry.id,))?;
            }
            Ok(())
        }

        fn close_all(&mut self) {
            let keys: Vec<String> = self.shown.keys().cloned().collect();
            for key in keys {
                let _ = self.close(&key);
            }
        }

        fn heard(&mut self, signal: Signal) {
            let (Signal::Clicked(id) | Signal::Closed(id, _)) = signal;
            // The service tells every app about every notification.
            let Some(key) = self.shown.iter().find(|(_, entry)| entry.id == id).map(|(key, _)| key.clone()) else {
                return;
            };
            match signal {
                // It may still be in the desktop's history, where a click opens it all the same.
                Signal::Closed(_, EXPIRED) => self.shown.get_mut(&key).expect("just found").expired = true,
                Signal::Closed(_, reason) => {
                    self.shown.remove(&key);
                    if reason == DISMISSED {
                        let _ = self.answers.send(Answer::Dismissed(key));
                    }
                }
                Signal::Clicked(_) => {
                    let entry = self.shown.remove(&key).expect("just found");
                    let lead = Lead { key, app: entry.app, label: entry.label };
                    let (title, conversation) = (entry.title, entry.conversation);
                    let _ = self.answers.send(Answer::Clicked { lead, title, conversation });
                    // It goes from here as a tap closes the phone's shade, which is not
                    // a dismissal: whether a click reads it is for the app to say.
                    let _: Result<(), _> = self.service().method_call(SERVICE, "CloseNotification", (entry.id,));
                }
            }
        }
    }
}

/// The desktop's own notifications on Windows: toasts under Orither's name, which say when
/// one is clicked or closed. Windows says so to the running app only, as Orither has no COM
/// server for toasts, which an app needs for Windows to start it on a click: a click on a
/// toast left in the Action Center after Orither quits does nothing. Orither takes its
/// toasts away as it quits, and any a crash left behind as it starts again.
#[cfg(windows)]
mod shade {
    use std::{
        collections::HashMap,
        fs,
        path::{Path, PathBuf},
        sync::mpsc::{self, Receiver, Sender},
    };

    use anyhow::Result;
    use tauri::Manager;
    use tokio::sync::mpsc::UnboundedSender;
    use windows::{
        Data::Xml::Dom::XmlDocument,
        Foundation::TypedEventHandler,
        UI::Notifications::{
            NotificationData, NotificationSetting, NotificationUpdateResult, ToastDismissalReason, ToastDismissedEventArgs,
            ToastFailedEventArgs, ToastNotification, ToastNotificationHistory, ToastNotificationManager, ToastNotifier,
        },
        core::{HSTRING, IInspectable},
    };

    use super::{Answer, Lead, Note, Order, Phone, Pixels};

    /// The group of the toasts for the phone's notifications, apart from Orither's others.
    const GROUP: &str = "phone";
    /// The app id a build that is not installed borrows, as it has no Start menu shortcut
    /// to carry its own, the way Tauri's notification plugin does. Its toasts show as PowerShell's.
    const BORROWED: &str = "{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\WindowsPowerShell\\v1.0\\powershell.exe";

    pub struct Shade {
        heard: Sender<Heard>,
    }

    impl Shade {
        pub fn start(phone: &Phone, answers: UnboundedSender<Answer>) -> Self {
            let (heard, hearing) = mpsc::channel();
            let pictures = phone.app.path().app_cache_dir().ok().map(|folder| folder.join("notifications"));
            let (app, told) = (app_id(phone), heard.clone());
            std::thread::spawn(move || match Toasts::new(app, pictures, told, answers) {
                Ok(toasts) => toasts.run(&hearing),
                Err(err) => tracing::warn!("no notifications on this desktop: {err:#}"),
            });
            Self { heard }
        }

        pub fn order(&self, order: Order) {
            let _ = self.heard.send(Heard::Order(Box::new(order)));
        }
    }

    /// The app id Orither's installer gives its Start menu shortcut, its bundle identifier.
    /// Windows shows a toast under the name and icon of the shortcut with the toast's app id.
    fn app_id(phone: &Phone) -> HSTRING {
        let exe = std::env::current_exe().ok();
        let folder = exe.as_deref().and_then(Path::parent);
        let built = folder.is_some_and(|folder| folder.ends_with("target/debug") || folder.ends_with("target/release"));
        HSTRING::from(if built { BORROWED } else { phone.app.config().identifier.as_str() })
    }

    /// A toast's tag, which Windows keeps to 64 characters, where the phone's keys run longer.
    fn tag(key: &str) -> HSTRING {
        HSTRING::from(&blake3::hash(key.as_bytes()).to_hex()[..32])
    }

    /// Words without the control characters XML forbids, which a toast fails on.
    fn printable(text: &str) -> String {
        text.chars().filter(|c| !c.is_control() || matches!(c, '\n' | '\t')).collect()
    }

    /// What the toasts' thread hears: orders, and what the user did with a toast, by its serial.
    enum Heard {
        Order(Box<Order>),
        Clicked(u64),
        Closed(u64, ToastDismissalReason),
    }

    /// One of the phone's notifications, as a toast.
    struct Entry {
        /// Which showing of it this is, as Windows may still tell of one it replaced.
        serial: u64,
        toast: ToastNotification,
        tag: HSTRING,
        app: String,
        label: String,
        title: String,
        conversation: Option<String>,
        picture: Option<PathBuf>,
    }

    struct Toasts {
        app: HSTRING,
        group: HSTRING,
        notifier: ToastNotifier,
        history: ToastNotificationHistory,
        /// Where the notifications' pictures go, as a toast shows pictures from files only.
        pictures: Option<PathBuf>,
        shown: HashMap<String, Entry>,
        serial: u64,
        /// Numbers each update, so that Windows never takes an older one for the newer.
        sequence: u32,
        heard: Sender<Heard>,
        answers: UnboundedSender<Answer>,
    }

    impl Toasts {
        fn new(app: HSTRING, pictures: Option<PathBuf>, heard: Sender<Heard>, answers: UnboundedSender<Answer>) -> Result<Self> {
            let notifier = ToastNotificationManager::CreateToastNotifierWithId(&app)?;
            // Toasts from an app id without a Start menu shortcut go nowhere, and nothing says so.
            // Windows often cannot say for an app from outside the Store ("Element not found")
            // even when its toasts show, so only a clear no is worth a warning.
            match notifier.Setting() {
                Ok(NotificationSetting::Enabled) => {}
                Ok(setting) => tracing::warn!("Windows holds back Orither's notifications: {setting:?}"),
                Err(err) => tracing::debug!("whether Windows shows notifications as {app}: {err}"),
            }
            let history = ToastNotificationManager::History()?;
            let group = HSTRING::from(GROUP);
            // Those of an Orither that ended without taking them away lead nowhere now.
            if let Err(err) = history.RemoveGroupWithId(&group, &app) {
                tracing::debug!("clearing old notifications: {err}");
            }
            if let Some(folder) = &pictures {
                let _ = fs::remove_dir_all(folder);
            }
            Ok(Self { app, group, notifier, history, pictures, shown: HashMap::new(), serial: 0, sequence: 0, heard, answers })
        }

        fn run(mut self, hearing: &Receiver<Heard>) {
            while let Ok(heard) = hearing.recv() {
                match heard {
                    Heard::Order(order) => match *order {
                        Order::Stop(done) => {
                            self.close_all();
                            let _ = done.send(());
                            return;
                        }
                        order => self.follow(order),
                    },
                    Heard::Clicked(serial) => self.clicked(serial),
                    Heard::Closed(serial, reason) => self.closed(serial, reason),
                }
            }
        }

        fn follow(&mut self, order: Order) {
            let done = match order {
                Order::Show(note) => self.show(note),
                Order::Close(key) => {
                    self.close(&key);
                    Ok(())
                }
                Order::Keep(keys) => {
                    let gone: Vec<String> = self.shown.keys().filter(|key| !keys.contains(*key)).cloned().collect();
                    gone.iter().for_each(|key| self.close(key));
                    Ok(())
                }
                Order::CloseAll | Order::Stop(_) => {
                    self.close_all();
                    Ok(())
                }
            };
            if let Err(err) = done {
                tracing::warn!("showing a notification: {err:#}");
            }
        }

        fn show(&mut self, note: Note) -> Result<()> {
            let Note { shown, origin, icon, picture } = note;
            let title = if shown.title.is_empty() { shown.label.clone() } else { shown.title.clone() };
            // An update that asks for no attention changes the words where they show, and brings nothing back.
            if !shown.alert {
                return self.update(&shown.key, title, &shown.text, shown.conversation);
            }
            let picture = picture.and_then(|picture| self.keep_picture(&picture));
            let logo = picture.as_deref().map(|path| (path, true)).or_else(|| icon.as_deref().map(|path| (path, false)));
            let attribution = printable(&format!("{} · {origin}", shown.label));
            let content = XmlDocument::new()?;
            content.LoadXml(&HSTRING::from(super::toast(logo, &attribution, shown.quiet)))?;
            let toast = ToastNotification::CreateToastNotification(&content)?;
            let tag = tag(&shown.key);
            // A toast with the same tag and group takes the place of the one before, in the Action Center too.
            toast.SetTag(&tag)?;
            toast.SetGroup(&self.group)?;
            // Quiet ones go to the Action Center without popping up, as on the phone they are only listed.
            toast.SetSuppressPopup(shown.quiet)?;
            toast.SetData(&self.data(&title, &shown.text)?)?;
            self.serial += 1;
            self.hear(&toast, self.serial)?;
            self.notifier.Show(&toast)?;
            let (app, label, conversation) = (shown.app, shown.label, shown.conversation);
            let entry = Entry { serial: self.serial, toast, tag, app, label, title, conversation, picture };
            if let Some(before) = self.shown.insert(shown.key, entry) {
                self.forget_picture(before.picture);
            }
            Ok(())
        }

        fn update(&mut self, key: &str, title: String, text: &str, conversation: Option<String>) -> Result<()> {
            let Some(tag) = self.shown.get(key).map(|entry| entry.tag.clone()) else {
                return Ok(());
            };
            let data = self.data(&title, text)?;
            let updated = self.notifier.UpdateWithTagAndGroup(&data, &tag, &self.group)?;
            if updated == NotificationUpdateResult::Succeeded {
                if let Some(entry) = self.shown.get_mut(key) {
                    (entry.title, entry.conversation) = (title, conversation);
                }
            } else if updated == NotificationUpdateResult::NotificationNotFound {
                // Cleared from the Action Center, which Windows tells only apps from the Store.
                if let Some(entry) = self.shown.remove(key) {
                    self.forget_picture(entry.picture);
                }
            } else {
                tracing::warn!("updating a notification: Windows did not take the update");
            }
            Ok(())
        }

        /// The words a toast shows, for the places its content leaves for them.
        fn data(&mut self, title: &str, text: &str) -> windows::core::Result<NotificationData> {
            let data = NotificationData::new()?;
            let values = data.Values()?;
            values.Insert(&HSTRING::from("title"), &HSTRING::from(printable(title)))?;
            values.Insert(&HSTRING::from("text"), &HSTRING::from(printable(text)))?;
            self.sequence = self.sequence.wrapping_add(1).max(1);
            data.SetSequenceNumber(self.sequence)?;
            Ok(data)
        }

        /// Has Windows tell this thread when the toast is clicked or closed. It calls from
        /// threads of its own, and about a toast it replaced as well.
        fn hear(&self, toast: &ToastNotification, serial: u64) -> Result<()> {
            let heard = self.heard.clone();
            toast.Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(move |_, _| {
                let _ = heard.send(Heard::Clicked(serial));
                Ok(())
            }))?;
            let heard = self.heard.clone();
            toast.Dismissed(&TypedEventHandler::<ToastNotification, ToastDismissedEventArgs>::new(move |_, dismissed| {
                if let Some(reason) = dismissed.as_ref().and_then(|dismissed| dismissed.Reason().ok()) {
                    let _ = heard.send(Heard::Closed(serial, reason));
                }
                Ok(())
            }))?;
            toast.Failed(&TypedEventHandler::<ToastNotification, ToastFailedEventArgs>::new(|_, failed| {
                match failed.as_ref().and_then(|failed| failed.ErrorCode().ok()) {
                    Some(code) => tracing::warn!("showing a notification: {}", code.message()),
                    None => tracing::warn!("showing a notification: Windows did not show it"),
                }
                Ok(())
            }))?;
            Ok(())
        }

        fn find(&self, serial: u64) -> Option<String> {
            self.shown.iter().find(|(_, entry)| entry.serial == serial).map(|(key, _)| key.clone())
        }

        fn clicked(&mut self, serial: u64) {
            let Some((key, entry)) = self.find(serial).and_then(|key| self.shown.remove_entry(&key)) else {
                return;
            };
            // It goes from here as a tap closes the phone's shade, which is not a dismissal:
            // whether a click reads it is for the app to say.
            self.take_away(&entry);
            self.forget_picture(entry.picture);
            let lead = Lead { key, app: entry.app, label: entry.label };
            let _ = self.answers.send(Answer::Clicked { lead, title: entry.title, conversation: entry.conversation });
        }

        fn closed(&mut self, serial: u64, reason: ToastDismissalReason) {
            // One that timed out waits in the Action Center, where a click opens it all the same,
            // and one hidden was hidden by Orither, or replaced.
            if reason != ToastDismissalReason::UserCanceled {
                return;
            }
            let Some((key, entry)) = self.find(serial).and_then(|key| self.shown.remove_entry(&key)) else {
                return;
            };
            self.take_away(&entry);
            self.forget_picture(entry.picture);
            let _ = self.answers.send(Answer::Dismissed(key));
        }

        fn close(&mut self, key: &str) {
            if let Some(entry) = self.shown.remove(key) {
                self.take_away(&entry);
                self.forget_picture(entry.picture);
            }
        }

        fn close_all(&mut self) {
            for entry in std::mem::take(&mut self.shown).into_values() {
                let _ = self.notifier.Hide(&entry.toast);
            }
            if let Err(err) = self.history.RemoveGroupWithId(&self.group, &self.app) {
                tracing::warn!("closing the notifications: {err}");
            }
            if let Some(folder) = &self.pictures {
                let _ = fs::remove_dir_all(folder);
            }
        }

        /// Takes a toast off the screen and out of the Action Center.
        fn take_away(&self, entry: &Entry) {
            // Hiding fails for a toast that is no longer on the screen, which is no matter.
            let _ = self.notifier.Hide(&entry.toast);
            if let Err(err) = self.history.RemoveGroupedTagWithId(&entry.tag, &self.group, &self.app) {
                tracing::warn!("closing a notification: {err}");
            }
        }

        /// Writes a notification's picture for a toast to show, named after what it shows, as
        /// Windows may show a picture it read before under the same name.
        fn keep_picture(&self, picture: &Pixels) -> Option<PathBuf> {
            let folder = self.pictures.as_ref()?;
            let png = super::png(picture)?;
            let path = folder.join(format!("{}.png", blake3::hash(&png).to_hex()));
            let written = fs::create_dir_all(folder).and_then(|()| if path.exists() { Ok(()) } else { fs::write(&path, &png) });
            match written {
                Ok(()) => Some(path),
                Err(err) => {
                    tracing::warn!("keeping a notification's picture: {err}");
                    None
                }
            }
        }

        /// Deletes a picture no toast shows any longer.
        fn forget_picture(&self, picture: Option<PathBuf>) {
            if let Some(picture) = picture
                && !self.shown.values().any(|entry| entry.picture.as_ref() == Some(&picture))
            {
                let _ = fs::remove_file(picture);
            }
        }
    }
}

/// On macOS they show through Tauri, which hears no clicks.
#[cfg(not(any(target_os = "linux", windows)))]
mod shade {
    use tauri::AppHandle;
    use tauri_plugin_notification::NotificationExt;
    use tokio::sync::mpsc::UnboundedSender;

    use super::{Answer, Order, Phone};

    pub struct Shade {
        app: AppHandle,
    }

    impl Shade {
        pub fn start(phone: &Phone, _answers: UnboundedSender<Answer>) -> Self {
            Self { app: phone.app.clone() }
        }

        pub fn order(&self, order: Order) {
            match order {
                Order::Show(note) if note.shown.alert && !note.shown.quiet => {
                    let shown = note.shown;
                    let title = if shown.title.is_empty() { shown.label } else { shown.title };
                    if let Err(err) = self.app.notification().builder().title(title).body(shown.text).show() {
                        tracing::warn!("showing a notification: {err}");
                    }
                }
                Order::Stop(done) => {
                    let _ = done.send(());
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_picture_as_rows_of_pixels() {
        let mut png = Vec::new();
        let mut encoder = png::Encoder::new(&mut png, 2, 3);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(&[200; 2 * 3 * 3]).unwrap();
        let picture = pixels(&png).unwrap();
        assert_eq!((picture.width, picture.height, picture.row), (2, 3, 8));
        assert_eq!(&picture.data[..4], &[200, 200, 200, 255]);
        assert_eq!(picture.data.len(), 2 * 3 * 4);
        assert!(pixels(b"not a picture").is_none());
    }

    #[test]
    fn writes_a_picture_back_as_png() {
        let picture = Pixels { width: 2, height: 1, row: 8, data: vec![1, 2, 3, 4, 5, 6, 7, 8] };
        let again = pixels(&png(&picture).unwrap()).unwrap();
        assert_eq!((again.width, again.height, again.row, &again.data), (2, 1, 8, &picture.data));
        assert!(png(&Pixels { row: 9, ..picture }).is_none());
    }

    #[test]
    fn a_toast_binds_its_words_and_escapes_the_rest() {
        let logo = std::path::Path::new(r"C:\Users\Ada & Bo\cache\a.png");
        let xml = toast(Some((logo, true)), "Mail <work> · Ada's phone", false);
        assert!(xml.starts_with(r#"<toast><visual><binding template="ToastGeneric">"#));
        assert!(xml.contains(r#"<image placement="appLogoOverride" hint-crop="circle" src="file:///C:\Users\Ada &amp; Bo\cache\a.png"/>"#));
        assert!(xml.contains("<text>{title}</text><text>{text}</text>"));
        assert!(xml.contains(r#"<text placement="attribution">Mail &lt;work&gt; · Ada's phone</text>"#));
        assert!(!xml.contains("<audio"));
        let xml = toast(Some((logo, false)), "Mail", true);
        assert!(xml.contains(r#"<image placement="appLogoOverride" src="#));
        assert!(xml.ends_with(r#"</binding></visual><audio silent="true"/></toast>"#));
        assert!(!toast(None, "Mail", false).contains("<image"));
    }

    fn shown(key: &str, text: &str) -> Shown {
        Shown {
            key: key.into(),
            app: "com.example".into(),
            label: "Example".into(),
            title: "Ada".into(),
            text: text.into(),
            alert: true,
            quiet: false,
            picture: false,
            conversation: None,
        }
    }

    #[test]
    fn a_notification_posted_again_unchanged_shows_once() {
        let (mut memory, now) = (Memory::default(), Instant::now());
        assert_eq!(memory.heed(&shown("a", "Hi"), now), Heed::Show);
        assert_eq!(memory.heed(&shown("a", "Hi"), now), Heed::Skip);
        assert_eq!(memory.heed(&shown("a", "Hi"), now + BURST * 6), Heed::Skip);
        assert_eq!(memory.heed(&shown("a", "Hi\nAre you there?"), now), Heed::Show);
        assert_eq!(memory.heed(&shown("a", "Hi\nAre you there?"), now), Heed::Skip);
        let pictured = Shown { picture: true, ..shown("a", "Hi\nAre you there?") };
        assert_eq!(memory.heed(&pictured, now), Heed::Show);
    }

    #[test]
    fn one_removed_from_the_phone_shows_again_when_posted_again() {
        let (mut memory, now) = (Memory::default(), Instant::now());
        assert_eq!(memory.heed(&shown("a", "Hi"), now), Heed::Show);
        memory.removed("a");
        assert_eq!(memory.heed(&shown("a", "Hi"), now + BURST), Heed::Show);
    }

    #[test]
    fn a_burst_pops_up_only_the_first_few() {
        let (mut memory, now) = (Memory::default(), Instant::now());
        let heeded: Vec<Heed> = (0..6).map(|n| memory.heed(&shown(&n.to_string(), "Hi"), now)).collect();
        assert_eq!(heeded, [Heed::Show, Heed::Show, Heed::Show, Heed::Hush, Heed::Hush, Heed::Hush]);
        // One that popped up still shows its changes, without taking more room.
        assert_eq!(memory.heed(&shown("0", "Hi again"), now), Heed::Show);
        assert_eq!(memory.heed(&shown("6", "Hi"), now + BURST / 2), Heed::Hush);
        assert_eq!(memory.heed(&shown("7", "Hi"), now + BURST), Heed::Show);
    }

    #[test]
    fn quiet_ones_and_updates_that_ask_for_no_attention_take_no_room() {
        let (mut memory, now) = (Memory::default(), Instant::now());
        for n in 0..5 {
            assert_eq!(memory.heed(&Shown { quiet: true, ..shown(&format!("quiet {n}"), "Hi") }, now), Heed::Show);
            assert_eq!(memory.heed(&Shown { alert: false, ..shown(&format!("update {n}"), "Hi") }, now), Heed::Show);
        }
        assert_eq!(memory.heed(&shown("loud", "Hi"), now), Heed::Show);
    }

    #[test]
    fn what_the_phone_has_as_it_reconnects_does_not_pop_up_again() {
        let (mut memory, now) = (Memory::default(), Instant::now());
        memory.heed(&shown("shown", "Hi"), now);
        memory.heed(&shown("gone", "Hi"), now);
        let listed = [shown("shown", "Hi"), shown("came meanwhile", "Hello")];
        let keys = listed.iter().map(|shown| shown.key.clone()).chain(["odd".into()]).collect();
        memory.connected(&keys, &listed);
        let later = now + BURST;
        assert_eq!(memory.heed(&shown("shown", "Hi"), later), Heed::Skip);
        assert_eq!(memory.heed(&shown("came meanwhile", "Hello"), later), Heed::Skip);
        assert_eq!(memory.heed(&shown("came meanwhile", "Hello\nAnyone?"), later), Heed::Show);
        assert_eq!(memory.heed(&shown("gone", "Hi"), later), Heed::Show);
    }
}
