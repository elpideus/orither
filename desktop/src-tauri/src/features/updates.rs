//! Keeps Orither up to date from its releases on GitHub.
//!
//! A check runs shortly after launch and every hour after that. A newer release is downloaded
//! in the background, unless the person turned that off, and checked against the public key in
//! `tauri.conf.json` by the updater plugin. It then waits in the cache folder until it is
//! installed: when the person clicks Restart, or by itself the next time Orither starts with the
//! computer, before anything is open. Installing replaces the AppImage, the package or the
//! Windows installation, and starts Orither again.
//!
//! The UI reads [`Status`] with `call("updates", "status")` and hears every change as the
//! notice `updates.status`.

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{Manager, async_runtime, utils::config::BundleType};
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_updater::{Error as UpdateError, Update, UpdaterExt};

use super::{Feature, Phone};
use crate::{link::Link, preferences};

const ID: &str = "updates";
/// Where the choice to download updates by themselves is remembered.
const AUTO: &str = "auto-update";
/// The first check waits for the network, which may still be coming up at login.
const FIRST_CHECK: Duration = Duration::from_secs(15);
const CHECK_EVERY: Duration = Duration::from_secs(60 * 60);
/// Download progress reaches the UI at most this often.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);
/// Starts at login that may install a downloaded update by themselves. After that many, the
/// update waits for a click, so one that keeps failing cannot keep Orither from starting.
const LAUNCH_ATTEMPTS: u32 = 2;
/// Where people download Orither themselves.
pub const RELEASES: &str = "https://github.com/elpideus/orither/releases";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Phase {
    /// Not checked yet since launch.
    #[default]
    Idle,
    Checking,
    /// This is the newest version.
    Current,
    /// A newer version was found and is not downloaded.
    Available,
    Downloading,
    /// A verified download waits to be installed.
    Ready,
    /// The update is being put in place, and Orither starts again in a moment.
    Installing,
    Failed,
}

#[derive(Clone, Debug, Serialize)]
struct Release {
    version: String,
    notes: Option<String>,
    /// When it was published, in milliseconds since 1970.
    date: Option<i64>,
    /// Its page on GitHub.
    page: String,
}

#[derive(Clone, Copy, Debug, Serialize)]
struct Progress {
    done: u64,
    total: Option<u64>,
}

/// Everything the UI shows about updates.
#[derive(Clone, Debug, Serialize)]
struct Status {
    /// The version running, as the updater compares it.
    current: String,
    phase: Phase,
    /// The newer version, once one is found.
    release: Option<Release>,
    progress: Option<Progress>,
    /// When the last check finished, in milliseconds since 1970.
    checked: Option<u64>,
    /// What went wrong, in phase `failed`, or when installing a ready update did not work.
    error: Option<String>,
    /// Whether new versions download by themselves.
    auto: bool,
    /// Why this copy cannot install updates by itself. It still checks, and links to the release.
    manual: Option<String>,
    /// Installing asks for the computer's password, as with a system package.
    asks_password: bool,
}

struct State {
    status: Status,
    /// The newer release, as the updater found it.
    update: Option<Update>,
    /// A check or a download is under way.
    busy: bool,
    /// The version the tray offers to update to.
    offered: Option<String>,
    /// The ready update was downloaded by an earlier run, not this one.
    from_earlier_run: bool,
}

/// How many times a start at login tried to install a version.
#[derive(Default, Serialize, Deserialize)]
struct Attempts {
    version: String,
    count: u32,
}

pub struct Updates {
    phone: Phone,
    state: Mutex<State>,
}

impl Updates {
    pub fn start(phone: Phone) -> Arc<Self> {
        let auto = preferences::read::<bool>(&phone.app, AUTO).unwrap_or(true);
        let status = Status {
            current: phone.app.package_info().version.to_string(),
            phase: Phase::Idle,
            release: None,
            progress: None,
            checked: None,
            error: None,
            auto,
            manual: manual_reason(),
            asks_password: matches!(bundle_type(), Some(BundleType::Deb | BundleType::Rpm)),
        };
        let state = State { status, update: None, busy: false, offered: None, from_earlier_run: false };
        let updates = Arc::new(Self { phone, state: Mutex::new(state) });
        async_runtime::spawn(updates.clone().watch());
        updates
    }

    async fn watch(self: Arc<Self>) {
        tokio::time::sleep(FIRST_CHECK).await;
        self.check().await;
        if std::env::args().any(|arg| arg == crate::BACKGROUND) {
            self.install_at_login().await;
        }
        loop {
            tokio::time::sleep(CHECK_EVERY).await;
            self.check().await;
        }
    }

