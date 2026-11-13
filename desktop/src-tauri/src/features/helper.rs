//! Starts the phone's helper, the part of Orither that runs there with adb's rights,
//! whenever it is not running. Only adb can start it. With the phone plugged in that
//! is one command. Without a cable the phone opens Android's wireless debugging for a
//! moment and this computer's adb reaches it through the link, so a phone that
//! restarted far from its computer comes back without anyone plugging it in.

use std::{
    collections::HashMap,
    env, fmt,
    net::Ipv4Addr,
    path::{Path, PathBuf},
    process::Output,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use orither_link::{PeerId, Session};
use serde::Deserialize;
use serde_json::{Value, json};
use tauri::async_runtime;
use tokio::{
    net::TcpListener,
    process::Command,
    sync::Mutex,
    time::{Instant, sleep},
};

use super::{Feature, Phone};
use crate::preferences;

const STARTS_IN: Duration = Duration::from_secs(2);
/// How long the phone may take to accept this computer's key over the network.
const ONLINE_WITHIN: Duration = Duration::from_secs(15);
const ONLINE_CHECK: Duration = Duration::from_millis(500);
/// A phone that has just restarted may not be on Wi-Fi yet.
const RETRY_EVERY: Duration = Duration::from_secs(60);
const ATTEMPTS: u32 = 15;
/// Android forgets a computer's adb key after seven days without contact, and a
/// restarted phone would then have no way back to its helper.
const REFRESH_AFTER: Duration = Duration::from_secs(4 * 24 * 60 * 60);
const CONTACT: &str = "helper-contact";

const NEEDS_USB: &str =
    "Connect your phone to this computer with a USB cable once, so that Orither can start its helper on it.";
const NEEDS_ADB: &str = "Install Android's adb tool, so that Orither can start its helper on your phone";
const NEEDS_TRUST: &str = "Allow USB debugging on your phone, with Always allow from this computer ticked.";
const NEEDS_WIFI: &str = "Connect your phone to Wi-Fi, so that Orither can start its helper without a cable.";
const NEEDS_KEY: &str =
    "Your phone no longer knows this computer. Connect it with a USB cable once and allow USB debugging.";
const UNREACHABLE: &str = "Your phone's wireless debugging did not answer.";
const NOT_ALLOWED: &str = "Wireless debugging was not allowed on your phone's network.";
const UNFAMILIAR: &str = "Your phone is on a network it has not used for this before.";
const DID_NOT_START: &str = "The helper did not start on your phone.";

/// One start at a time, since each one goes through the same adb and the same phone.
static STARTING: Mutex<()> = Mutex::const_new(());

/// Why the helper could not start, when trying again a little later may help.
#[derive(Debug)]
struct Later(&'static str);

impl fmt::Display for Later {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for Later {}

#[derive(Deserialize)]
struct Status {
    running: bool,
    launch: String,
    model: String,
    /// Whether the phone can open its wireless debugging.
    recovers: bool,
}

async fn status(session: &Session) -> Result<Status> {
    Ok(serde_json::from_value(session.call("helper", "status", json!({})).await?)?)
}

#[derive(Deserialize)]
#[serde(tag = "state", rename_all = "lowercase")]
enum Opened {
    Ready { port: u16 },
    Permission,
    Wifi,
    Unreachable,
    Denied,
    Unfamiliar,
}

/// Starts the helper on the phone, unless it is running already.
pub async fn ensure_engine(phone: &Phone, session: &Session) -> Result<()> {
    let _alone = STARTING.lock().await;
    let status = status(session).await?;
    if status.running {
        return Ok(());
    }
    match usb(&status.model).await {
        Ok(Usb::Ready(serial)) => shell(&serial, &status.launch).await?,
        _ if status.recovers => {
            let woke = async {
                let port = open(session, false).await?;
                phone.tell_ui("helper", "waking", json!({}));
                through_link(session, port, Some(&status.launch)).await
            }
            .await;
            phone.tell_ui("helper", "woke", json!({}));
            woke?;
        }
        Ok(Usb::Unauthorized) => bail!(NEEDS_TRUST),
        Ok(Usb::Absent) => bail!(NEEDS_USB),
        Err(err) => return Err(err),
    }
    sleep(STARTS_IN).await;
    if !self::status(session).await?.running {
        bail!(DID_NOT_START);
    }
    touched(phone, &session.peer());
    Ok(())
}

fn tool() -> PathBuf {
    let in_sdk = |sdk: PathBuf| sdk.join("platform-tools/adb");
    let candidates = env::var_os("ANDROID_HOME")
        .map(|sdk| in_sdk(sdk.into()))
        .into_iter()
        .chain(env::var_os("HOME").map(|home| in_sdk(Path::new(&home).join("Android/Sdk"))));
    let found = candidates.into_iter().find(|path| path.exists());
    found.unwrap_or_else(|| "adb".into())
}

async fn adb(args: &[&str]) -> Result<Output> {
    Command::new(tool()).args(args).kill_on_drop(true).output().await.context(NEEDS_ADB)
}

async fn shell(serial: &str, command: &str) -> Result<()> {
    if !adb(&["-s", serial, "shell", command]).await?.status.success() {
        bail!(DID_NOT_START);
    }
    Ok(())
}

#[derive(Debug, PartialEq)]
enum Usb {
    Ready(String),
    /// Some phone waits for its user to allow this computer, likely this one.
    Unauthorized,
    Absent,
}

async fn usb(model: &str) -> Result<Usb> {
    let listed = adb(&["devices", "-l"]).await?;
    Ok(find(&String::from_utf8_lossy(&listed.stdout), model))
}

/// Finds the phone among the devices on USB by its model, which adb lists only
/// once the phone has allowed this computer.
fn find(listed: &str, model: &str) -> Usb {
    let model = format!("model:{}", model.replace(' ', "_"));
    let mut found = Usb::Absent;
    for line in listed.lines().skip(1) {
        let mut words = line.split_whitespace();
        let (Some(serial), Some(state)) = (words.next(), words.next()) else { continue };
        // Devices over the network have an address for a name.
        if serial.contains(':') {
            continue;
        }
        match state {
            "device" if words.any(|word| word == model) => return Usb::Ready(serial.to_owned()),
            "unauthorized" => found = Usb::Unauthorized,
            _ => {}
        }
    }
    found
}

/// Has the phone open its wireless debugging, and returns the port it is on. A
/// `quiet` open gives up rather than have Android ask the user anything.
async fn open(session: &Session, quiet: bool) -> Result<u16> {
    let opened = session.call("helper", "open", json!({ "quiet": quiet })).await?;
    match serde_json::from_value(opened)? {
        Opened::Ready { port } => Ok(port),
        Opened::Permission => bail!(NEEDS_USB),
        Opened::Wifi => bail!(Later(NEEDS_WIFI)),
        Opened::Unreachable => bail!(Later(UNREACHABLE)),
        Opened::Denied => bail!(NOT_ALLOWED),
        Opened::Unfamiliar => bail!(UNFAMILIAR),
    }
}

/// Reaches the phone's adbd at the `port` it opened, carried by the link, and runs
/// `command` there. The phone then closes its wireless debugging again.
async fn through_link(session: &Session, port: u16, command: Option<&str>) -> Result<()> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let serial = listener.local_addr()?.to_string();
    let carrying = async_runtime::spawn(carry(listener, session.clone(), port));
    let reached = async {
        log_in(&serial).await?;
        match command {
            Some(command) => shell(&serial, command).await,
            None => Ok(()),
        }
    }
    .await;
    let _ = adb(&["disconnect", &serial]).await;
    carrying.abort();
    let _ = session.notify("helper", "close", json!({}));
    reached
}

/// Carries each connection adb makes to `listener` to the phone's adbd.
async fn carry(listener: TcpListener, session: Session, port: u16) {
    while let Ok((socket, _)) = listener.accept().await {
        let session = session.clone();
        async_runtime::spawn(async move {
            let (reader, writer) = socket.into_split();
            let carried = match session.open("helper", "connect", json!({ "port": port })).await {
                Ok(stream) => stream.bridge(reader, writer).await,
                Err(err) => Err(err),
            };
            if let Err(err) = carried {
                tracing::debug!("carrying adb: {err:#}");
            }
        });
    }
}

/// Connects adb, and waits for the phone to accept this computer's key.
async fn log_in(serial: &str) -> Result<()> {
    let connected = adb(&["connect", serial]).await?;
    let said = String::from_utf8_lossy(&connected.stdout);
    if said.contains("authenticate") {
        bail!(NEEDS_KEY);
    }
    if !said.contains("connected to") {
        tracing::debug!("adb connect: {}", said.trim());
        bail!(Later(UNREACHABLE));
    }
    let deadline = Instant::now() + ONLINE_WITHIN;
    while Instant::now() < deadline {
        let state = adb(&["-s", serial, "get-state"]).await?;
        if String::from_utf8_lossy(&state.stdout).trim() == "device" {
            return Ok(());
        }
        sleep(ONLINE_CHECK).await;
    }
    bail!(NEEDS_KEY)
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs())
}

/// When this computer's key last reached each phone, in seconds since 1970.
fn contacts(phone: &Phone) -> HashMap<String, u64> {
    preferences::read(&phone.app, CONTACT).unwrap_or_default()
}

fn touched(phone: &Phone, peer: &PeerId) {
    let mut contacts = contacts(phone);
    contacts.insert(peer.to_string(), now());
    preferences::write(&phone.app, CONTACT, &contacts);
}

/// Uses this computer's key once more, if it has gone unused for a while.
async fn refresh(phone: &Phone, session: &Session) -> Result<()> {
    let last = contacts(phone).get(&session.peer().to_string()).copied().unwrap_or(0);
    if now().saturating_sub(last) < REFRESH_AFTER.as_secs() {
        return Ok(());
    }
    let _alone = STARTING.lock().await;
    let status = status(session).await?;
    if !status.recovers {
        return Ok(());
    }
    if !matches!(usb(&status.model).await?, Usb::Ready(_)) {
        let port = open(session, true).await?;
        through_link(session, port, None).await?;
    }
    touched(phone, &session.peer());
    Ok(())
}

/// Starts the helper once the phone connects, trying again for a while if the
/// phone is not ready for it yet.
async fn wake(phone: Phone, session: Session) {
    for attempt in 1..=ATTEMPTS {
        let err = match ensure_engine(&phone, &session).await {
            Ok(()) => match refresh(&phone, &session).await {
                Ok(()) => return,
                Err(err) => return tracing::debug!("the adb key was not refreshed: {err:#}"),
            },
            Err(err) => err,
        };
        if err.is::<Later>() && attempt < ATTEMPTS && session.is_alive() {
            sleep(RETRY_EVERY).await;
            continue;
        }
        tracing::debug!("the helper did not start: {err:#}");
        // Only a phone that was set up to come back without the cable expects to.
        if status(&session).await.is_ok_and(|status| status.recovers) {
            phone.tell_ui("helper", "failed", json!({ "error": format!("{err:#}") }));
        }
        return;
    }
}

/// Checks the way back without a cable while the user is at the phone, so that
/// Android asks about the network now rather than after a restart.
async fn rehearse(phone: Phone) {
    let Ok(session) = phone.session() else { return };
    let _alone = STARTING.lock().await;
    phone.tell_ui("helper", "rehearsing", json!({}));
    let reached = async { through_link(&session, open(&session, false).await?, None).await }.await;
    match reached {
        Ok(()) => {
            touched(&phone, &session.peer());
            phone.tell_ui("helper", "rehearsed", json!({}));
        }
        Err(err) => phone.tell_ui("helper", "failed", json!({ "error": format!("{err:#}") })),
    }
}

pub struct Helper {
    phone: Phone,
}

impl Helper {
    pub fn new(phone: Phone) -> Arc<dyn Feature> {
        Arc::new(Self { phone })
    }
}

#[async_trait::async_trait]
impl Feature for Helper {
    fn id(&self) -> &'static str {
        "helper"
    }

    /// Other features lean on the helper, so it should not wait to be needed.
    fn connected(&self, session: &Session) {
        async_runtime::spawn(wake(self.phone.clone(), session.clone()));
    }

    fn notice(&self, _peer: &PeerId, method: &str, _body: &Value) -> bool {
        if method != "ready" {
            return false;
        }
        async_runtime::spawn(rehearse(self.phone.clone()));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LISTED: &str = "List of devices attached\n\
        127.0.0.1:40111 device product:pirates model:POCO_F7 device:pirates transport_id:4\n\
        R5CT1234 device usb:1-2 product:a54 model:SM_A546B device:a54x transport_id:2\n\
        8f1c2d3e device usb:1-1 product:pirates model:POCO_F7 device:pirates transport_id:3\n";

    #[test]
    fn the_phone_is_found_on_usb_by_its_model() {
        assert_eq!(find(LISTED, "POCO F7"), Usb::Ready("8f1c2d3e".into()));
        assert_eq!(find(LISTED, "SM-A556B"), Usb::Absent);
    }

    #[test]
    fn a_phone_still_asking_is_told_apart_from_none() {
        let asking = "List of devices attached\n8f1c2d3e unauthorized usb:1-1 transport_id:5\n";
        assert_eq!(find(asking, "POCO F7"), Usb::Unauthorized);
        assert_eq!(find("List of devices attached\n\n", "POCO F7"), Usb::Absent);
    }

    #[test]
    fn every_answer_to_open_is_understood() {
        let ready: Opened = serde_json::from_value(json!({ "state": "ready", "port": 37099 })).unwrap();
        assert!(matches!(ready, Opened::Ready { port: 37099 }));
        for state in ["permission", "wifi", "unreachable", "denied", "unfamiliar"] {
            assert!(serde_json::from_value::<Opened>(json!({ "state": state })).is_ok(), "{state}");
        }
    }

    #[test]
    fn only_some_failures_are_worth_waiting_out() {
        assert!(anyhow::anyhow!(Later(NEEDS_WIFI)).is::<Later>());
        assert!(!anyhow::anyhow!(NOT_ALLOWED).is::<Later>());
    }
}
