//! Moves files between the phone and this computer, and shows the UI how far
//! along they are. Other features send their files through here too.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use tauri::Manager;
use tauri_plugin_opener::OpenerExt;
use tokio::{
    fs::{self, File},
    io::{self, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
};

use super::{Feature, Phone};

const REPORT_EVERY: Duration = Duration::from_millis(100);
const CHUNK: usize = 64 * 1024;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// A file on the phone, as any feature reads one.
#[derive(Deserialize)]
pub struct Source {
    pub feature: String,
    pub method: String,
    pub body: Value,
    pub name: String,
    pub size: Option<u64>,
}

#[derive(Deserialize)]
struct Upload {
    local: PathBuf,
    feature: String,
    method: String,
    folder: String,
}

/// Tells the UI about a transfer, often enough to look alive.
struct Progress<'a> {
    phone: &'a Phone,
    id: u64,
    name: &'a str,
    total: Option<u64>,
    done: u64,
    told: Instant,
}

impl<'a> Progress<'a> {
    fn new(phone: &'a Phone, name: &'a str, total: Option<u64>) -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        Self { phone, id, name, total, done: 0, told: Instant::now() }
    }

    fn tell(&self, finished: bool) {
        let body = json!({
            "id": self.id,
            "name": self.name,
            "done": self.done,
            "total": self.total,
            "finished": finished,
        });
        self.phone.tell_ui("transfer", "progress", body);
    }

    fn advance(&mut self, by: usize) {
        self.done += by as u64;
        if self.told.elapsed() >= REPORT_EVERY {
            self.told = Instant::now();
            self.tell(false);
        }
    }
}

impl Drop for Progress<'_> {
    fn drop(&mut self) {
        self.tell(true);
    }
}

async fn pump(
    from: &mut (impl AsyncRead + Unpin),
    to: &mut (impl AsyncWrite + Unpin),
    progress: &mut Progress<'_>,
) -> Result<()> {
    let mut buffer = vec![0; CHUNK];
    loop {
        let read = from.read(&mut buffer).await?;
        if read == 0 {
            return Ok(to.flush().await?);
        }
        to.write_all(&buffer[..read]).await?;
        progress.advance(read);
    }
}

/// The name of a file from the phone, without any folders it may claim.
pub fn file_name(name: &str) -> &str {
    Path::new(name).file_name().and_then(|name| name.to_str()).unwrap_or("file")
}

/// Saves a file from the phone as `target`, which appears only once it is whole.
pub async fn fetch(phone: &Phone, source: &Source, target: &Path) -> Result<()> {
    let session = phone.session()?;
    let mut stream = session.open(&source.feature, &source.method, source.body.clone()).await?;
    let partial = target.with_file_name(format!("{}.part", file_name(&source.name)));
    let mut progress = Progress::new(phone, &source.name, source.size);

    let mut file = File::create(&partial).await.context("creating the file")?;
    let received = pump(&mut stream.recv, &mut file, &mut progress).await;
    let whole = progress.done > 0 || source.size == Some(0);
    let done = received.and_then(|()| {
        ensure!(whole, "your phone could not read {}", source.name);
        Ok(())
    });
    if let Err(err) = done {
        let _ = fs::remove_file(&partial).await;
        return Err(err);
    }
    fs::rename(&partial, target).await.context("saving the file")
}

/// Sends a local file to the phone and waits until the phone has all of it.
pub async fn send(phone: &Phone, feature: &str, method: &str, body: Value, local: &Path) -> Result<()> {
    let name = local.file_name().and_then(|name| name.to_str()).context("a file needs a name")?;
    let mut file = File::open(local).await.with_context(|| format!("opening {name}"))?;
    let size = file.metadata().await?.len();

    let mut stream = phone.session()?.open(feature, method, body).await?;
    let mut progress = Progress::new(phone, name, Some(size));
    pump(&mut file, &mut stream.send, &mut progress).await?;
    stream.send.finish()?;
    io::copy(&mut stream.recv, &mut io::sink()).await?;
    Ok(())
}

/// A name in `folder` that nothing has yet, in the way file managers pick one.
async fn free_path(folder: &Path, name: &str) -> PathBuf {
    let plain = Path::new(name);
    let stem = plain.file_stem().and_then(|stem| stem.to_str()).unwrap_or(name);
    let extension = plain.extension().and_then(|extension| extension.to_str());
    let mut candidate = folder.join(name);
    let mut number = 1;
    while fs::try_exists(&candidate).await.unwrap_or(false) {
        candidate = folder.join(match extension {
            Some(extension) => format!("{stem} ({number}).{extension}"),
            None => format!("{stem} ({number})"),
        });
        number += 1;
    }
    candidate
}

pub struct Transfer {
    phone: Phone,
}

impl Transfer {
    pub fn new(phone: Phone) -> Arc<dyn Feature> {
        Arc::new(Self { phone })
    }

    async fn download(&self, source: &Source) -> Result<Value> {
        let paths = self.phone.app.path();
        // Systems without a configured downloads folder still have this usual one.
        let folder = paths.download_dir().or_else(|_| Ok::<_, tauri::Error>(paths.home_dir()?.join("Downloads")))?;
        fs::create_dir_all(&folder).await?;
        let target = free_path(&folder, file_name(&source.name)).await;
        fetch(&self.phone, source, &target).await?;
        Ok(json!(target))
    }

    /// Fetches a file into the cache and opens it, leaving nothing in Downloads.
    async fn open(&self, source: &Source) -> Result<Value> {
        let folder = self.phone.app.path().app_cache_dir()?.join("opened");
        fs::create_dir_all(&folder).await?;
        let target = folder.join(file_name(&source.name));
        fetch(&self.phone, source, &target).await?;
        self.phone.app.opener().open_path(target.to_string_lossy(), None::<&str>)?;
        Ok(Value::Null)
    }

    async fn upload(&self, upload: Upload) -> Result<Value> {
        let name = upload.local.file_name().and_then(|name| name.to_str()).context("a file needs a name")?;
        let body = json!({ "folder": upload.folder, "name": name });
        send(&self.phone, &upload.feature, &upload.method, body, &upload.local).await?;
        Ok(Value::Null)
    }
}

#[async_trait::async_trait]
impl Feature for Transfer {
    fn id(&self) -> &'static str {
        "transfer"
    }

    async fn command(&self, method: &str, body: &Value) -> Option<Result<Value>> {
        let source = || serde_json::from_value::<Source>(body.clone());
        let outcome = match method {
            "download" => self.download(&source().ok()?).await,
            "open" => self.open(&source().ok()?).await,
            "upload" => self.upload(serde_json::from_value(body.clone()).ok()?).await,
            _ => return None,
        };
        if let Err(err) = &outcome {
            self.phone.tell_ui("transfer", "failed", json!({ "error": format!("{err:#}") }));
        }
        Some(outcome)
    }
}
