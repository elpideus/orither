//! The computer as a phone's Bluetooth headset. BlueZ pairs the two and connects the
//! Hands-Free Profile. PipeWire plays its hands-free side and tells about it on the
//! session bus, through its telephony service, which speaks ofono's language too.
//! WirePlumber makes the call's sound two streams that follow the default speakers
//! and microphone, so normally there is nothing left to link.

use std::{
    collections::{HashMap, HashSet},
    process::Stdio,
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail, ensure};
use bluer::{AddressType, l2cap};
use orither_link::Session;
use dbus::{
    Message, MessageType, Path,
    arg::{PropMap, prop_cast},
    blocking::{
        Connection,
        stdintf::org_freedesktop_dbus::{ObjectManager, Properties},
    },
    message::MatchRule,
};
use serde_json::Value;
use tauri::async_runtime;
use tokio::{process::Command, sync::mpsc::UnboundedSender};

use super::Facts;

const BLUEZ: &str = "org.bluez";
const ADAPTER: &str = "org.bluez.Adapter1";
const DEVICE: &str = "org.bluez.Device1";
const TELEPHONY: &str = "org.pipewire.Telephony";
const TELEPHONY_PATH: &str = "/org/pipewire/Telephony";
/// A phone that has this computer as its hands-free device, by its `Address`.
const GATEWAY: &str = "org.pipewire.Telephony.AudioGateway1";
/// The same connection's sound, whose `State` is `active` while it flows.
const TRANSPORT: &str = "org.pipewire.Telephony.AudioGatewayTransport1";
const CALL: &str = "org.pipewire.Telephony.Call1";
/// The phone's side of the Hands-Free Profile. BlueZ connects a profile by the
/// other device's side of it, and the computer takes the hands-free side.
const HANDS_FREE_GATEWAY: &str = "0000111f-0000-1000-8000-00805f9b34fb";
/// The older Headset Profile's, which a phone may also have connected.
const HEADSET_GATEWAY: &str = "00001112-0000-1000-8000-00805f9b34fb";
const ALREADY_CONNECTED: &str = "org.bluez.Error.AlreadyConnected";
const ALREADY_EXISTS: &str = "org.bluez.Error.AlreadyExists";
const NOT_CONNECTED: &str = "org.bluez.Error.NotConnected";
const INVALID_ARGUMENTS: &str = "org.bluez.Error.InvalidArguments";
const SDP_PSM: u16 = 1;
const CALL_TIMEOUT: Duration = Duration::from_secs(5);
/// Pairing waits for a person on each device to confirm a code.
const PAIR_TIMEOUT: Duration = Duration::from_secs(120);
/// Connecting pages the phone and waits for it to agree.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a phone may take to answer a page, and then to set up hands-free.
const APPEAR_TIMEOUT: Duration = Duration::from_secs(15);
const SETUP_TIMEOUT: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(300);
/// How long the watching thread listens to one bus before it turns to the other.
const LISTEN_FOR: Duration = Duration::from_millis(250);
/// In the names of the loopbacks Orither makes, so that leftovers can be found.
const MARK: &str = "orither_call";
const LOOPBACK_LATENCY: &str = "latency_msec=60";
const NO_TELEPHONY: &str = "PipeWire's Bluetooth telephony isn't running on this computer";

type Objects = HashMap<Path<'static>, HashMap<String, PropMap>>;

fn bluez(bus: &Connection) -> Result<Objects> {
    Ok(bus.with_proxy(BLUEZ, "/", CALL_TIMEOUT).get_managed_objects()?)
}

fn telephony(bus: &Connection) -> Result<Objects> {
    Ok(bus.with_proxy(TELEPHONY, TELEPHONY_PATH, CALL_TIMEOUT).get_managed_objects()?)
}

fn text<'a>(properties: &'a PropMap, key: &str) -> Option<&'a str> {
    prop_cast::<String>(properties, key).map(String::as_str)
}

fn flag(properties: &PropMap, key: &str) -> bool {
    prop_cast::<bool>(properties, key).copied().unwrap_or(false)
}