    fn status(&self) -> Status {
        self.state.lock().unwrap().status.clone()
    }

    fn publish(&self) {
        let (status, offer) = {
            let mut state = self.state.lock().unwrap();
            let ready = (state.status.phase == Phase::Ready).then(|| state.status.release.as_ref().map(|r| r.version.clone()));
            let ready = ready.flatten();
            let offer = (state.offered != ready).then(|| ready.clone());
            state.offered = ready;
            (state.status.clone(), offer)
        };
        self.phone.tell_ui(ID, "status", json!(status));
        if let Some(version) = offer {
            crate::tray::offer_update(&self.phone.app, version.as_deref());
        }
    }

    fn change(&self, change: impl FnOnce(&mut State)) {
        change(&mut self.state.lock().unwrap());
        self.publish();
    }

    /// Asks GitHub for the newest release, and downloads it when that is the choice.
    async fn check(self: &Arc<Self>) {
        let proceed = {
            let mut state = self.state.lock().unwrap();
            if state.busy || state.status.phase == Phase::Installing {
                false
            } else {
                state.busy = true;
                if state.status.phase != Phase::Ready {
                    state.status.phase = Phase::Checking;
                    state.status.error = None;
                }
                true
            }
        };
        if !proceed {
            return;
        }
        self.publish();

        let found = self.find().await;
        let checked = Some(now_millis());
        let update = match found {
            Err(err) => {
                tracing::warn!("checking for updates: {err}");
                return self.change(|state| {
                    state.busy = false;
                    state.status.checked = checked;
                    if state.status.phase != Phase::Ready {
                        state.status.phase = Phase::Failed;
                        state.status.error = Some(words(&err));
                    }
                });
            }
            Ok(None) => {
                self.forget_downloads(None).await;
                return self.change(|state| {
                    state.busy = false;
                    state.update = None;
                    state.status.checked = checked;
                    state.status.release = None;
                    state.status.phase = Phase::Current;
                });
            }
            Ok(Some(update)) => update,
        };

        let already_ready = {
            let state = self.state.lock().unwrap();
            state.status.phase == Phase::Ready
                && state.status.release.as_ref().is_some_and(|release| release.version == update.version)
        };
        if already_ready {
            return self.change(|state| {
                state.busy = false;
                state.status.checked = checked;
            });
        }

        // A download from an earlier run, still good, needs no second download.
        let downloaded = self.downloaded(&update).await;
        let download = !downloaded && self.state.lock().unwrap().status.auto && manual_reason().is_none();
        self.change(|state| {
            state.busy = download;
            state.update = Some(update.clone());
            state.status.checked = checked;
            state.status.release = Some(release_of(&update));
            state.status.phase = if downloaded { Phase::Ready } else { Phase::Available };
            state.from_earlier_run = downloaded;
        });
        if download {
            self.download(update).await;
        }
    }

    async fn find(&self) -> Result<Option<Update>, UpdateError> {
        let app = self.phone.app.clone();
        // Windows ends Orither as soon as the installer starts, so the phone is told first.
        let updater = self
            .phone
            .app
            .updater_builder()
            .on_before_exit(move || async_runtime::block_on(app.state::<Link>().shutdown()))
            .build()?;
        updater.check().await
    }

    /// Downloads `update` into the cache folder. The updater checks its signature.
    async fn download(self: &Arc<Self>, update: Update) {
        self.change(|state| {
            state.busy = true;
            state.status.phase = Phase::Downloading;
            state.status.progress = Some(Progress { done: 0, total: None });
            state.status.error = None;
        });
        let (mut done, mut shown) = (0u64, Instant::now());
        let progress = |chunk: usize, total: Option<u64>| {
            done += chunk as u64;
            if shown.elapsed() >= PROGRESS_EVERY {
                shown = Instant::now();
                self.change(|state| state.status.progress = Some(Progress { done, total }));
            }
        };
        let saved = match update.download(progress, || {}).await {
            Ok(bytes) => self.keep(&update.version, &bytes).await,
            Err(err) => Err(anyhow::anyhow!(words(&err))),
        };
        if let Err(err) = &saved {
            tracing::warn!("downloading {}: {err:#}", update.version);
        }
        self.change(|state| {
            state.busy = false;
            state.status.progress = None;
            match saved {
                Ok(()) => state.status.phase = Phase::Ready,
                Err(err) => {
                    state.status.phase = Phase::Failed;
                    state.status.error = Some(format!("{err:#}"));
                }
            }
        });
    }

