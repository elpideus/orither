//! Puts the phone's calls on this computer's microphone and speakers. The sound
//! travels over the link like everything else: the phone's engine takes it from the
//! modem or from the calling app, and this side plays and records it (see
//! `platform::carry`). Where the phone has the computer as its Bluetooth headset,
//! the Hands-Free Profile carries it instead, as a fallback that needs no link.
//! Answering, ending and dialing are the phone's own `calls` feature, and pass
//! through to it untouched.

#[cfg(target_os = "linux")]
#[path = "calls/linux.rs"]
mod platform;
#[cfg(not(target_os = "linux"))]
#[path = "calls/unsupported.rs"]
mod platform;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use orither_link::{PeerId, Route, Session};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::async_runtime;
use tokio::{
    sync::mpsc::{self, UnboundedReceiver, UnboundedSender},
    time::sleep,
};

use super::{Feature, Phone};
use crate::preferences;

/// Where each phone's calls should sound, by phone.
const PLACES: &str = "calls-audio";
/// The phones' Bluetooth addresses, which `nearby` learns and keeps.
const ADDRESSES: &str = "nearby-phones";
/// Bluetooth's signals come in bursts while a connection sets up.
const SETTLE: Duration = Duration::from_millis(300);
/// A phone that did not take the computer as its headset is asked again only after this.
const RETRY: Duration = Duration::from_secs(30);
/// WirePlumber links a call's sound by itself, given a moment.
const LINK_WAIT: Duration = Duration::from_millis(1500);
/// How often a call's sound that stops at once is asked for again.
const CARRY_ATTEMPTS: u32 = 5;
/// Sound that flowed this long ended for a reason, and is not asked for again.
const CARRY_SETTLED: Duration = Duration::from_secs(5);
const CARRY_RETRY: Duration = Duration::from_millis(800);
/// The states of a call during which its sound matters.
const IN_CALL: [&str; 4] = ["ringing", "dialing", "active", "held"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum State {
    Unsupported,
    NoBluetooth,
    UnknownPhone,
    Unpaired,
    Pairing,
    Disconnected,
    Connecting,
    Connected,
}

/// Where the user wants a phone's calls to sound.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Place {
    Computer,
    /// Until the user chooses, calls stay where they always were.
    #[default]
    Phone,
}

/// What the UI is told about one phone's call sound.
#[derive(Clone, Debug, PartialEq, Serialize)]
struct Audio {
    state: State,
    #[serde(rename = "where")]
    place: Place,
    detail: Option<String>,
}

/// What this computer knows of one phone over Bluetooth.
#[derive(Clone, Debug, Default)]
pub struct Facts {
    /// Why this computer cannot be a phone's headset, when it cannot.
    pub unsupported: Option<String>,
    /// A Bluetooth adapter is there and on.
    pub adapter: bool,
    pub paired: bool,
    /// The phone has the computer as its hands-free device.
    pub connected: bool,
    /// A call's sound flows between the two right now.
    pub sounding: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Busy {
    Pairing,
    Connecting,
}

/// The state to show, for a phone whose address is `known` or not.
fn state_of(facts: &Facts, known: bool, busy: Option<Busy>) -> State {
    if facts.unsupported.is_some() {
        State::Unsupported
    } else if !facts.adapter {
        State::NoBluetooth
    } else if !known {
        State::UnknownPhone
    } else if busy == Some(Busy::Pairing) {
        State::Pairing
    } else if !facts.paired {
        State::Unpaired
    } else if facts.connected {
        State::Connected
    } else if busy == Some(Busy::Connecting) {
        State::Connecting
    } else {
        State::Disconnected
    }
}

/// Whether the phone's `calls/changed` notice has a call that wants sound.
fn in_call(changed: &Value) -> bool {
    let calls = changed["calls"].as_array().map(Vec::as_slice).unwrap_or_default();
    calls.iter().any(|call| call["state"].as_str().is_some_and(|state| IN_CALL.contains(&state)))
}

struct Shared {
    phone: Phone,
    places: Mutex<HashMap<String, Place>>,
    busy: Mutex<HashMap<PeerId, Busy>>,
    /// Why the last attempt for each phone failed, until the next one.
    failures: Mutex<HashMap<PeerId, String>>,
    tried: Mutex<HashMap<PeerId, Instant>>,
    told: Mutex<HashMap<PeerId, Audio>>,
    /// Links for a call's sound that WirePlumber did not make, while there is a call.
    bridge: Mutex<Option<platform::Bridge>>,
    /// Each call whose sound comes over the link now, by phone and call id.
    carried: Mutex<HashMap<(PeerId, String), Carried>>,
    /// The calls each phone said it has, so a change of place can act on them.
    calls: Mutex<HashMap<PeerId, Value>>,
    changes: UnboundedSender<()>,
}

/// A call's sound on the link, which stops when this is dropped.
struct Carried(async_runtime::JoinHandle<()>);

impl Drop for Carried {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// The calls whose sound should flow: those being talked on.
fn talking(changed: &Value) -> Vec<String> {
    let calls = changed["calls"].as_array().map(Vec::as_slice).unwrap_or_default();
    calls
        .iter()
        .filter(|call| call["state"].as_str() == Some("active"))
        .filter_map(|call| call["id"].as_str().map(str::to_owned))
        .collect()
}

impl Shared {
    /// Whether the link can carry a call's sound: it runs over a network, not over
    /// Bluetooth, which is too slow for it. Bluetooth hands-free is the fallback then.
    fn link_carries(&self, peer: &PeerId) -> bool {
        self.phone.session_of(peer).is_ok_and(|session| session.route() != Route::Nearby)
    }

