//! The state behind the native functions: one node, for the life of the process.

use std::{
    collections::HashMap,
    fs::File,
    future::Future,
    os::{fd::OwnedFd, unix::net::UnixStream as StdUnixStream},
    path::PathBuf,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

use anyhow::{Context, Result, anyhow};
use jni::{
    JavaVM, jni_sig, jni_str,
    objects::{Global, JObject, JValue},
};
use orither_link::{Device, DeviceKind, Duplex, Event, Node, NodeConfig, Peer, PeerId, Responder, Route};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::{
    io::{self, AsyncWriteExt},
    net::UnixStream,
    runtime::Runtime,
    sync::mpsc,
};

static BRIDGE: OnceLock<Bridge> = OnceLock::new();

pub struct Bridge {
    pub runtime: Runtime,
    pub node: Node,
    calls: Mutex<HashMap<u64, Responder>>,
    streams: Mutex<HashMap<u64, Duplex>>,
    next_handle: AtomicU64,
    kotlin: std::sync::mpsc::Sender<Delivery>,
}

/// Something on its way to Kotlin.
enum Delivery {
    Event(String),
    Reply { to: Global<JObject<'static>>, result: Result<String, String> },
}

/// What Kotlin hears about. Calls and streams carry a handle to answer them with.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Update<'a> {
    Paired { peer: &'a Peer },
    Unpaired { peer: String },
    Connected { peer: &'a Peer },
    Disconnected { peer: String },
    Route { peer: String, route: Route },
    Call { handle: u64, peer: String, feature: &'a str, method: &'a str, body: &'a Value },
    Notice { peer: String, feature: &'a str, method: &'a str, body: &'a Value },
    Stream { handle: u64, peer: String, feature: &'a str, method: &'a str, body: &'a Value },
}

impl Bridge {
    pub fn get() -> Result<&'static Self> {
        BRIDGE.get().context("the link has not been started")
    }

    /// Brings the node online once. Starting again returns the running node, so
    /// a restarted Android service picks up where it left off.
    pub fn start(
        vm: JavaVM,
        listener: Global<JObject<'static>>,
        data_dir: PathBuf,
        name: String,
    ) -> Result<&'static Self> {
        if let Some(bridge) = BRIDGE.get() {
            return Ok(bridge);
        }
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("orither-link")
            .build()
            .context("starting the async runtime")?;
        let device = Device { name, kind: DeviceKind::Phone };
        let config = NodeConfig { data_dir, device, offline: false };
        let (node, events) = runtime.block_on(Node::start(config))?;

        let (kotlin, deliveries) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("orither-kotlin".into())
            .spawn(move || deliver(&vm, &listener, &deliveries))
            .context("starting the delivery thread")?;

        let bridge = BRIDGE.get_or_init(|| Self {
            runtime,
            node,
            calls: Mutex::default(),
            streams: Mutex::default(),
            next_handle: AtomicU64::new(1),
            kotlin,
        });
        bridge.runtime.spawn(bridge.forward(events));
        Ok(bridge)
    }

    /// Runs `work` in the background and sends its outcome to the `reply` object.
    pub fn answer<F>(&'static self, reply: Global<JObject<'static>>, work: F)
    where
        F: Future<Output = Result<Value>> + Send + 'static,
    {
        self.runtime.spawn(async move {
            let result = work.await.map(|value| value.to_string()).map_err(|err| format!("{err:#}"));
            let _ = self.kotlin.send(Delivery::Reply { to: reply, result });
        });
    }

    pub fn respond(&self, handle: u64, result: Result<Value, String>) {
        let Some(responder) = self.calls.lock().unwrap().remove(&handle) else {
            return;
        };
        match result {
            Ok(body) => responder.ok(body),
            Err(error) => responder.err(error),
        }
    }

    /// Takes hold of a stream until Kotlin decides what to attach it to.
    pub fn keep_stream(&self, stream: Duplex) -> u64 {
        let handle = self.next_handle();
        self.streams.lock().unwrap().insert(handle, stream);
        handle
    }

    fn take_stream(&self, handle: u64) -> Result<Duplex> {
        self.streams.lock().unwrap().remove(&handle).ok_or_else(|| anyhow!("unknown stream {handle}"))
    }

    /// Ends a stream nobody will use.
    pub fn drop_stream(&self, handle: u64) {
        self.streams.lock().unwrap().remove(&handle);
    }

    /// Connects a stream to a socket of any stream type: this only ever reads,
    /// writes and shuts it down, which is the same for every family.
    pub fn pump(&'static self, handle: u64, socket: OwnedFd) -> Result<()> {
        let stream = self.take_stream(handle)?;
        let socket = StdUnixStream::from(socket);
        socket.set_nonblocking(true)?;
        self.runtime.spawn(async move {
            let socket = UnixStream::from_std(socket)?;
            let (reader, writer) = socket.into_split();
            stream.bridge(reader, writer).await
        });
        Ok(())
    }

    /// Offers a connected socket, such as a Bluetooth link, as a way to reach a
    /// paired device. The reply holds that device's id.
    pub fn add_nearby(&'static self, socket: OwnedFd, reply: Global<JObject<'static>>) -> Result<()> {
        let socket = StdUnixStream::from(socket);
        socket.set_nonblocking(true)?;
        self.answer(reply, async move {
            let peer = self.node.add_nearby(UnixStream::from_std(socket)?).await?;
            Ok(json!(peer.to_string()))
        });
        Ok(())
    }

    /// Sends everything left in `file`, then ends the stream.
    pub fn send_file(&'static self, handle: u64, file: OwnedFd) -> Result<()> {
        let mut stream = self.take_stream(handle)?;
        self.runtime.spawn(async move {
            let mut file = tokio::fs::File::from_std(File::from(file));
            io::copy(&mut file, &mut stream.send).await?;
            stream.send.finish()?;
            stream.send.stopped().await?;
            anyhow::Ok(())
        });
        Ok(())
    }

    /// Writes everything the other device sends into `file`.
    pub fn receive_file(&'static self, handle: u64, file: OwnedFd, reply: Global<JObject<'static>>) -> Result<()> {
        let mut stream = self.take_stream(handle)?;
        self.answer(reply, async move {
            let mut file = tokio::fs::File::from_std(File::from(file));
            let written = io::copy(&mut stream.recv, &mut file).await?;
            file.flush().await?;
            Ok(json!(written))
        });
        Ok(())
    }

    pub fn peer(text: &str) -> Result<PeerId> {
        text.parse().context("bad device id")
    }

    fn next_handle(&self) -> u64 {
        self.next_handle.fetch_add(1, Ordering::Relaxed)
    }

    async fn forward(&'static self, mut events: mpsc::UnboundedReceiver<Event>) {
        while let Some(event) = events.recv().await {
            let text = match event {
                Event::Paired(peer) => to_text(&Update::Paired { peer: &peer }),
                Event::Unpaired(id) => to_text(&Update::Unpaired { peer: id.to_string() }),
                Event::Connected(peer) => to_text(&Update::Connected { peer: &peer }),
                Event::Disconnected(id) => to_text(&Update::Disconnected { peer: id.to_string() }),
                Event::Route { peer, route } => to_text(&Update::Route { peer: peer.to_string(), route }),
                Event::Notice { peer, feature, method, body } => to_text(&Update::Notice {
                    peer: peer.to_string(),
                    feature: &feature,
                    method: &method,
                    body: &body,
                }),
                Event::Call(call) => {
                    let handle = self.next_handle();
                    self.calls.lock().unwrap().insert(handle, call.responder);
                    to_text(&Update::Call {
                        handle,
                        peer: call.peer.to_string(),
                        feature: &call.feature,
                        method: &call.method,
                        body: &call.body,
                    })
                }
                Event::Stream(incoming) => {
                    let handle = self.keep_stream(incoming.stream);
                    to_text(&Update::Stream {
                        handle,
                        peer: incoming.peer.to_string(),
                        feature: &incoming.feature,
                        method: &incoming.method,
                        body: &incoming.body,
                    })
                }
            };
            let _ = self.kotlin.send(Delivery::Event(text));
        }
    }
}

fn to_text(update: &Update<'_>) -> String {
    serde_json::to_string(update).expect("updates always serialize")
}

fn deliver(
    vm: &JavaVM,
    listener: &Global<JObject<'static>>,
    deliveries: &std::sync::mpsc::Receiver<Delivery>,
) {
    let attached = vm.attach_current_thread(|env| -> jni::errors::Result<()> {
        while let Ok(delivery) = deliveries.recv() {
            let outcome = env.with_local_frame(16, |env| -> jni::errors::Result<()> {
                match delivery {
                    Delivery::Event(text) => {
                        let text = env.new_string(text)?;
                        env.call_method(
                            listener,
                            jni_str!("onEvent"),
                            jni_sig!("(Ljava/lang/String;)V"),
                            &[JValue::Object(&text)],
                        )?;
                    }
                    Delivery::Reply { to, result } => {
                        let (ok, error) = match result {
                            Ok(value) => (env.new_string(value)?.into(), JObject::null()),
                            Err(error) => (JObject::null(), env.new_string(error)?.into()),
                        };
                        env.call_method(
                            &to,
                            jni_str!("onReply"),
                            jni_sig!("(Ljava/lang/String;Ljava/lang/String;)V"),
                            &[JValue::Object(&ok), JValue::Object(&error)],
                        )?;
                    }
                }
                Ok(())
            });
            if let Err(err) = outcome {
                tracing::error!("calling into Kotlin: {err}");
                if env.exception_check() {
                    env.exception_clear();
                }
            }
        }
        Ok(())
    });
    if let Err(err) = attached {
        tracing::error!("attaching to the JVM: {err}");
    }
}
