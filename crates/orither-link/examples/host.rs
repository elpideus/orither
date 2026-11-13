//! A bare desktop for trying the link against a real phone: prints a pairing
//! code, then answers with an echo round trip whenever the phone connects.
//!
//! `cargo run -p orither-link --example host -- <data dir>`

use std::{path::PathBuf, time::Instant};

use anyhow::{Context, Result};
use orither_link::{Device, DeviceKind, Event, Node, NodeConfig};
use serde_json::json;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
    let data_dir = PathBuf::from(std::env::args().nth(1).context("usage: host <data dir>")?);
    let device = Device { name: "Dev host".into(), kind: DeviceKind::Desktop };
    let (node, mut events) = Node::start(NodeConfig { data_dir, device, offline: false }).await?;

    println!("id: {}", node.id());
    if node.peers().is_empty() {
        println!("code: {}", node.invite().code);
    }

    while let Some(event) = events.recv().await {
        match event {
            Event::Paired(peer) => println!("paired with {}", peer.device.name),
            Event::Unpaired(id) => println!("unpaired {}", id.fmt_short()),
            Event::Disconnected(id) => println!("disconnected {}", id.fmt_short()),
            Event::Connected(peer) => {
                println!("connected to {}", peer.device.name);
                let Some(session) = node.session(&peer.id) else { continue };
                tokio::spawn(async move {
                    let started = Instant::now();
                    let answer = session.call("echo", "ping", json!({"hello": "phone"})).await;
                    println!("echo: {answer:?} in {:?}", started.elapsed());
                });
            }
            _ => {}
        }
    }
    Ok(())
}