    /// Carries the sound of each call being talked on over the link, for phones whose
    /// calls should sound here, and stops the rest. While the link carries calls, the
    /// phone is kept from sending them to this computer over Bluetooth as well, which
    /// Android does by itself once the computer is a paired headset.
    fn carry_calls(self: &Arc<Self>, peer: PeerId) {
        let wanted = self.place(&peer) == Place::Computer;
        let by_link = wanted && self.link_carries(&peer);
        let talking = self.calls.lock().unwrap().get(&peer).map(talking).unwrap_or_default();
        if by_link && !talking.is_empty() {
            self.clone().leave_bluetooth(peer);
        }
        let mut carried = self.carried.lock().unwrap();
        carried.retain(|(phone, id), _| *phone != peer || (by_link && talking.contains(id)));
        if !by_link {
            return;
        }
        let Ok(session) = self.phone.session_of(&peer) else { return };
        for id in talking {
            if carried.contains_key(&(peer, id.clone())) {
                continue;
            }
            let (session, call, phone) = (session.clone(), id.clone(), self.phone.clone());
            let task = async_runtime::spawn(async move {
                // The phone can refuse in the first moments of a call, before Android has set the
                // call up for sound. Sound that stops that quickly is asked for again, a few times.
                for attempt in 1..=CARRY_ATTEMPTS {
                    let started = Instant::now();
                    let carried = platform::carry(session.clone(), &call).await;
                    if started.elapsed() >= CARRY_SETTLED {
                        break;
                    }
                    match carried {
                        Err(err) if attempt == CARRY_ATTEMPTS => {
                            tracing::warn!("call sound: {err:#}");
                            phone.tell_ui("calls", "failed", serde_json::json!({ "error": format!("Call sound: {err:#}") }));
                        }
                        Err(err) => tracing::info!("call sound, attempt {attempt}: {err:#}"),
                        Ok(()) => tracing::info!("call sound stopped at once, attempt {attempt}"),
                    }
                    sleep(CARRY_RETRY).await;
                }
            });
            tracing::info!("carrying a call's sound over the link");
            carried.insert((peer, id), Carried(task));
        }
    }

    /// Drops the phone's Bluetooth hands-free connection to this computer, if it has one.
    fn leave_bluetooth(self: Arc<Self>, peer: PeerId) {
        let connected = self.told.lock().unwrap().get(&peer).is_some_and(|audio| audio.state == State::Connected);
        let Some(address) = self.address(&peer).filter(|_| connected) else { return };
        async_runtime::spawn(async move {
            let gone = async_runtime::spawn_blocking(move || platform::disconnect(&address)).await;
            match gone {
                Ok(Ok(())) => tracing::info!("left Bluetooth hands-free, the link carries the call"),
                Ok(Err(err)) => tracing::warn!("leaving Bluetooth hands-free: {err:#}"),
                Err(err) => tracing::warn!("leaving Bluetooth hands-free: {err:#}"),
            }
            let _ = self.changes.send(());
        });
    }