/// Whether the phone keeps this computer's keys. BlueZ can show a phone as paired for
/// one connection only, as after the Bluetooth link, and Android lets only a bonded
/// device be its headset. BlueZ before 5.66 has no `Bonded`, and pairs for good.
fn bonded(properties: &PropMap) -> bool {
    match prop_cast::<bool>(properties, "Bonded") {
        Some(bonded) => *bonded,
        None => flag(properties, "Paired"),
    }
}

fn same(found: &str, address: &str) -> bool {
    found.eq_ignore_ascii_case(address)
}

/// The phone's object in BlueZ, once BlueZ knows of it.
fn device<'a>(objects: &'a Objects, address: &str) -> Option<(&'a Path<'static>, &'a PropMap)> {
    objects.iter().find_map(|(path, interfaces)| {
        let properties = interfaces.get(DEVICE)?;
        text(properties, "Address").filter(|found| same(found, address)).map(|_| (path, properties))
    })
}

/// The phone's hands-free connection to this computer, as PipeWire's telephony shows it.
struct Gateway {
    path: String,
    sounding: bool,
    in_call: bool,
}

fn gateway(objects: &Objects, address: &str) -> Option<Gateway> {
    let (path, _) = objects.iter().find(|(_, interfaces)| {
        interfaces.get(GATEWAY).and_then(|properties| text(properties, "Address")).is_some_and(|found| same(found, address))
    })?;
    let children = format!("{path}/");
    let within = |at: &Path| at == path || at.starts_with(&children);
    let state_is = |interface: &str, state: &str| {
        objects.iter().any(|(at, interfaces)| {
            within(at) && interfaces.get(interface).and_then(|properties| text(properties, "State")) == Some(state)
        })
    };
    Some(Gateway { path: path.to_string(), sounding: state_is(TRANSPORT, "active"), in_call: state_is(CALL, "active") })
}

pub fn facts(address: Option<&str>) -> Facts {
    let mut facts = Facts::default();
    let Ok(gateways) = Connection::new_session().map_err(anyhow::Error::from).and_then(|bus| telephony(&bus)) else {
        facts.unsupported = Some(NO_TELEPHONY.into());
        return facts;
    };
    let Ok(objects) = Connection::new_system().map_err(anyhow::Error::from).and_then(|bus| bluez(&bus)) else {
        return facts;
    };
    facts.adapter = objects.values().any(|interfaces| interfaces.get(ADAPTER).is_some_and(|adapter| flag(adapter, "Powered")));
    let Some(address) = address else { return facts };
    facts.paired = device(&objects, address).is_some_and(|(_, properties)| bonded(properties));
    if let Some(gateway) = gateway(&gateways, address) {
        facts.connected = true;
        facts.sounding = gateway.sounding;
    }
    facts
}

/// What a person should read when BlueZ says no.
fn explain(err: &dbus::Error) -> String {
    let reason = err.name().unwrap_or_default().trim_start_matches("org.bluez.Error.");
    match reason {
        "AuthenticationCanceled" | "AuthenticationRejected" => "Pairing was turned down".into(),
        "AuthenticationTimeout" => "Nobody confirmed the pairing in time".into(),
        "AuthenticationFailed" => "Pairing failed. The codes may not have matched".into(),
        "InProgress" => "Bluetooth is busy with this already".into(),
        "NotReady" => "Bluetooth is off on this computer".into(),
        "InvalidArguments" => "The phone doesn't offer hands-free calls".into(),
        _ => format!("Bluetooth: {}", err.message().unwrap_or(reason)),
    }
}

/// Waits for BlueZ to know of the phone.
fn appeared(bus: &Connection, address: &str) -> Result<Path<'static>> {
    let until = Instant::now() + APPEAR_TIMEOUT;
    loop {
        let objects = bluez(bus)?;
        if let Some((path, _)) = device(&objects, address) {
            return Ok(path.clone());
        }
        ensure!(Instant::now() < until, "The phone didn't answer over Bluetooth. Is its Bluetooth on, and is it nearby?");
        thread::sleep(POLL);
    }
}

