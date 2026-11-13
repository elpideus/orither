//! Records the phone's screen through the link into a raw H.264 file.
//!
//! `cargo run -p orither-link --example capture -- <data dir> <output> <megabytes>`

use std::{env, path::PathBuf, process::Command, time::Instant};

use anyhow::{Context, Result, bail};
use orither_link::{Device, DeviceKind, Event, Node, NodeConfig};
use serde_json::json;
use tokio::{fs::File, io::AsyncWriteExt};

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = env::args().skip(1);
    let data_dir = PathBuf::from(args.next().context("missing data dir")?);
    let output = args.next().context("missing output path")?;
    let limit: usize = args.next().context("missing size")?.parse()?;

    let device = Device { name: "Capture".into(), kind: DeviceKind::Desktop };
    let (node, mut events) = Node::start(NodeConfig { data_dir, device, offline: false }).await?;
    println!("id {}", node.id());

    let peer = loop {
        match events.recv().await.context("node stopped")? {
            Event::Connected(peer) => break peer,
            _ => continue,
        }
    };
    let session = node.session(&peer.id).context("no session")?;

    let status = session.call("screen", "status", json!({})).await?;
    println!("engine running: {}", status["running"]);
    if status["running"] != json!(true) {
        let launch = status["launch"].as_str().context("no launch command")?;
        let done = Command::new("adb").args(["shell", launch]).status()?;
        if !done.success() {
            bail!("could not start the engine");
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }

    let options = ["raw_stream=true", "video=true", "audio=false", "control=false", "video_bit_rate=8000000"];
    let body = json!({ "options": options, "channels": 1 });
    let capture = session.call("screen", "start", body).await?;
    println!("capture {capture}");

    let mut stream = session.open("screen", "attach", json!({ "session": capture, "channel": 0 })).await?;
    let mut file = File::create(output).await?;
    let mut buffer = vec![0u8; 64 * 1024];
    let (mut total, started) = (0, Instant::now());
    while total < limit * 1024 * 1024 {
        match stream.recv.read(&mut buffer).await? {
            Some(count) => {
                file.write_all(&buffer[..count]).await?;
                total += count;
            }
            None => break,
        }
    }
    file.flush().await?;
    let seconds = started.elapsed().as_secs_f64();
    println!("{total} bytes in {seconds:.1} s, {:.1} Mbit/s", total as f64 * 8e-6 / seconds);

    session.call("screen", "stop", json!({ "session": capture })).await?;
    node.shutdown().await;
    Ok(())
}