    /// The phone a UI request names, or else the one the UI shows.
    fn peer_for(&self, body: &Value) -> Result<PeerId> {
        match body["peer"].as_str().and_then(|peer| peer.parse().ok()) {
            Some(peer) => Ok(peer),
            None => self.peer(),
        }
    }

    /// The phone the UI shows, or else the one connected.
    fn peer(&self) -> Result<PeerId> {
        match self.phone.selected() {
            Some(peer) => Ok(peer),
            None => Ok(self.phone.session()?.peer()),
        }
    }

    fn address(&self, peer: &PeerId) -> Option<String> {
        let mut addresses: HashMap<String, String> = preferences::read(&self.phone.app, ADDRESSES)?;
        addresses.remove(&peer.to_string())
    }

    fn place(&self, peer: &PeerId) -> Place {
        self.places.lock().unwrap().get(&peer.to_string()).copied().unwrap_or_default()
    }

    fn set_place(&self, peer: &PeerId, place: Place) {
        let mut places = self.places.lock().unwrap();
        places.insert(peer.to_string(), place);
        preferences::write(&self.phone.app, PLACES, &*places);
    }

    async fn facts(&self, address: Option<String>) -> Facts {
        async_runtime::spawn_blocking(move || platform::facts(address.as_deref())).await.unwrap_or_default()
    }

    async fn audio(&self, peer: &PeerId) -> (Audio, Facts) {
        let address = self.address(peer);
        let known = address.is_some();
        let facts = self.facts(address).await;
        let busy = self.busy.lock().unwrap().get(peer).copied();
        let detail = facts.unsupported.clone().or_else(|| self.failures.lock().unwrap().get(peer).cloned());
        let audio = Audio { state: state_of(&facts, known, busy), place: self.place(peer), detail };
        (audio, facts)
    }

    async fn audio_now(&self, body: &Value) -> Result<Value> {
        let peer = self.peer_for(body)?;
        Ok(serde_json::to_value(self.audio(&peer).await.0)?)
    }

    /// Tells the UI about a phone's call sound, if anything changed.
    async fn tell(&self, peer: &PeerId) -> Facts {
        let (audio, facts) = self.audio(peer).await;
        let before = self.told.lock().unwrap().insert(*peer, audio.clone());
        if before.as_ref() != Some(&audio) {
            self.phone.tell_ui_about(peer, "calls", "audio", serde_json::to_value(&audio).unwrap_or_default());
        }
        facts
    }

    fn set_busy(&self, peer: &PeerId, busy: Option<Busy>) {
        let mut all = self.busy.lock().unwrap();
        match busy {
            Some(busy) => all.insert(*peer, busy),
            None => all.remove(peer),
        };
        let _ = self.changes.send(());
    }

    fn failed(&self, peer: &PeerId, outcome: &Result<()>) {
        let mut failures = self.failures.lock().unwrap();
        match outcome {
            Ok(()) => failures.remove(peer),
            Err(err) => {
                tracing::warn!("call sound: {err:#}");
                failures.insert(*peer, format!("{err:#}"))
            }
        };
    }

    /// Asks BlueZ to pair with the phone. Both devices show a code for the user to confirm.
    async fn pair(self: Arc<Self>, body: &Value) -> Result<Value> {
        let peer = self.peer_for(body)?;
        let address = self.address(&peer).context("this computer doesn't know the phone's Bluetooth address yet")?;
        self.set_busy(&peer, Some(Busy::Pairing));
        let shared = self.clone();
        async_runtime::spawn(async move {
            let paired = platform::pair(&address).await;
            shared.failed(&peer, &paired);
            shared.set_busy(&peer, None);
            // Pairing for calls means the user wants them here.
            if paired.is_ok() {
                shared.set_place(&peer, Place::Computer);
                shared.connect(peer, address).await;
            }
        });
        self.audio_now(body).await
    }

