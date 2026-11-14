//! Sets a phone up over one USB cable: installs the Orither app that matches this computer, links
//! it, starts its helper and turns everything on. The person only turns on USB debugging and plugs
//! the phone in. Nothing is installed from a file on the phone, which is what Android's restricted
//! settings and Play Protect's checks on downloaded apps are about.
//!
//! The UI lists the phones adb sees with `setup.devices` and starts with `setup.start {serial}`.
//! It hears `setup.progress {serial, step}` for each step, then `setup.done {serial, missing}`
//! with what Android kept off, or `setup.failed {serial, error}`.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use orither_link::Session;
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tauri::{Manager, async_runtime};
use tokio::time::{Instant, sleep, timeout};

use super::{Feature, Phone, helper};
use crate::adb;

const ID: &str = "setup";
const PACKAGE: &str = "dev.orither";
const RELEASES: &str = "https://github.com/elpideus/orither/releases/download";
const APK: &str = "Orither-android-arm64.apk";
/// An APK to install instead of the release's, for builds that have no release yet.
const APK_OVERRIDE: &str = "ORITHER_PHONE_APK";
/// After this long, an install is waiting for the person to confirm something on the phone.
const CONFIRM_AFTER: Duration = Duration::from_secs(4);
const INSTALL_WITHIN: Duration = Duration::from_secs(300);
const LINK_WITHIN: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_millis(500);

/// A phone adb sees.
#[derive(Serialize)]
struct Device {
    serial: String,
    model: String,
    /// `ready`, `unauthorized` (waiting for the person to allow this computer) or `offline`.
    state: &'static str,
    /// The version of Orither on it, if any.
    app: Option<String>,
}

pub struct Setup {
    phone: Phone,
    /// Phones being set up now, by serial.
    running: Mutex<HashSet<String>>,
}

impl Setup {
    pub fn new(phone: Phone) -> Arc<Self> {
        Arc::new(Self { phone, running: Mutex::new(HashSet::new()) })
    }

    fn version(&self) -> String {
        self.phone.app.package_info().version.to_string()
    }

    fn tell(&self, method: &str, serial: &str, mut body: Value) {
        body["serial"] = json!(serial);
        self.phone.tell_ui(ID, method, body);
    }

    async fn run(self: Arc<Self>, serial: String) {
        let result = self.set_up(&serial).await;
        self.running.lock().unwrap().remove(&serial);
        match result {
            Ok(missing) => self.tell("done", &serial, json!({ "missing": missing })),
            Err(err) => {
                tracing::warn!("setting up {serial}: {err:#}");
                self.tell("failed", &serial, json!({ "error": format!("{err:#}") }));
            }
        }
    }

    async fn set_up(&self, serial: &str) -> Result<Vec<String>> {
        let device = devices().await?.into_iter().find(|device| device.serial == serial).context("The phone is no longer connected.")?;
        if device.state == "unauthorized" {
            bail!("Allow USB debugging on your phone, with Always allow from this computer ticked.");
        }
        if device.state != "ready" {
            bail!("The phone is connected but not ready. Unplug it and plug it in again.");
        }

        if outdated(device.app.as_deref(), &self.version()) {
            self.tell("progress", serial, json!({ "step": "download" }));
            let apk = self.apk().await?;
            self.tell("progress", serial, json!({ "step": "install" }));
            self.install(serial, &apk).await?;
        }

        self.tell("progress", serial, json!({ "step": "link" }));
        let session = self.link(serial, &device.model).await?;

        self.tell("progress", serial, json!({ "step": "helper" }));
        helper::ensure_engine(&self.phone, &session).await?;

        self.tell("progress", serial, json!({ "step": "allow" }));
        let allowed = session.call("helper", "allow", json!({})).await?;
        Ok(allowed["missing"].as_array().map(|missing| missing.iter().filter_map(|name| name.as_str().map(str::to_owned)).collect()).unwrap_or_default())
    }

