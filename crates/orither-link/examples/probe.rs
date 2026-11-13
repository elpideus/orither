//! Walks through every phone feature over the link, printing only the shape of
//! each answer so that no personal data ends up on the terminal.
//!
//! Nothing is sent to anyone: no text message, no WhatsApp reply, and the audio
//! played through the phone's speaker is silence. It does replace the phone's
//! clipboard, and it writes and deletes one small file in `Download`.
//!
//! `cargo run -p orither-link --example probe -- <data dir>`

use std::{
    env,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use orither_link::{Device, DeviceKind, Event, Node, NodeConfig, Session};
use serde_json::{Value, json};
use tokio::time::{sleep, timeout};

const WAIT: Duration = Duration::from_secs(20);
const READ_LIMIT: usize = 8 * 1024 * 1024;
const PROBE_FILE: &str = "orither-probe.txt";
const PROBE_TEXT: &[u8] = b"orither probe";
const SET_CLIPBOARD: u8 = 9;

fn shape(value: &Value) -> Value {
    match value {
        Value::Null | Value::Bool(_) => value.clone(),
        Value::Number(_) => json!("number"),
        Value::String(text) => json!(format!("text:{}", text.chars().count())),
        Value::Array(items) => json!({ "count": items.len(), "first": items.first().map(shape) }),
        Value::Object(fields) => {
            Value::Object(fields.iter().map(|(key, value)| (key.clone(), shape(value))).collect())
        }
    }
}

async fn call(session: &Session, feature: &str, method: &str, body: Value) -> Value {
    match timeout(WAIT, session.call(feature, method, body)).await {
        Ok(Ok(value)) => {
            println!("ok   {feature}.{method} {}", shape(&value));
            value
        }
        Ok(Err(err)) => {
            println!("FAIL {feature}.{method}: {err:#}");
            Value::Null
        }
        Err(_) => {
            println!("FAIL {feature}.{method}: no answer");
            Value::Null
        }
    }
}

/// Reads a stream until it ends, [`READ_LIMIT`] or `patience`, returning how much arrived.
async fn read_for(patience: Duration, session: &Session, feature: &str, method: &str, body: Value) -> usize {
    let received = AtomicUsize::new(0);
    let reading = async {
        let mut stream = session.open(feature, method, body).await?;
        let mut buffer = vec![0; 64 * 1024];
        while received.load(Ordering::Relaxed) < READ_LIMIT {
            match stream.recv.read(&mut buffer).await? {
                Some(count) => received.fetch_add(count, Ordering::Relaxed),
                None => break,
            };
        }
        anyhow::Ok(())
    };
    let outcome = timeout(patience, reading).await;
    let total = received.load(Ordering::Relaxed);
    match outcome {
        Ok(Err(err)) => println!("FAIL {feature}.{method} stream: {err:#}"),
        _ if total == 0 => println!("FAIL {feature}.{method} stream: nothing arrived"),
        _ => println!("ok   {feature}.{method} stream: {total} bytes"),
    }
    total
}

async fn read(session: &Session, feature: &str, method: &str, body: Value) -> usize {
    read_for(WAIT, session, feature, method, body).await
}

async fn write(session: &Session, feature: &str, method: &str, body: Value, bytes: &[u8]) -> Result<()> {
    let mut stream = session.open(feature, method, body).await?;
    stream.send.write_all(bytes).await?;
    stream.send.finish()?;
    // The other side ending its half says the bytes were taken in.
    let mut buffer = [0; 16];
    while stream.recv.read(&mut buffer).await?.is_some() {}
    Ok(())
}

async fn ensure_engine(session: &Session) -> Result<()> {
    let status = call(session, "screen", "status", json!({})).await;
    if status["running"] == json!(true) {
        return Ok(());
    }
    let launch = status["launch"].as_str().context("no launch command")?;
    if !Command::new("adb").args(["shell", launch]).status()?.success() {
        bail!("could not start the engine");
    }
    sleep(Duration::from_secs(2)).await;
    let status = call(session, "screen", "status", json!({})).await;
    if status["running"] != json!(true) {
        bail!("the engine did not start");
    }
    Ok(())
}

async fn round_trips(session: &Session) {
    let mut times = Vec::new();
    for _ in 0..20 {
        let started = Instant::now();
        if session.call("system", "battery", json!({})).await.is_ok() {
            times.push(started.elapsed());
        }
        sleep(Duration::from_millis(50)).await;
    }
    times.sort();
    println!("round trip: best {:?}, median {:?}, worst {:?}", times[0], times[times.len() / 2], times[times.len() - 1]);
}

async fn screen(session: &Session) -> Result<()> {
    let options = ["raw_stream=true", "video=true", "audio=false", "control=true", "video_bit_rate=2000000"];
    let capture = call(session, "screen", "start", json!({ "options": options, "channels": 2 })).await;
    if capture.is_null() {
        return Ok(());
    }
    let mut control = session.open("screen", "attach", json!({ "session": capture, "channel": 1 })).await?;
    let text = b"Copied by the Orither probe";
    let mut message = vec![SET_CLIPBOARD];
    message.extend_from_slice(&0u64.to_be_bytes());
    message.push(0);
    message.extend_from_slice(&(text.len() as u32).to_be_bytes());
    message.extend_from_slice(text);
    control.send.write_all(&message).await?;
    println!("ok   screen control: clipboard message sent");

    let video = json!({ "session": capture, "channel": 0 });
    read_for(Duration::from_secs(4), session, "screen", "attach", video).await;
    call(session, "screen", "stop", json!({ "session": capture })).await;
    Ok(())
}

async fn files(session: &Session) -> Result<()> {
    let roots = call(session, "files", "roots", json!({})).await;
    let root = roots[0]["path"].as_str().context("no storage")?;
    call(session, "files", "list", json!({ "path": root })).await;

    let folder = format!("{root}/Download");
    write(session, "files", "write", json!({ "folder": folder, "name": PROBE_FILE }), PROBE_TEXT).await?;
    let listed = call(session, "files", "list", json!({ "path": folder })).await;
    let path = format!("{folder}/{PROBE_FILE}");
    let found = listed.as_array().is_some_and(|all| all.iter().any(|file| file["path"] == json!(path)));
    println!("{} files.write left the file there", if found { "ok  " } else { "FAIL" });
    let total = read(session, "files", "read", json!({ "path": path })).await;
    println!("{} files.read length matches", if total == PROBE_TEXT.len() { "ok  " } else { "FAIL" });
    call(session, "files", "delete", json!({ "path": path })).await;
    Ok(())
}

async fn walk(session: &Session) -> Result<()> {
    println!("route {:?}", session.route());
    ensure_engine(session).await?;
    round_trips(session).await;

    call(session, "system", "battery", json!({})).await;
    call(session, "system", "about", json!({})).await;
    read(session, "system", "wallpaper", json!({})).await;

    let threads = call(session, "sms", "threads", json!({})).await;
    if let Some(thread) = threads[0].get("id") {
        call(session, "sms", "messages", json!({ "thread": thread })).await;
    }

    let chats = call(session, "whatsapp", "chats", json!({})).await;
    if let Some(chat) = chats["chats"][0].get("id") {
        call(session, "whatsapp", "messages", json!({ "chat": chat })).await;
        read(session, "whatsapp", "avatar", json!({ "chat": chat })).await;
    }

    call(session, "gallery", "albums", json!({})).await;
    let items = call(session, "gallery", "items", json!({ "offset": 0, "limit": 30 })).await;
    if let Some(item) = items[0].get("id") {
        read(session, "gallery", "thumbnail", json!({ "id": item, "size": 256 })).await;
        read(session, "gallery", "original", json!({ "id": item })).await;
    }

    files(session).await?;

    let apps = call(session, "apps", "list", json!({})).await;
    if let Some(app) = apps[0].get("package") {
        read(session, "apps", "icon", json!({ "package": app })).await;
    }
    call(session, "apps", "open", json!({ "package": "dev.orither" })).await;

    session.notify("clipboard", "follow", json!({ "on": true }))?;
    sleep(Duration::from_secs(2)).await;
    session.notify("clipboard", "set", json!({ "text": "Set by the Orither probe" }))?;
    sleep(Duration::from_secs(1)).await;
    screen(session).await?;
    sleep(Duration::from_secs(2)).await;
    session.notify("clipboard", "follow", json!({ "on": false }))?;

    let mut speaker = session.open("speaker", "play", json!({})).await?;
    speaker.send.write_all(&vec![0; 48_000 * 4]).await?;
    speaker.send.finish()?;
    println!("ok   speaker: one second of silence sent");
    println!("route {:?}", session.route());
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let data_dir = PathBuf::from(env::args().nth(1).context("missing data dir")?);
    let device = Device { name: "Capture".into(), kind: DeviceKind::Desktop };
    let (node, mut events) = Node::start(NodeConfig { data_dir, device, offline: false }).await?;
    println!("id {}", node.id());

    let peer = timeout(Duration::from_secs(60), async {
        loop {
            match events.recv().await.context("node stopped")? {
                Event::Connected(peer) => break anyhow::Ok(peer),
                _ => continue,
            }
        }
    })
    .await
    .context("the phone never connected")??;
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            match event {
                Event::Notice { feature, method, body, .. } if body["error"].is_string() => {
                    println!("FAIL {feature}.{method}: {}", body["error"]);
                }
                Event::Notice { feature, method, body, .. } => println!("note {feature}.{method} {}", shape(&body)),
                Event::Route { route, .. } => println!("route now {route:?}"),
                _ => {}
            }
        }
    });

    let session = node.session(&peer.id).context("no session")?;
    let outcome = walk(&session).await;
    node.shutdown().await;
    outcome
}
