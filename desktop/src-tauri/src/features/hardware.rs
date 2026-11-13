//! Lends the phone's microphone, speaker and camera to this computer. A device
//! stays on for as long as it is wanted, and comes back when the phone does.

#[cfg(target_os = "linux")]
#[path = "hardware/linux.rs"]
mod platform;

#[cfg(not(target_os = "linux"))]
#[path = "hardware/unsupported.rs"]
mod platform;

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use orither_link::{PeerId, Session};
use serde_json::{Value, json};
use tauri::async_runtime::{self, JoinHandle};

use super::{Feature, Phone};
use crate::preferences;

const DEVICES: [&str; 3] = ["microphone", "speaker", "camera"];
const WANTED: &str = "hardware-wanted";

/// A device in use, which [`platform`] tidies up once it is dropped.
type Running = JoinHandle<()>;

pub struct Hardware {
    phone: Phone,
    wanted: Mutex<HashSet<String>>,
    running: Arc<Mutex<HashMap<String, Running>>>,
    /// The phone the devices belong to while they run. Only one phone lends them at a time.
    lender: Mutex<Option<PeerId>>,
}

impl Hardware {
    pub fn new(phone: Phone) -> Arc<dyn Feature> {
        // Only one Orither runs at a time, so any device there now was left by a crash.
        platform::remove_devices();
        // The phone only listens or films while a program uses the device, so all are safe to offer.
        let wanted = preferences::read(&phone.app, WANTED).unwrap_or_else(|| DEVICES.map(str::to_owned).into());
        Arc::new(Self { phone, wanted: Mutex::new(wanted), running: Arc::default(), lender: Mutex::default() })
    }

    fn stop(&self, id: &str) {
        if let Some(running) = self.running.lock().unwrap().remove(id) {
            running.abort();
        }
    }

    /// Starts a device, unless something already explained why it cannot be.
    fn start(&self, id: &str, session: Session) {
        if platform::blocked(id).is_some() || self.running.lock().unwrap().contains_key(id) {
            return;
        }
        *self.lender.lock().unwrap() = Some(session.peer());
        let (name, running, phone) = (id.to_owned(), self.running.clone(), self.phone.clone());
        let task = async_runtime::spawn(async move {
            let ended = platform::run(&name, &phone, session).await;
            running.lock().unwrap().remove(&name);
            if let Err(err) = ended {
                tracing::warn!("{name}: {err:#}");
                phone.tell_ui("hardware", "failed", json!({ "id": name, "error": format!("{err:#}") }));
            }
        });
        self.running.lock().unwrap().insert(id.to_owned(), task);
    }

    /// Does the one-time setup a device needs, then starts it if it is wanted.
    async fn set_up(&self, body: &Value) -> Result<Value> {
        let id = body["id"].as_str().context("no device")?;
        platform::set_up(id).await?;
        let wanted = self.wanted.lock().unwrap().contains(id);
        if wanted && let Ok(session) = self.phone.session() {
            self.start(id, session);
        }
        Ok(self.devices())
    }

    fn stop_all(&self) {
        DEVICES.iter().for_each(|id| self.stop(id));
        *self.lender.lock().unwrap() = None;
    }

    fn start_wanted(&self, session: &Session) {
        let wanted: Vec<_> = self.wanted.lock().unwrap().iter().cloned().collect();
        wanted.iter().for_each(|id| self.start(id, session.clone()));
    }

    fn set(&self, body: &Value) -> Result<Value> {
        let id = body["id"].as_str().context("no device")?;
        let on = body["on"].as_bool().context("neither on nor off")?;
        let mut wanted = self.wanted.lock().unwrap();
        if on {
            wanted.insert(id.to_owned());
        } else {
            wanted.remove(id);
        }
        preferences::write(&self.phone.app, WANTED, &*wanted);
        drop(wanted);

        if on {
            self.start(id, self.phone.session()?);
        } else {
            self.stop(id);
        }
        Ok(Value::Null)
    }

    fn devices(&self) -> Value {
        let (wanted, running) = (self.wanted.lock().unwrap(), self.running.lock().unwrap());
        let device = |id: &&str| {
            let blocked = platform::blocked(id);
            let on = blocked.is_none() && (wanted.contains(*id) || running.contains_key(*id));
            json!({ "id": id, "on": on, "blocked": blocked, "setup": platform::can_set_up(id) })
        };
        Value::Array(DEVICES.iter().map(|id| device(&id)).collect())
    }
}

#[async_trait::async_trait]
impl Feature for Hardware {
    fn id(&self) -> &'static str {
        "hardware"
    }

    async fn command(&self, method: &str, body: &Value) -> Option<Result<Value>> {
        Some(match method {
            "devices" => Ok(self.devices()),
            "set" => self.set(body),
            "setup" => self.set_up(body).await,
            _ => return None,
        })
    }

    /// The phone the UI shows lends its devices, or the first to connect while none does.
    fn connected(&self, session: &Session) {
        let theirs = match self.phone.selected() {
            Some(selected) => selected == session.peer() || self.phone.session_of(&selected).is_err(),
            None => true,
        };
        if theirs && self.running.lock().unwrap().is_empty() {
            self.start_wanted(session);
        }
    }

    fn disconnected(&self, peer: &PeerId) {
        if *self.lender.lock().unwrap() == Some(*peer) {
            self.stop_all();
        }
    }

    fn switched(&self, session: Option<&Session>) {
        if let Some(session) = session.filter(|session| *self.lender.lock().unwrap() != Some(session.peer())) {
            self.stop_all();
            self.start_wanted(session);
        }
    }

    /// A stopped device goes once its task ends, which may come too late as Orither quits.
    fn stopping(&self) {
        self.stop_all();
        platform::remove_devices();
    }
}