    /// The Android app of this computer's version, downloaded once and checked against the release's checksums.
    async fn apk(&self) -> Result<PathBuf> {
        if let Some(path) = std::env::var_os(APK_OVERRIDE) {
            return Ok(path.into());
        }
        let version = self.version();
        let folder = self.phone.app.path().app_cache_dir()?.join("phone-app");
        let path = folder.join(format!("Orither-{version}.apk"));
        if path.is_file() {
            return Ok(path);
        }
        install_crypto();
        let client = reqwest::Client::builder().timeout(Duration::from_secs(600)).build()?;
        let base = format!("{RELEASES}/v{version}");
        let sums = client.get(format!("{base}/SHA256SUMS")).send().await.and_then(|reply| reply.error_for_status());
        let sums = match sums {
            Ok(reply) => reply.text().await?,
            Err(err) if err.status().is_some() => bail!("Orither {version} has no published Android app to install. Install the app on the phone yourself."),
            Err(_) => bail!("Could not download the Android app. Check the internet connection."),
        };
        let wanted = sums.lines().find_map(|line| line.strip_suffix(APK)?.split_whitespace().next().map(str::to_owned)).context("The release lists no Android app.")?;
        let bytes = client.get(format!("{base}/{APK}")).send().await.and_then(|reply| reply.error_for_status()).context("Could not download the Android app")?.bytes().await?;
        if hex(&Sha256::digest(&bytes)) != wanted.to_lowercase() {
            bail!("The downloaded Android app did not match the release's checksum, so it was not installed.");
        }
        tokio::fs::create_dir_all(&folder).await?;
        let partial = path.with_extension("part");
        tokio::fs::write(&partial, &bytes).await?;
        tokio::fs::rename(&partial, &path).await?;
        Ok(path)
    }

    /// Installs (or updates) the app with every permission it asks for already granted.
    async fn install(&self, serial: &str, apk: &Path) -> Result<()> {
        let apk = apk.to_string_lossy().into_owned();
        let args = ["-s", serial, "install", "-r", "-g", &apk];
        let mut install = Box::pin(adb::run(&args));
        let output = match timeout(CONFIRM_AFTER, &mut install).await {
            Ok(output) => output?,
            Err(_) => {
                // Play Protect, or a phone that confirms installs over USB, is asking on the phone.
                self.tell("progress", serial, json!({ "step": "confirm" }));
                timeout(INSTALL_WITHIN, install).await.context("The phone did not finish installing Orither.")??
            }
        };
        let said = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        if said.contains("Success") {
            return Ok(());
        }
        bail!(install_failure(&said))
    }

    /// Links the phone to this computer through a pairing link opened on it, unless it is linked already.
    async fn link(&self, serial: &str, model: &str) -> Result<Session> {
        if let Some(session) = self.linked(model).await {
            return Ok(session);
        }
        let known: HashSet<String> = self.phone.node.peers().into_iter().map(|status| status.peer.id.to_string()).collect();
        let invite = self.phone.node.invite();
        // The link goes through the phone's shell, so it is quoted for it.
        let open = format!("am start -a android.intent.action.VIEW -d '{}' {PACKAGE}", invite.uri.replace('\'', ""));
        let opened = adb::run(&["-s", serial, "shell", &open]).await?;
        if !opened.status.success() {
            bail!("Could not open Orither on the phone.");
        }
        let deadline = Instant::now() + LINK_WITHIN;
        while Instant::now() < deadline {
            let fresh = self.phone.node.peers().into_iter().find(|status| status.online && !known.contains(&status.peer.id.to_string()));
            if let Some(session) = fresh.and_then(|status| self.phone.node.session(&status.peer.id)) {
                return Ok(session);
            }
            sleep(POLL).await;
        }
        self.phone.node.cancel_invite();
        bail!("The phone did not link. Make sure it is online, then try again.")
    }

    /// The connected phone that is this model, if one is linked already.
    async fn linked(&self, model: &str) -> Option<Session> {
        for status in self.phone.node.peers().into_iter().filter(|status| status.online) {
            let Some(session) = self.phone.node.session(&status.peer.id) else { continue };
            let Ok(about) = session.call("helper", "status", json!({})).await else { continue };
            if about["model"].as_str().is_some_and(|theirs| theirs.replace(' ', "_") == model.replace(' ', "_")) {
                return Some(session);
            }
        }
        None
    }
}

/// The `Feature` is implemented on a handle, because a setup outlives the call that starts it.
pub struct Handle(pub Arc<Setup>);

