//! The phone's notifications, shown as this computer's own. A click opens the
//! app in a window, where the notification leads on the phone, see
//! [`Screen::notification`], except WhatsApp's, which open in the WhatsApp page.
//! Closing one here takes it off the phone too.
// Elsewhere there are no clicks to hear and no pictures to show.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::{
    collections::HashSet,
    io::Cursor,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
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

struct Shared {
    phone: Phone,
    enabled: AtomicBool,
    shade: shade::Shade,
}

impl Shared {
    async fn posted(&self, peer: PeerId, shown: Shown) -> Result<()> {
        if !self.enabled.load(Ordering::Relaxed) {
            return Ok(());
        }
        let session = self.phone.node.session(&peer).context("your phone is not connected")?;
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
    /// meanwhile stay on the phone, as it is late to call attention to them.
    async fn connected(&self) -> Result<()> {
        let listed = self.phone.session()?.call("notifications", "list", Value::Null).await?;
        let listed = listed.as_array().map(Vec::as_slice).unwrap_or_default();
        let keys = listed.iter().filter_map(|shown| shown["key"].as_str()).map(str::to_owned).collect();
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
        let shared = Arc::new(Shared { phone, enabled: AtomicBool::new(enabled), shade });
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

/// Elsewhere they show through Tauri, which hears no clicks.
#[cfg(not(target_os = "linux"))]
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
}