    /// Moves call sound to the computer or back to the phone, and remembers that.
    async fn route(self: Arc<Self>, body: &Value) -> Result<Value> {
        let place: Place = serde_json::from_value(body["to"].clone()).context("to the computer or the phone?")?;
        let peer = self.peer_for(body)?;
        self.set_place(&peer, place);
        self.failures.lock().unwrap().remove(&peer);
        match place {
            // The link carries the sound; Bluetooth joins in only for a phone already paired for it.
            Place::Computer => {
                self.tried.lock().unwrap().remove(&peer);
                self.clone().reconnect(peer);
            }
            Place::Phone => {
                if let Some(address) = self.address(&peer) {
                    let gone = async_runtime::spawn_blocking(move || platform::disconnect(&address)).await?;
                    self.failed(&peer, &gone);
                }
            }
        }
        let _ = self.changes.send(());
        self.carry_calls(peer);
        self.audio_now(body).await
    }

    async fn connect(&self, peer: PeerId, address: String) {
        self.tried.lock().unwrap().insert(peer, Instant::now());
        self.set_busy(&peer, Some(Busy::Connecting));
        let connected = async_runtime::spawn_blocking(move || {
            platform::connect(&address)?;
            // A call already going on moves over too, as it would to a headset turned on mid-call.
            platform::pull(&address);
            anyhow::Ok(())
        })
        .await
        .unwrap_or_else(|err| Err(err.into()));
        self.failed(&peer, &connected);
        self.set_busy(&peer, None);
    }

    /// Connects a phone whose calls should sound here, unless that was just tried.
    fn reconnect(self: Arc<Self>, peer: PeerId) {
        let recently = self.tried.lock().unwrap().get(&peer).is_some_and(|tried| tried.elapsed() < RETRY);
        if self.place(&peer) != Place::Computer || recently || self.busy.lock().unwrap().contains_key(&peer) {
            return;
        }
        // Bluetooth hands-free is only the fallback, for when the link itself runs over Bluetooth.
        if self.link_carries(&peer) {
            return;
        }
        let Some(address) = self.address(&peer) else { return };
        async_runtime::spawn(async move {
            let facts = self.facts(Some(address.clone())).await;
            if facts.unsupported.is_none() && facts.adapter && facts.paired && !facts.connected {
                self.connect(peer, address).await;
            }
        });
    }

    /// Keeps the UI up to date with Bluetooth and PipeWire, and links a call's sound
    /// should WirePlumber leave it unlinked.
    async fn follow(self: Arc<Self>, mut changes: UnboundedReceiver<()>) {
        while changes.recv().await.is_some() {
            sleep(SETTLE).await;
            while changes.try_recv().is_ok() {}
            let Ok(peer) = self.peer() else { continue };
            let facts = self.tell(&peer).await;
            match self.address(&peer).filter(|_| facts.sounding) {
                Some(address) => self.bridge(&address).await,
                None => drop(self.bridge.lock().unwrap().take()),
            }
        }
    }

    async fn bridge(&self, address: &str) {
        if self.bridge.lock().unwrap().is_some() {
            return;
        }
        sleep(LINK_WAIT).await;
        match platform::bridge(address).await {
            Ok(bridge) => *self.bridge.lock().unwrap() = Some(bridge),
            Err(err) => tracing::warn!("linking the call's sound: {err:#}"),
        }
    }
}

pub struct Calls {
    shared: Arc<Shared>,
}

impl Calls {
    pub fn start(phone: Phone) -> Arc<dyn Feature> {
        // Only one Orither runs at a time, so any link left now was left by a crash.
        platform::tidy();
        let (changes, heard) = mpsc::unbounded_channel();
        platform::watch(changes.clone());
        let places = preferences::read(&phone.app, PLACES).unwrap_or_default();
        let shared = Arc::new(Shared {
            phone,
            places: Mutex::new(places),
            busy: Mutex::default(),
            failures: Mutex::default(),
            tried: Mutex::default(),
            told: Mutex::default(),
            bridge: Mutex::default(),
            carried: Mutex::default(),
            calls: Mutex::default(),
            changes,
        });
        async_runtime::spawn(shared.clone().follow(heard));
        Arc::new(Self { shared })
    }