/// Pairs with the phone and trusts it. Each side asks its user to confirm a code:
/// the desktop's own Bluetooth agent here, and Android on the phone.
pub async fn pair(address: &str) -> Result<()> {
    let target: bluer::Address = address.parse()?;
    // A pairing kept for one connection only, as the Bluetooth link leaves, makes the
    // kernel answer "already paired" and no bond is made. Forgetting it first lets a
    // real one start. The link's own connection drops and comes back by itself.
    let held = address.to_owned();
    async_runtime::spawn_blocking(move || forget_passing_pairing(&held)).await??;
    // BlueZ pairs only with a device it knows of. It learns of one as soon as anything
    // connects to it, such as this question to its service directory, which is kept
    // open until pairing is over. The phone need not be discoverable for this.
    let _reach = l2cap::SeqPacket::connect(l2cap::SocketAddr::new(target, AddressType::BrEdr, SDP_PSM)).await;
    let address = address.to_owned();
    async_runtime::spawn_blocking(move || {
        let bus = Connection::new_system()?;
        let path = appeared(&bus, &address)?;
        let device = bus.with_proxy(BLUEZ, path, PAIR_TIMEOUT);
        // A pairing kept for one connection only does not count: Android wants a bond.
        let kept: bool = device.get(DEVICE, "Bonded").or_else(|_| device.get(DEVICE, "Paired"))?;
        if !kept {
            match device.method_call::<(), _, _, _>(DEVICE, "Pair", ()) {
                Err(err) if err.name() != Some(ALREADY_EXISTS) => bail!(explain(&err)),
                _ => {}
            }
        }
        // A trusted phone may connect hands-free again by itself, without anyone asked.
        device.set(DEVICE, "Trusted", true)?;
        Ok(())
    })
    .await?
}

/// Removes the phone from BlueZ when it is paired for this connection only, never when bonded.
fn forget_passing_pairing(address: &str) -> Result<()> {
    let bus = Connection::new_system()?;
    let objects = bluez(&bus)?;
    let Some((path, properties)) = device(&objects, address) else { return Ok(()) };
    if !flag(properties, "Paired") || bonded(properties) {
        return Ok(());
    }
    let device = path.to_string();
    let adapter = device.rsplit_once('/').map(|(adapter, _)| adapter.to_owned()).context("odd BlueZ device path")?;
    let proxy = bus.with_proxy(BLUEZ, adapter, CONNECT_TIMEOUT);
    proxy.method_call::<(), _, _, _>(ADAPTER, "RemoveDevice", (path.clone(),)).map_err(|err| anyhow!(explain(&err)))?;
    // Give BlueZ a moment to let go of the connection before asking again.
    thread::sleep(Duration::from_millis(800));
    Ok(())
}

/// Becomes the phone's hands-free device.
pub fn connect(address: &str) -> Result<()> {
    let bus = Connection::new_system()?;
    let objects = bluez(&bus)?;
    let paired = device(&objects, address).filter(|(_, properties)| bonded(properties));
    // An unpaired phone would be paired on the way, without the user having asked for that.
    let (path, _) = paired.context("Pair the phone with this computer first")?;
    let device = bus.with_proxy(BLUEZ, path.clone(), CONNECT_TIMEOUT);
    match device.method_call::<(), _, _, _>(DEVICE, "ConnectProfile", (HANDS_FREE_GATEWAY,)) {
        Err(err) if err.name() != Some(ALREADY_CONNECTED) => Err(anyhow!(explain(&err))),
        _ => Ok(()),
    }
}

/// Stops being the phone's headset, which puts its calls back on the phone.
pub fn disconnect(address: &str) -> Result<()> {
    let bus = Connection::new_system()?;
    let objects = bluez(&bus)?;
    let Some((path, _)) = device(&objects, address) else { return Ok(()) };
    let device = bus.with_proxy(BLUEZ, path.clone(), CONNECT_TIMEOUT);
    for profile in [HANDS_FREE_GATEWAY, HEADSET_GATEWAY] {
        match device.method_call::<(), _, _, _>(DEVICE, "DisconnectProfile", (profile,)) {
            Err(err) if !matches!(err.name(), Some(NOT_CONNECTED | INVALID_ARGUMENTS)) => bail!(explain(&err)),
            _ => {}
        }
    }
    Ok(())
}