#[async_trait]
impl Feature for Handle {
    fn id(&self) -> &'static str {
        ID
    }

    async fn command(&self, method: &str, body: &Value) -> Option<Result<Value>> {
        Some(match method {
            "devices" => devices().await.map(|devices| json!({ "adb": adb::available(), "devices": devices })),
            "start" => {
                let Some(serial) = body["serial"].as_str().map(str::to_owned) else { return Some(Err(anyhow::anyhow!("start needs a serial"))) };
                if self.0.running.lock().unwrap().insert(serial.clone()) {
                    async_runtime::spawn(self.0.clone().run(serial));
                }
                Ok(Value::Null)
            }
            _ => return None,
        })
    }
}

/// The phones adb lists. A phone over the network has an address for a serial; the ones Orither's
/// helper makes for itself are left out.
async fn devices() -> Result<Vec<Device>> {
    let listed = adb::run(&["devices", "-l"]).await?;
    let mut devices = Vec::new();
    for line in String::from_utf8_lossy(&listed.stdout).lines().skip(1) {
        let mut words = line.split_whitespace();
        let (Some(serial), Some(state)) = (words.next(), words.next()) else { continue };
        if serial.starts_with("127.0.0.1:") {
            continue;
        }
        let model = words.find_map(|word| word.strip_prefix("model:")).unwrap_or(serial).replace('_', " ");
        let state = match state {
            "device" => "ready",
            "unauthorized" => "unauthorized",
            _ => "offline",
        };
        let app = if state == "ready" { installed_version(serial).await } else { None };
        devices.push(Device { serial: serial.to_owned(), model, state, app });
    }
    Ok(devices)
}

async fn installed_version(serial: &str) -> Option<String> {
    let dump = adb::run(&["-s", serial, "shell", "dumpsys", "package", PACKAGE]).await.ok()?;
    String::from_utf8_lossy(&dump.stdout).lines().find_map(|line| line.trim().strip_prefix("versionName=").map(str::to_owned))
}

/// Whether the phone needs this computer's version of the app: it has none, or an older one.
fn outdated(installed: Option<&str>, ours: &str) -> bool {
    let Some(installed) = installed else { return true };
    match (semver::Version::parse(installed), semver::Version::parse(ours)) {
        (Ok(theirs), Ok(ours)) => theirs < ours,
        _ => false,
    }
}

/// Words a person can act on, for adb's install failures.
fn install_failure(said: &str) -> String {
    let code = said.split(|c: char| !(c.is_ascii_uppercase() || c == '_')).find(|word| word.starts_with("INSTALL_")).unwrap_or("");
    match code {
        "INSTALL_FAILED_UPDATE_INCOMPATIBLE" => "Orither on your phone came from another build. Uninstall it from the phone, then try again.".into(),
        "INSTALL_FAILED_VERIFICATION_FAILURE" => "Play Protect stopped the install. Try again, and on your phone tap More details, then Install anyway.".into(),
        "INSTALL_FAILED_USER_RESTRICTED" => "Your phone did not allow installing over USB. On Xiaomi phones, turn on Install via USB in Developer options, then try again.".into(),
        "INSTALL_FAILED_NO_MATCHING_ABIS" => "Orither needs a phone with a 64-bit ARM processor.".into(),
        "INSTALL_FAILED_OLDER_SDK" => "Orither needs Android 10 or newer.".into(),
        "INSTALL_FAILED_INSUFFICIENT_STORAGE" => "Your phone is out of storage. Free some space, then try again.".into(),
        _ => format!("Installing Orither on your phone failed: {}", said.trim()),
    }
}

/// The TLS provider the updater uses too, set once for the whole process.
fn install_crypto() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installs_when_missing_or_older() {
        assert!(outdated(None, "0.2.0"));
        assert!(outdated(Some("0.1.1"), "0.2.0"));
        assert!(!outdated(Some("0.2.0"), "0.2.0"));
        assert!(!outdated(Some("0.3.0"), "0.2.0"));
        assert!(outdated(Some("0.2.0-beta.1"), "0.2.0"));
    }

    #[test]
    fn explains_install_failures() {
        let said = "Performing Streamed Install\nadb: failed to install x.apk: Failure [INSTALL_FAILED_VERIFICATION_FAILURE: Install not allowed]";
        assert!(install_failure(said).starts_with("Play Protect stopped"));
        assert!(install_failure("Failure [INSTALL_FAILED_UPDATE_INCOMPATIBLE: Existing package signatures do not match]").contains("another build"));
        assert!(install_failure("something odd").contains("something odd"));
    }
}
