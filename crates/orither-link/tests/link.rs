//! End-to-end tests over the real relay network.
//! Run with `cargo test -p orither-link -- --ignored`.

use std::{path::Path, time::Duration};

use orither_link::{Device, DeviceKind, Event, Node, NodeConfig};
use serde_json::json;
use tokio::{
    sync::mpsc::UnboundedReceiver,
    time::timeout,
};

const PATIENCE: Duration = Duration::from_secs(60);

async fn start(dir: &Path, name: &str, kind: DeviceKind) -> (Node, UnboundedReceiver<Event>) {
    let config = NodeConfig {
        data_dir: dir.to_owned(),
        device: Device { name: name.into(), kind },
        offline: false,
    };
    Node::start(config).await.expect("node starts")
}

async fn wait_for(events: &mut UnboundedReceiver<Event>, wanted: impl Fn(&Event) -> bool) -> Event {
    timeout(PATIENCE, async {
        loop {
            let event = events.recv().await.expect("event stream open");
            if wanted(&event) {
                return event;
            }
        }
    })
    .await
    .expect("event arrives in time")
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs internet access"]
async fn pairs_talks_and_survives_restart() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_test_writer()
        .try_init();

    let desktop_dir = tempfile::tempdir().unwrap();
    let phone_dir = tempfile::tempdir().unwrap();

    let (desktop, mut desktop_events) = start(desktop_dir.path(), "PC", DeviceKind::Desktop).await;
    let (phone, mut phone_events) = start(phone_dir.path(), "Phone", DeviceKind::Phone).await;

    let invite = desktop.invite();
    let peer = timeout(PATIENCE, phone.join(&invite.code)).await.unwrap().unwrap();
    assert_eq!(peer.id, desktop.id());
    assert_eq!(peer.device.name, "PC");

    wait_for(&mut desktop_events, |event| matches!(event, Event::Connected(_))).await;
    wait_for(&mut phone_events, |event| matches!(event, Event::Connected(_))).await;

    let session = desktop.session(&phone.id()).expect("desktop sees phone");

    // Calls reach the other side and answers find their way back.
    let call = tokio::spawn({
        let session = session.clone();
        async move { session.call("sms", "threads", json!({ "limit": 3 })).await }
    });
    match wait_for(&mut phone_events, |event| matches!(event, Event::Call(_))).await {
        Event::Call(call) => {
            assert_eq!((call.feature.as_str(), call.method.as_str()), ("sms", "threads"));
            call.responder.ok(json!({ "echo": call.body["limit"] }));
        }
        _ => unreachable!(),
    }
    assert_eq!(call.await.unwrap().unwrap(), json!({ "echo": 3 }));

    // Byte streams carry their header and then arbitrary bytes.
    let mut stream = session.open("files", "read", json!({ "path": "/x" })).await.unwrap();
    stream.send.write_all(b"ping").await.unwrap();
    stream.send.finish().unwrap();
    match wait_for(&mut phone_events, |event| matches!(event, Event::Stream(_))).await {
        Event::Stream(mut incoming) => {
            assert_eq!(incoming.body, json!({ "path": "/x" }));
            let got = incoming.stream.recv.read_to_end(64).await.unwrap();
            assert_eq!(got, b"ping");
            incoming.stream.send.write_all(b"pong").await.unwrap();
            incoming.stream.send.finish().unwrap();
        }
        _ => unreachable!(),
    }
    let reply = stream.recv.read_to_end(64).await.unwrap();
    assert_eq!(reply, b"pong");

    // Restarting the desktop must not lose the pairing.
    let phone_id = phone.id();
    desktop.shutdown().await;
    drop(desktop);
    wait_for(&mut phone_events, |event| matches!(event, Event::Disconnected(_))).await;

    let (desktop, mut desktop_events) = start(desktop_dir.path(), "PC", DeviceKind::Desktop).await;
    wait_for(&mut desktop_events, |event| matches!(event, Event::Connected(_))).await;
    assert!(desktop.session(&phone_id).is_some());
    assert_eq!(desktop.peers().len(), 1);

    // Unpairing reaches both sides.
    desktop.unpair(&phone_id).unwrap();
    wait_for(&mut phone_events, |event| matches!(event, Event::Unpaired(_))).await;
    assert!(phone.peers().is_empty());

    phone.shutdown().await;
    desktop.shutdown().await;
}