    /// Whether `peer` is the phone whose calls the computer follows.
    fn followed(&self, peer: &PeerId) -> bool {
        self.shared.phone.selected().is_none_or(|selected| selected == *peer)
    }
}

#[async_trait::async_trait]
impl Feature for Calls {
    fn id(&self) -> &'static str {
        "calls"
    }

    async fn command(&self, method: &str, body: &Value) -> Option<Result<Value>> {
        let shared = self.shared.clone();
        Some(match method {
            "audio" => shared.audio_now(body).await,
            "pair" => shared.pair(body).await,
            "route" => shared.route(body).await,
            _ => return None,
        })
    }

    fn connected(&self, session: &Session) {
        if self.followed(&session.peer()) {
            let _ = self.shared.changes.send(());
            self.shared.clone().reconnect(session.peer());
        }
    }

    fn switched(&self, session: Option<&Session>) {
        let _ = self.shared.changes.send(());
        if let Some(session) = session {
            self.shared.clone().reconnect(session.peer());
        }
    }

    /// A call is a good moment to bring its sound here, should the phone have dropped
    /// the computer. The notice still goes on to the UI.
    fn notice(&self, peer: &PeerId, method: &str, body: &Value) -> bool {
        if method == "changed" {
            self.shared.calls.lock().unwrap().insert(*peer, body.clone());
            self.shared.carry_calls(*peer);
            if in_call(body) && self.followed(peer) {
                self.shared.clone().reconnect(*peer);
            }
        }
        false
    }

    fn disconnected(&self, peer: &PeerId) {
        self.shared.calls.lock().unwrap().remove(peer);
        self.shared.carried.lock().unwrap().retain(|(phone, _), _| phone != peer);
    }

    fn stopping(&self) {
        self.shared.carried.lock().unwrap().clear();
        drop(self.shared.bridge.lock().unwrap().take());
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn facts(adapter: bool, paired: bool, connected: bool) -> Facts {
        Facts { unsupported: None, adapter, paired, connected, sounding: false }
    }

    #[test]
    fn what_is_missing_first_is_what_shows() {
        let unsupported = Facts { unsupported: Some("no PipeWire".into()), ..facts(true, true, true) };
        assert_eq!(state_of(&unsupported, true, None), State::Unsupported);
        assert_eq!(state_of(&facts(false, true, true), true, None), State::NoBluetooth);
        assert_eq!(state_of(&facts(true, true, true), false, None), State::UnknownPhone);
        assert_eq!(state_of(&facts(true, false, false), true, None), State::Unpaired);
    }

    #[test]
    fn pairing_and_connecting_show_until_they_are_done() {
        assert_eq!(state_of(&facts(true, false, false), true, Some(Busy::Pairing)), State::Pairing);
        assert_eq!(state_of(&facts(true, true, false), true, Some(Busy::Connecting)), State::Connecting);
        assert_eq!(state_of(&facts(true, true, true), true, Some(Busy::Connecting)), State::Connected);
        assert_eq!(state_of(&facts(true, true, false), true, None), State::Disconnected);
    }

    #[test]
    fn the_ui_reads_states_and_places_as_words() {
        let audio = Audio { state: State::NoBluetooth, place: Place::Computer, detail: None };
        assert_eq!(serde_json::to_value(audio).unwrap(), json!({ "state": "no-bluetooth", "where": "computer", "detail": null }));
        assert_eq!(serde_json::from_value::<Place>(json!("phone")).unwrap(), Place::Phone);
        assert!(serde_json::from_value::<Place>(json!("tablet")).is_err());
    }

    #[test]
    fn only_live_calls_want_sound() {
        assert!(in_call(&json!({ "calls": [{ "state": "ended" }, { "state": "ringing" }] })));
        assert!(!in_call(&json!({ "calls": [{ "state": "ended" }] })));
        assert!(!in_call(&json!({ "calls": [] })));
        assert!(!in_call(&json!(null)));
    }
}