    /// Puts the ready update in place and starts Orither again.
    async fn install(self: &Arc<Self>) -> Result<()> {
        if let Some(reason) = manual_reason() {
            bail!(reason);
        }
        let update = {
            let mut state = self.state.lock().unwrap();
            if state.status.phase != Phase::Ready {
                bail!("no update is ready to install");
            }
            state.status.phase = Phase::Installing;
            state.status.error = None;
            state.update.clone().context("no update is ready to install")?
        };
        self.publish();

        let installed = async {
            let bytes = tokio::fs::read(self.file_for(&update.version)?).await.context("the download is gone")?;
            let version = update.version.clone();
            // Blocking: a package asks for the password, and Windows ends Orither from here.
            async_runtime::spawn_blocking(move || update.install(bytes)).await?.map_err(|err| anyhow::anyhow!(words(&err)))?;
            anyhow::Ok(version)
        }
        .await;
        match installed {
            Ok(version) => {
                tracing::info!("installed {version}, starting again");
                self.phone.app.restart();
            }
            Err(err) => {
                tracing::warn!("installing the update: {err:#}");
                self.change(|state| {
                    state.status.phase = Phase::Ready;
                    state.status.error = Some(format!("{err:#}"));
                });
                Err(err)
            }
        }
    }

    /// At login, before the window has opened, an update that finished downloading during an
    /// earlier run is installed right away, the way Discord does. Only where installing needs
    /// no password. One downloaded during this run waits: by the time it is done, the phone may
    /// be in use.
    async fn install_at_login(self: &Arc<Self>) {
        let ready = {
            let state = self.state.lock().unwrap();
            let version = state.status.release.as_ref().map(|release| release.version.clone());
            let quiet = state.status.auto && !state.status.asks_password && state.from_earlier_run;
            (state.status.phase == Phase::Ready && quiet).then_some(version).flatten()
        };
        let Some(version) = ready else { return };
        let shown = self.phone.app.get_webview_window("main").is_some_and(|window| window.is_visible().unwrap_or(false));
        if shown {
            return;
        }
        let mut attempts = self.attempts();
        if attempts.version != version {
            attempts = Attempts { version, count: 0 };
        }
        if attempts.count >= LAUNCH_ATTEMPTS {
            return tracing::info!("{} is ready, and waits for a click after {} tries", attempts.version, attempts.count);
        }
        // Counted first, so an attempt that never comes back still counts.
        attempts.count += 1;
        self.save_attempts(&attempts);
        tracing::info!("installing {} at login", attempts.version);
        let _ = self.install().await;
    }

    fn folder(&self) -> Result<PathBuf> {
        Ok(self.phone.app.path().app_cache_dir()?.join("updates"))
    }

    fn file_for(&self, version: &str) -> Result<PathBuf> {
        Ok(self.folder()?.join(format!("orither-{version}.update")))
    }

    /// Whether `update` was downloaded before and still passes its signature check.
    async fn downloaded(&self, update: &Update) -> bool {
        let Ok(path) = self.file_for(&update.version) else { return false };
        let Ok(bytes) = tokio::fs::read(&path).await else { return false };
        match update.verify(&bytes) {
            Ok(()) => true,
            Err(err) => {
                tracing::warn!("dropped the earlier download of {}: {err}", update.version);
                let _ = tokio::fs::remove_file(&path).await;
                false
            }
        }
    }

    async fn keep(&self, version: &str, bytes: &[u8]) -> Result<()> {
        let path = self.file_for(version)?;
        let partial = path.with_extension("part");
        tokio::fs::create_dir_all(self.folder()?).await?;
        tokio::fs::write(&partial, bytes).await.context("could not save the update")?;
        tokio::fs::rename(&partial, &path).await?;
        self.forget_downloads(Some(&path)).await;
        Ok(())
    }

    /// Removes old downloads, all but `keep`.
    async fn forget_downloads(&self, keep: Option<&PathBuf>) {
        let Ok(folder) = self.folder() else { return };
        let Ok(mut entries) = tokio::fs::read_dir(&folder).await else { return };
        while let Ok(Some(entry)) = entries.next_entry().await {
            if Some(&entry.path()) != keep {
                let _ = tokio::fs::remove_file(entry.path()).await;
            }
        }
    }

