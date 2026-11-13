//! One clipboard across both devices. A thread owns the computer's clipboard
//! and looks at it every moment, because few desktops can announce a change.

use std::{
    collections::VecDeque,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow};
use orither_link::{PeerId, Session};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{Manager, async_runtime};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use super::{Feature, Phone, transfer};
use crate::preferences;

const LOOK_EVERY: Duration = Duration::from_millis(500);
/// Pictures are slow to read, so they get every fourth look.
const LOOKS_PER_PICTURE: u32 = 4;
const HISTORY: usize = 50;
/// Android cannot hold much more than this on its clipboard.
const MAX_TEXT: usize = 512 * 1024;
const MAX_FILES: u64 = 256 * 1024 * 1024;
const ENABLED: &str = "clipboard-enabled";

#[derive(Clone, Debug, PartialEq, Eq)]
enum Content {
    Nothing,
    Text(String),
    Files(Vec<PathBuf>),
    /// Stands for a picture that was saved as the file in [`Content::Files`].
    Picture(blake3::Hash),
}

enum Write {
    Text(String),
    Files(Vec<PathBuf>),
}

#[derive(Clone, Serialize)]
struct Clip {
    id: u64,
    text: Option<String>,
    files: Vec<String>,
    from: &'static str,
    time: u64,
}

#[derive(Deserialize)]
struct Change {
    text: Option<String>,
    #[serde(default)]
    files: Vec<RemoteFile>,
}

#[derive(Deserialize)]
struct RemoteFile {
    uri: String,
    name: String,
    size: Option<u64>,
}

/// Saves a picture from the clipboard, since the phone takes files.
fn save_picture(picture: &arboard::ImageData<'_>, folder: &Path) -> Result<PathBuf> {
    fs::create_dir_all(folder)?;
    let path = folder.join("Copied picture.png");
    let file = fs::File::create(&path)?;
    let mut encoder = png::Encoder::new(file, picture.width as u32, picture.height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&picture.bytes)?;
    Ok(path)
}

struct System {
    clipboard: arboard::Clipboard,
    pictures: PathBuf,
    looks: u32,
}

impl System {
    fn apply(&mut self, write: Write) -> Result<(), arboard::Error> {
        match write {
            Write::Text(text) => self.clipboard.set_text(text),
            Write::Files(files) => self.clipboard.set().file_list(&files),
        }
    }

    fn read(&mut self) -> Content {
        if let Ok(files) = self.clipboard.get().file_list()
            && !files.is_empty()
        {
            return Content::Files(files);
        }
        if let Ok(text) = self.clipboard.get_text()
            && !text.is_empty()
        {
            return Content::Text(text);
        }
        self.looks = (self.looks + 1) % LOOKS_PER_PICTURE;
        if self.looks != 0 {
            return Content::Nothing;
        }
        self.clipboard.get_image().map_or(Content::Nothing, |picture| Content::Picture(blake3::hash(&picture.bytes)))
    }