/// Brings the sound of a call in progress to the computer, once hands-free is set up.
/// A phone moves a new call's sound by itself, but not always one already going on.
pub fn pull(address: &str) {
    let Ok(bus) = Connection::new_session() else { return };
    let until = Instant::now() + SETUP_TIMEOUT;
    let gateway = loop {
        if let Some(gateway) = telephony(&bus).ok().and_then(|objects| gateway(&objects, address)) {
            break gateway;
        }
        if Instant::now() > until {
            return;
        }
        thread::sleep(POLL);
    };
    if gateway.in_call && !gateway.sounding {
        let transport = bus.with_proxy(TELEPHONY, gateway.path, CALL_TIMEOUT);
        if let Err(err) = transport.method_call::<(), _, _, _>(TRANSPORT, "Activate", ()) {
            tracing::debug!("the call's sound stayed on the phone: {err}");
        }
    }
}

/// Sends a message on `changes` for every signal from BlueZ or PipeWire's telephony.
pub fn watch(changes: UnboundedSender<()>) {
    thread::spawn(move || {
        let buses = [(Connection::new_system(), BLUEZ), (Connection::new_session(), TELEPHONY)];
        let listening: Vec<Connection> = buses
            .into_iter()
            .filter_map(|(bus, sender)| {
                let bus = bus.ok()?;
                let mut rule = MatchRule::new();
                rule.msg_type = Some(MessageType::Signal);
                rule.sender = Some(sender.into());
                let told = changes.clone();
                bus.add_match(rule, move |(): (), _: &Connection, _: &Message| told.send(()).is_ok()).ok()?;
                Some(bus)
            })
            .collect();
        while !listening.is_empty() && !changes.is_closed() {
            for bus in &listening {
                if let Err(err) = bus.process(LISTEN_FOR) {
                    tracing::warn!("no longer following Bluetooth for calls: {err}");
                    return;
                }
            }
        }
    });
}

/// One end of the call's sound, as PipeWire offers it.
#[derive(Debug, PartialEq)]
enum End {
    /// The phone's side of the call, as a microphone, to play on the speakers.
    FromPhone(String),
    /// What the phone hears, as speakers, to feed from the microphone.
    ToPhone(String),
}

impl End {
    fn loopback(&self) -> Vec<String> {
        let (end, other) = match self {
            Self::FromPhone(node) => (format!("source={node}"), format!("sink_input_properties=node.name={MARK}_from_phone")),
            Self::ToPhone(node) => (format!("sink={node}"), format!("source_output_properties=node.name={MARK}_to_phone")),
        };
        let stay = if matches!(self, Self::FromPhone(_)) { "source_dont_move=true" } else { "sink_dont_move=true" };
        ["module-loopback".into(), end, stay.into(), other, LOOPBACK_LATENCY.into()].into()
    }
}

/// The phone's call sound that nothing links. WirePlumber makes it two streams that it
/// links itself, so this finds something only when a configuration made the phone's
/// side a plain device instead, which no program would use.
fn unlinked(dump: &Value, address: &str) -> Vec<End> {
    let objects = dump.as_array().map(Vec::as_slice).unwrap_or_default();
    let of_type = |kind: &'static str| objects.iter().filter(move |object| object["type"] == kind);
    let linked: HashSet<u64> = of_type("PipeWire:Interface:Link")
        .flat_map(|link| [&link["info"]["output-node-id"], &link["info"]["input-node-id"]])
        .filter_map(Value::as_u64)
        .collect();
    of_type("PipeWire:Interface:Node")
        .filter_map(|node| {
            let properties = &node["info"]["props"];
            let found = properties["api.bluez5.address"].as_str()?;
            let call = properties["api.bluez5.profile"] == "headset-audio-gateway";
            if !same(found, address) || !call || linked.contains(&node["id"].as_u64()?) {
                return None;
            }
            let name = properties["node.name"].as_str()?.to_owned();
            match properties["media.class"].as_str()? {
                "Audio/Source" => Some(End::FromPhone(name)),
                "Audio/Sink" => Some(End::ToPhone(name)),
                _ => None,
            }
        })
        .collect()
}