    fn attempts(&self) -> Attempts {
        preferences::read(&self.phone.app, "update-attempts").unwrap_or_default()
    }

    fn save_attempts(&self, attempts: &Attempts) {
        preferences::write(&self.phone.app, "update-attempts", attempts);
    }

    fn set_auto(self: &Arc<Self>, on: bool) {
        preferences::write(&self.phone.app, AUTO, &on);
        let download = {
            let mut state = self.state.lock().unwrap();
            state.status.auto = on;
            let start = on && !state.busy && state.status.phase == Phase::Available && manual_reason().is_none();
            if start {
                state.busy = true;
            }
            start.then(|| state.update.clone()).flatten()
        };
        self.publish();
        if let Some(update) = download {
            let this = self.clone();
            async_runtime::spawn(async move { this.download(update).await });
        }
    }

    /// Opens the newer release's page, or the list of releases.
    fn open_page(&self) -> Result<()> {
        let page = self.state.lock().unwrap().status.release.as_ref().map_or(RELEASES.to_owned(), |release| release.page.clone());
        self.phone.app.opener().open_url(page, None::<&str>)?;
        Ok(())
    }

    fn start_download(self: &Arc<Self>) -> Result<()> {
        if let Some(reason) = manual_reason() {
            bail!(reason);
        }
        let update = {
            let mut state = self.state.lock().unwrap();
            if state.busy {
                bail!("already busy with an update");
            }
            let update = state.update.clone().context("no update to download")?;
            state.busy = true;
            update
        };
        let this = self.clone();
        async_runtime::spawn(async move { this.download(update).await });
        Ok(())
    }
}

/// The `Feature` is implemented on a handle, because the work it starts outlives each call.
pub struct Handle(pub Arc<Updates>);

#[async_trait]
impl Feature for Handle {
    fn id(&self) -> &'static str {
        ID
    }

    async fn command(&self, method: &str, body: &Value) -> Option<Result<Value>> {
        let updates = &self.0;
        let result = match method {
            "status" => Ok(()),
            "check" => {
                updates.check().await;
                Ok(())
            }
            "download" => updates.start_download(),
            "open" => updates.open_page(),
            "install" => {
                let updates = updates.clone();
                async_runtime::spawn(async move { updates.install().await });
                Ok(())
            }
            "auto" => match body.get("on").and_then(Value::as_bool) {
                Some(on) => {
                    updates.set_auto(on);
                    Ok(())
                }
                None => Err(anyhow::anyhow!("auto needs on: true or false")),
            },
            _ => return None,
        };
        Some(result.map(|()| json!(updates.status())))
    }
}

fn bundle_type() -> Option<BundleType> {
    tauri::utils::platform::bundle_type()
}

/// Why this copy cannot install updates by itself, if it cannot.
fn manual_reason() -> Option<String> {
    if cfg!(debug_assertions) {
        return Some("This is a development build, which does not update itself.".into());
    }
    match bundle_type() {
        Some(BundleType::AppImage | BundleType::Deb | BundleType::Rpm | BundleType::Nsis) => None,
        // Any Windows copy can run the installer, which installs Orither properly.
        _ if cfg!(windows) => None,
        _ => Some("This copy was not installed from a release, so it does not update itself.".into()),
    }
}

fn release_of(update: &Update) -> Release {
    Release {
        version: update.version.clone(),
        notes: update.body.clone().filter(|notes| !notes.trim().is_empty()),
        date: update.date.map(|date| date.unix_timestamp() * 1000),
        page: format!("{RELEASES}/tag/v{}", update.version),
    }
}

/// Words a person can act on, for what went wrong.
fn words(err: &UpdateError) -> String {
    match err {
        UpdateError::Reqwest(_) | UpdateError::Network(_) => "Could not reach GitHub. Check the internet connection.".into(),
        UpdateError::ReleaseNotFound => "GitHub has no release information right now. Try again later.".into(),
        UpdateError::TargetNotFound(_) | UpdateError::TargetsNotFound(_) => {
            "The newest release has no download for this kind of installation yet.".into()
        }
        UpdateError::Minisign(_) | UpdateError::SignedVersionMismatch { .. } | UpdateError::MissingSignedVersion => {
            "The download failed its signature check, so it was not used.".into()
        }
        UpdateError::AuthenticationFailed => "The password was not given, so nothing changed.".into(),
        other => other.to_string(),
    }
}

fn now_millis() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_millis() as u64)
}