    /// Follows the clipboard, reporting what the user copies but not what the
    /// phone puts there, and not what was already there when this started.
    fn watch(mut self, writes: Receiver<Write>, changes: UnboundedSender<Content>) {
        let mut seen = self.read();
        loop {
            match writes.recv_timeout(LOOK_EVERY) {
                Ok(write) => {
                    if let Err(err) = self.apply(write) {
                        tracing::warn!("writing the clipboard: {err}");
                    }
                    seen = self.read();
                }
                Err(RecvTimeoutError::Timeout) => {
                    let now = self.read();
                    if now == Content::Nothing || now == seen {
                        continue;
                    }
                    seen = now.clone();
                    let change = match now {
                        Content::Picture(_) => self.picture_file(),
                        change => Some(change),
                    };
                    if let Some(change) = change {
                        let _ = changes.send(change);
                    }
                }
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    fn picture_file(&mut self) -> Option<Content> {
        let picture = self.clipboard.get_image().ok()?;
        match save_picture(&picture, &self.pictures) {
            Ok(path) => Some(Content::Files(vec![path])),
            Err(err) => {
                tracing::warn!("saving a copied picture: {err:#}");
                None
            }
        }
    }
}

struct Shared {
    phone: Phone,
    system: Option<Sender<Write>>,
    enabled: AtomicBool,
    history: Mutex<VecDeque<Clip>>,
    next: AtomicU64,
}

impl Shared {
    fn write(&self, write: Write) -> Result<()> {
        let system = self.system.as_ref().ok_or_else(|| anyhow!("this computer's clipboard is out of reach"))?;
        system.send(write).map_err(|_| anyhow!("this computer's clipboard stopped"))
    }

    fn folder(&self, name: &str) -> Result<PathBuf> {
        Ok(self.phone.app.path().app_cache_dir()?.join("clipboard").join(name))
    }

    fn remember(&self, text: Option<String>, files: Vec<String>, from: &'static str) {
        let time = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_millis() as u64);
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let mut history = self.history.lock().unwrap();
        history.push_front(Clip { id, text, files, from, time });
        history.truncate(HISTORY);
        drop(history);
        self.phone.tell_ui("clipboard", "changed", Value::Null);
    }

    /// Passes on what the user copied here.
    async fn copied(&self, content: Content) -> Result<()> {
        if !self.enabled.load(Ordering::Relaxed) {
            return Ok(());
        }
        match content {
            Content::Text(text) if text.len() <= MAX_TEXT => {
                self.phone.session()?.notify("clipboard", "set", json!({ "text": text }))?;
                self.remember(Some(text), Vec::new(), "computer");
            }
            Content::Files(files) => self.send_files(&files).await?,
            _ => {}
        }
        Ok(())
    }

    async fn send_files(&self, files: &[PathBuf]) -> Result<()> {
        let sizes = files.iter().map(|file| fs::metadata(file).map(|file| (file.is_file(), file.len())));
        let sizes = sizes.collect::<std::io::Result<Vec<_>>>()?;
        let regular: Vec<_> = files.iter().zip(&sizes).filter(|(_, (is_file, _))| *is_file).map(|(file, _)| file).collect();
        let total: u64 = sizes.iter().filter(|(is_file, _)| *is_file).map(|(_, size)| size).sum();
        if regular.is_empty() || total > MAX_FILES {
            return Ok(());
        }

        let batch = self.next.fetch_add(1, Ordering::Relaxed).to_string();
        let mut names = Vec::new();
        for (index, file) in regular.iter().enumerate() {
            let name = file.file_name().and_then(|name| name.to_str()).context("a file needs a name")?;
            let body = json!({ "name": name, "batch": batch, "last": index + 1 == regular.len() });
            transfer::send(&self.phone, "clipboard", "put", body, file).await?;
            names.push(name.to_owned());
        }
        self.remember(None, names, "computer");
        Ok(())
    }

    /// Takes in what the user copied on the phone.
    async fn arrived(&self, change: Change) -> Result<()> {
        if let Some(text) = change.text {
            self.write(Write::Text(text.clone()))?;
            self.remember(Some(text), Vec::new(), "phone");
            return Ok(());
        }
        let total: u64 = change.files.iter().filter_map(|file| file.size).sum();
        if change.files.is_empty() || total > MAX_FILES {
            return Ok(());
        }

        let folder = self.folder(&self.next.fetch_add(1, Ordering::Relaxed).to_string())?;
        fs::create_dir_all(&folder)?;
        let mut paths = Vec::new();
        for file in &change.files {
            let name = transfer::file_name(&file.name);
            let source = transfer::Source {
                feature: "clipboard".into(),
                method: "read".into(),
                body: json!({ "uri": file.uri }),
                name: name.to_owned(),
                size: file.size,
            };
            let path = folder.join(name);
            transfer::fetch(&self.phone, &source, &path).await?;
            paths.push(path);
        }
        self.write(Write::Files(paths))?;
        self.remember(None, change.files.into_iter().map(|file| file.name).collect(), "phone");
        Ok(())
    }
}

async fn forward(shared: Arc<Shared>, mut changes: UnboundedReceiver<Content>) {
    while let Some(content) = changes.recv().await {
        if let Err(err) = shared.copied(content).await {
            tracing::debug!("sharing the clipboard: {err:#}");
        }
    }
}

pub struct Clipboard {
    shared: Arc<Shared>,
}

impl Clipboard {
    pub fn start(phone: Phone) -> Arc<dyn Feature> {
        let (writes, written) = mpsc::channel();
        let (changed, changes) = unbounded_channel();
        let pictures = phone.app.path().app_cache_dir().map(|cache| cache.join("clipboard")).unwrap_or_default();
        let system = match arboard::Clipboard::new() {
            Ok(clipboard) => {
                let system = System { clipboard, pictures, looks: 0 };
                std::thread::spawn(move || system.watch(written, changed));
                Some(writes)
            }
            Err(err) => {
                tracing::warn!("no clipboard: {err}");
                None
            }
        };
        let enabled = preferences::read(&phone.app, ENABLED).unwrap_or(true);
        let shared = Arc::new(Shared {
            phone,
            system,
            enabled: AtomicBool::new(enabled),
            history: Mutex::default(),
            next: AtomicU64::new(1),
        });
        async_runtime::spawn(forward(shared.clone(), changes));
        Arc::new(Self { shared })
    }

    fn enable(&self, on: bool) -> Result<Value> {
        self.shared.enabled.store(on, Ordering::Relaxed);
        preferences::write(&self.shared.phone.app, ENABLED, &on);
        if let Ok(session) = self.shared.phone.session() {
            session.notify("clipboard", "follow", json!({ "on": on }))?;
        }
        Ok(Value::Null)
    }
}

fn text_in(body: &Value) -> Result<String> {
    body["text"].as_str().map(str::to_owned).context("no text")
}

#[async_trait::async_trait]
impl Feature for Clipboard {
    fn id(&self) -> &'static str {
        "clipboard"
    }

    async fn command(&self, method: &str, body: &Value) -> Option<Result<Value>> {
        Some(match method {
            "state" => Ok(json!({
                "enabled": self.shared.enabled.load(Ordering::Relaxed),
                "history": *self.shared.history.lock().unwrap(),
            })),
            "enable" => self.enable(body["on"].as_bool().unwrap_or(true)),
            "copy" => text_in(body).and_then(|text| self.shared.write(Write::Text(text))).map(|()| Value::Null),
            _ => return None,
        })
    }

    fn connected(&self, session: &Session) {
        let on = self.shared.enabled.load(Ordering::Relaxed);
        let _ = session.notify("clipboard", "follow", json!({ "on": on }));
    }

    fn notice(&self, _peer: &PeerId, method: &str, body: &Value) -> bool {
        if method != "changed" {
            return false;
        }
        match serde_json::from_value(body.clone()) {
            Ok(change) if self.shared.enabled.load(Ordering::Relaxed) => {
                let shared = self.shared.clone();
                async_runtime::spawn(async move {
                    if let Err(err) = shared.arrived(change).await {
                        tracing::warn!("taking in the phone's clipboard: {err:#}");
                    }
                });
            }
            Ok(_) => {}
            Err(err) => tracing::warn!("odd clipboard change: {err}"),
        }
        true
    }
}