/// Loopbacks that carry a call's sound which nothing else linked. They go when this is dropped.
pub struct Bridge(Vec<String>);

impl Drop for Bridge {
    fn drop(&mut self) {
        self.0.iter().for_each(|module| unload(module));
    }
}

pub async fn bridge(address: &str) -> Result<Bridge> {
    let dump = Command::new("pw-dump").kill_on_drop(true).output().await.context("running pw-dump")?;
    ensure!(dump.status.success(), "pw-dump: {}", String::from_utf8_lossy(&dump.stderr).trim());
    let mut bridge = Bridge(Vec::new());
    for end in unlinked(&serde_json::from_slice(&dump.stdout)?, address) {
        tracing::info!("linking the call's sound through {end:?}, which nothing else linked");
        let arguments = end.loopback();
        let loaded = Command::new("pactl").arg("load-module").args(&arguments).kill_on_drop(true).output().await?;
        ensure!(loaded.status.success(), "pactl: {}", String::from_utf8_lossy(&loaded.stderr).trim());
        bridge.0.push(String::from_utf8_lossy(&loaded.stdout).trim().to_owned());
    }
    Ok(bridge)
}

fn unload(module: &str) {
    let _ = std::process::Command::new("pactl").args(["unload-module", module]).stderr(Stdio::null()).status();
}

/// The loopbacks of Orither's in a `pactl list short modules` listing.
fn leftovers(listing: &str) -> Vec<&str> {
    listing
        .lines()
        .filter(|line| line.contains("module-loopback") && line.contains(MARK))
        .filter_map(|line| line.split('\t').next())
        .collect()
}

/// Removes loopbacks a crash or a kill left behind.
pub fn tidy() {
    let Ok(output) = std::process::Command::new("pactl").args(["list", "short", "modules"]).output() else {
        return;
    };
    leftovers(&String::from_utf8_lossy(&output.stdout)).into_iter().for_each(unload);
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const PHONE: &str = "AA:BB:CC:DD:EE:FF";

    fn node(id: u64, name: &str, class: &str, profile: &str) -> Value {
        json!({
            "id": id,
            "type": "PipeWire:Interface:Node",
            "info": { "props": {
                "node.name": name,
                "media.class": class,
                "api.bluez5.address": PHONE,
                "api.bluez5.profile": profile,
            } },
        })
    }

    fn link(from: u64, to: u64) -> Value {
        json!({ "id": 900 + from, "type": "PipeWire:Interface:Link", "info": { "output-node-id": from, "input-node-id": to } })
    }

    #[test]
    fn streams_that_wireplumber_linked_need_nothing() {
        let dump = json!([
            node(70, "bluez_input.AA_BB_CC_DD_EE_FF.0", "Stream/Output/Audio", "headset-audio-gateway"),
            node(71, "bluez_output.AA_BB_CC_DD_EE_FF.1", "Stream/Input/Audio", "headset-audio-gateway"),
            link(70, 40),
            link(41, 71),
        ]);
        assert!(unlinked(&dump, PHONE).is_empty());
    }

    #[test]
    fn devices_nothing_links_get_a_loopback() {
        let dump = json!([
            node(70, "bluez_input.phone", "Audio/Source", "headset-audio-gateway"),
            node(71, "bluez_output.phone", "Audio/Sink", "headset-audio-gateway"),
            // The phone's music is not the call's business.
            node(72, "bluez_input.music", "Audio/Source", "a2dp-source"),
        ]);
        let found = unlinked(&dump, &PHONE.to_lowercase());
        assert_eq!(found, [End::FromPhone("bluez_input.phone".into()), End::ToPhone("bluez_output.phone".into())]);
    }

    #[test]
    fn a_loopback_stays_on_the_phone_and_follows_the_defaults() {
        let arguments = End::FromPhone("bluez_input.phone".into()).loopback();
        assert!(arguments.contains(&"source=bluez_input.phone".to_owned()));
        assert!(arguments.contains(&"source_dont_move=true".to_owned()));
        assert!(!arguments.iter().any(|argument| argument.starts_with("sink=")));
        assert!(arguments.iter().any(|argument| argument.contains(MARK)));
    }

    #[test]
    fn only_orither_loopbacks_are_left_over() {
        let listing = "12\tmodule-loopback\tsource=x sink_input_properties=node.name=orither_call_from_phone\t\n\
                       13\tmodule-loopback\tsource=y\t\n\
                       14\tmodule-pipe-sink\tsink_name=orither_speaker\t\n";
        assert_eq!(leftovers(listing), ["12"]);
    }

    #[test]
    fn bluez_refusals_read_as_sentences() {
        let turned_down = dbus::Error::new_custom("org.bluez.Error.AuthenticationRejected", "Authentication Rejected");
        assert_eq!(explain(&turned_down), "Pairing was turned down");
        let other = dbus::Error::new_custom("org.bluez.Error.Failed", "Page Timeout");
        assert_eq!(explain(&other), "Bluetooth: Page Timeout");
    }

    /// An object's path, one of its interfaces, and that interface's text properties.
    type Entry<'a> = (&'a str, &'a str, &'a [(&'a str, &'a str)]);

    fn objects(entries: &[Entry]) -> Objects {
        let mut objects = Objects::new();
        for (path, interface, properties) in entries {
            let properties: PropMap = properties
                .iter()
                .map(|(key, value)| (key.to_string(), dbus::arg::Variant(Box::new(value.to_string()) as Box<dyn dbus::arg::RefArg>)))
                .collect();
            objects.entry(Path::from(path.to_string())).or_default().insert(interface.to_string(), properties);
        }
        objects
    }

    #[test]
    fn a_gateway_is_the_phone_by_its_address() {
        let objects = objects(&[
            ("/org/pipewire/Telephony/ag0", GATEWAY, &[("Address", PHONE)]),
            ("/org/pipewire/Telephony/ag0", TRANSPORT, &[("State", "idle")]),
            ("/org/pipewire/Telephony/ag0/call0", CALL, &[("State", "active")]),
            ("/org/pipewire/Telephony/ag1", GATEWAY, &[("Address", "11:22:33:44:55:66")]),
            ("/org/pipewire/Telephony/ag10", TRANSPORT, &[("State", "active")]),
        ]);
        let found = gateway(&objects, &PHONE.to_lowercase()).expect("the phone's gateway");
        assert_eq!(found.path, "/org/pipewire/Telephony/ag0");
        assert!(found.in_call);
        // ag10's sound is another phone's, though its path starts the same.
        assert!(!found.sounding);
        assert!(gateway(&objects, "00:00:00:00:00:00").is_none());
    }
}

/// Call sound on the link: 16-bit mono at 16 kHz, as the phone's engine sends and takes it.
const LINK_SOUND: [&str; 4] = ["--raw", "--format=s16le", "--rate=16000", "--channels=1"];
/// Marks both streams as a call's, so PipeWire can apply echo cancelling and the like to them.
const CALL_STREAM: [&str; 3] = ["--client-name=Orither", "--stream-name=Phone call", "--property=media.role=phone"];

/// Plays a call's sound from the phone on this computer's default output, and sends
/// its default microphone into the call, until either side stops.
pub async fn carry(session: Session, id: &str) -> Result<()> {
    let mut stream = session.open("calls", "audio", serde_json::json!({ "id": id })).await?;
    let mut player = tokio::process::Command::new("pacat")
        .arg("--playback")
        .args(LINK_SOUND)
        .args(CALL_STREAM)
        .arg("--latency-msec=60")
        .stdin(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("running pacat")?;
    let mut recorder = tokio::process::Command::new("parec")
        .args(LINK_SOUND)
        .args(CALL_STREAM)
        .arg("--latency-msec=20")
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("running parec")?;
    let mut speaker = player.stdin.take().context("no way to play the call")?;
    let mut microphone = recorder.stdout.take().context("no way to hear the microphone")?;
    let hearing = tokio::io::copy(&mut stream.recv, &mut speaker);
    let speaking = tokio::io::copy(&mut microphone, &mut stream.send);
    // Whichever way stops first ends the call's sound here; dropping the rest stops the other.
    tokio::select! {
        heard = hearing => { heard?; }
        spoke = speaking => { spoke?; }
    }
    Ok(())
}
