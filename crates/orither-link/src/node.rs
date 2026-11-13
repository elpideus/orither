//! This device on the network: keeps one session open to every paired peer.
//!
//! Within a pair, the device with the smaller id always opens the session, which
//! rules out two sessions racing each other. The other device can still bring
//! the pair back together: it rings, a throwaway connection that only asks to
//! be dialed.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow, bail, ensure};
use iroh::{
    Endpoint, EndpointAddr, TransportAddr,
    endpoint::{
        Connection, ConnectionError, Incoming, QuicTransportConfig, RecvStream, SendStream, VarInt,
        presets,
    },
};
use iroh_mdns_address_lookup::MdnsAddressLookup;
use serde::Serialize;
use serde_json::Value;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::{Notify, mpsc, oneshot},
    task::AbortHandle,
    time::{sleep, timeout},
};

use crate::{
    frame::{self, Frame, PROTOCOL},
    nearby::{self, Nearby},
    pairing::{self, Invite},
    route::{Route, RoutePriority},
    session::{Duplex, Session},
    store::{Device, Peer, PeerId, Store},
};

const ALPN: &[u8] = b"orither/link/1";
const RING_ALPN: &[u8] = b"orither/ring/1";
/// A connection that only exists to add a path, which new connections do by
/// themselves. Once selected, the path spreads to the session.
const PATH_ALPN: &[u8] = b"orither/path/1";
const KEEP_ALIVE: Duration = Duration::from_secs(10);
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const DIAL_TIMEOUT: Duration = Duration::from_secs(20);
const RING_PATIENCE: Duration = Duration::from_secs(5);
const PING_PATIENCE: Duration = Duration::from_secs(2);
const PATH_PATIENCE: Duration = Duration::from_secs(3);
const NEARBY_CHECK: Duration = Duration::from_secs(10);
const RETRY_STEPS: [u64; 6] = [1, 2, 5, 10, 30, 60];
const MAX_STREAMS: u32 = 4096;
/// Sent when closing a connection from a device we are not paired with. Only the
/// real peer can send it, so hearing it means the pairing ended on their side,
/// even if that happened while we were offline.
const CLOSE_NOT_PAIRED: VarInt = VarInt::from_u32(1);

#[derive(Debug)]
struct Refused;

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the other device is no longer paired with this one")
    }
}

impl std::error::Error for Refused {}

fn is_refusal(error: &ConnectionError) -> bool {
    matches!(error, ConnectionError::ApplicationClosed(close) if close.error_code == CLOSE_NOT_PAIRED)
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs())
}

pub struct NodeConfig {
    /// Holds the device key and the paired device list.
    pub data_dir: PathBuf,
    pub device: Device,
    /// Stays off every network, leaving only links from [`Node::add_nearby`].
    pub offline: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct PeerStatus {
    #[serde(flatten)]
    pub peer: Peer,
    pub online: bool,
    pub route: Option<Route>,
}

/// Everything a host app reacts to.
pub enum Event {
    Paired(Peer),
    Unpaired(PeerId),
    Connected(Peer),
    Disconnected(PeerId),
    Route { peer: PeerId, route: Route },
    Call(IncomingCall),
    Notice { peer: PeerId, feature: String, method: String, body: Value },
    Stream(IncomingStream),
}

pub struct IncomingCall {
    pub peer: PeerId,
    pub feature: String,
    pub method: String,
    pub body: Value,
    pub responder: Responder,
}

pub struct IncomingStream {
    pub peer: PeerId,
    pub feature: String,
    pub method: String,
    pub body: Value,
    pub stream: Duplex,
}

/// The one-time reply to an [`IncomingCall`].
pub struct Responder {
    id: u64,
    outbox: mpsc::UnboundedSender<Frame>,
}

impl Responder {
    pub(crate) fn new(id: u64, outbox: mpsc::UnboundedSender<Frame>) -> Self {
        Self { id, outbox }
    }

    pub fn ok(self, body: Value) {
        let _ = self.outbox.send(Frame::Res { id: self.id, b: body });
    }

    pub fn err(self, error: impl ToString) {
        let _ = self.outbox.send(Frame::Err { id: self.id, e: error.to_string() });
    }
}

#[derive(Clone)]
pub struct Node {
    inner: Arc<Inner>,
}

struct Inner {
    endpoint: Endpoint,
    nearby: Arc<Nearby>,
    store: Store,
    device: Device,
    events: mpsc::UnboundedSender<Event>,
    sessions: Mutex<HashMap<PeerId, Session>>,
    dialers: Mutex<HashMap<PeerId, AbortHandle>>,
    /// Dropping this withdraws the invite that is out.
    invite: Mutex<Option<oneshot::Sender<()>>>,
    /// Interrupts retry delays.
    wake: Notify,
    /// Tells a ringing device that its session arrived.
    arrived: Notify,
}

impl Node {
    /// Brings the device online and starts reaching for every paired peer.
    pub async fn start(config: NodeConfig) -> Result<(Self, mpsc::UnboundedReceiver<Event>)> {
        let store = Store::open(&config.data_dir)?;
        let secret = store.secret_key()?;
        let id = secret.public();
        let transport = QuicTransportConfig::builder()
            .keep_alive_interval(KEEP_ALIVE)
            .max_idle_timeout(Some(IDLE_TIMEOUT.try_into().context("idle timeout")?))
            .max_concurrent_bidi_streams(VarInt::from_u32(MAX_STREAMS))
            .build();
        let nearby = Arc::new(Nearby::new(&id));
        let mut builder = Endpoint::builder(presets::N0)
            .secret_key(secret)
            .alpns(vec![ALPN.to_vec(), RING_ALPN.to_vec(), PATH_ALPN.to_vec()])
            .transport_config(transport)
            .add_custom_transport(nearby.clone())
            .path_selector(Arc::new(RoutePriority));
        if config.offline {
            builder = builder.clear_ip_transports().clear_relay_transports();
        }
        let endpoint = builder.bind().await.context("starting network endpoint")?;
        // Local discovery is a bonus that some networks forbid.
        if !config.offline {
            match MdnsAddressLookup::builder().build(id) {
                Ok(mdns) => endpoint.address_lookup()?.add(mdns),
                Err(err) => tracing::warn!("local discovery unavailable: {err}"),
            }
        }

        let (events, receiver) = mpsc::unbounded_channel();
        let node = Self {
            inner: Arc::new(Inner {
                endpoint,
                nearby,
                store,
                device: config.device,
                events,
                sessions: Mutex::new(HashMap::new()),
                dialers: Mutex::new(HashMap::new()),
                invite: Mutex::new(None),
                wake: Notify::new(),
                arrived: Notify::new(),
            }),
        };

        let acceptor = node.clone();
        tokio::spawn(async move { acceptor.accept_loop().await });
        for peer in node.inner.store.peers() {
            node.watch(peer.id);
        }
        Ok((node, receiver))
    }

    pub fn id(&self) -> PeerId {
        self.inner.endpoint.id()
    }

    pub fn device(&self) -> &Device {
        &self.inner.device
    }

    pub fn peers(&self) -> Vec<PeerStatus> {
        self.inner
            .store
            .peers()
            .into_iter()
            .map(|peer| {
                let session = self.session(&peer.id);
                PeerStatus { online: session.is_some(), route: session.map(|s| s.route()), peer }
            })
            .collect()
    }

    pub fn session(&self, peer: &PeerId) -> Option<Session> {
        self.inner.sessions.lock().unwrap().get(peer).filter(|s| s.is_alive()).cloned()
    }

    /// The only connected peer, which is nearly every install.
    pub fn any_session(&self) -> Option<Session> {
        self.inner.sessions.lock().unwrap().values().find(|s| s.is_alive()).cloned()
    }

    /// Shows up for the next `expires_in_secs` as a device that can be joined.
    pub fn invite(&self) -> Invite {
        let invite = Invite::generate();
        let tokens = invite.tokens();
        let (keep, cancelled) = oneshot::channel();
        *self.inner.invite.lock().unwrap() = Some(keep);
        let node = self.clone();
        tokio::spawn(async move {
            match pairing::host(&tokens, node.id(), &node.inner.device, node.saver(), cancelled).await {
                Ok(id) => drop(node.welcome(id)),
                Err(err) => tracing::info!("invite ended: {err:#}"),
            }
        });
        invite
    }

    pub fn cancel_invite(&self) {
        self.inner.invite.lock().unwrap().take();
    }

    /// Redeems a code or QR payload from another device.
    pub async fn join(&self, token: &str) -> Result<Peer> {
        let id = pairing::join(token, self.id(), &self.inner.device, self.saver()).await?;
        self.welcome(id)
    }

    /// Forgets a peer. It learns about this the next time the two connect.
    pub fn unpair(&self, peer: &PeerId) -> Result<()> {
        self.remove_peer(peer)
    }

    /// Adds a byte stream to a nearby device as another route to it, for as long
    /// as the stream lasts. Returns who is at the other end.
    pub async fn add_nearby(
        &self,
        mut stream: impl AsyncRead + AsyncWrite + Send + Unpin + 'static,
    ) -> Result<PeerId> {
        let peer = timeout(DIAL_TIMEOUT, nearby::introduce(&mut stream, self.id())).await.context("timed out")??;
        ensure!(self.inner.store.get(&peer).is_some(), "not paired with this device");
        self.inner.nearby.attach(&peer, stream);
        self.inner.wake.notify_waiters();
        if self.id() < peer {
            let node = self.clone();
            tokio::spawn(async move { node.keep_using_nearby(peer).await });
        }
        Ok(peer)
    }

    /// Whether a stream from [`Self::add_nearby`] still reaches `peer`.
    pub fn has_nearby(&self, peer: &PeerId) -> bool {
        self.inner.nearby.has_link(peer)
    }

    /// Call after the network changed so that peers reconnect right away.
    pub async fn network_changed(&self) {
        self.inner.endpoint.network_change().await;
        self.inner.wake.notify_waiters();
    }

    pub async fn shutdown(&self) {
        self.inner.endpoint.close().await;
    }

    fn saver(&self) -> pairing::Save {
        let node = self.clone();
        Arc::new(move |id, device| {
            let peer = Peer { id, device, paired_at: now_secs() };
            if let Err(err) = node.inner.store.put(peer) {
                tracing::error!("saving paired device: {err:#}");
            }
        })
    }

    /// Starts using a device that both sides have finished saving.
    fn welcome(&self, id: PeerId) -> Result<Peer> {
        let peer = self.inner.store.get(&id).context("paired device was not saved")?;
        self.watch(id);
        let _ = self.inner.events.send(Event::Paired(peer.clone()));
        Ok(peer)
    }

    fn remove_peer(&self, peer: &PeerId) -> Result<()> {
        if let Some(dialer) = self.inner.dialers.lock().unwrap().remove(peer) {
            dialer.abort();
        }
        if let Some(session) = self.inner.sessions.lock().unwrap().remove(peer) {
            session.close(CLOSE_NOT_PAIRED, "unpaired");
        }
        if self.inner.store.remove(peer)? {
            let _ = self.inner.events.send(Event::Unpaired(*peer));
        }
        Ok(())
    }

    /// Starts the task that keeps `peer` connected for as long as it is paired.
    fn watch(&self, peer: PeerId) {
        let mut dialers = self.inner.dialers.lock().unwrap();
        if dialers.contains_key(&peer) {
            return;
        }
        let node = self.clone();
        let task = tokio::spawn(async move { node.keep_connected(peer).await });
        dialers.insert(peer, task.abort_handle());
    }

    async fn keep_connected(self, peer: PeerId) {
        let mut failures = 0;
        while !self.inner.endpoint.is_closed() {
            if let Some(session) = self.session(&peer) {
                failures = 0;
                if is_refusal(&session.closed().await) {
                    self.forget(peer);
                    return;
                }
                continue;
            }
            let attempt = if self.id() < peer { self.dial(peer).await } else { self.ring(peer).await };
            match attempt {
                Ok(()) => failures = 0,
                Err(err) if err.is::<Refused>() => {
                    self.forget(peer);
                    return;
                }
                Err(err) => {
                    tracing::debug!("reaching {}: {err:#}", peer.fmt_short());
                    let delay = RETRY_STEPS[failures.min(RETRY_STEPS.len() - 1)];
                    failures += 1;
                    tokio::select! {
                        () = sleep(Duration::from_secs(delay)) => {}
                        () = self.inner.wake.notified() => failures = 0,
                    }
                }
            }
        }
    }

    /// Moves the session onto a nearby link whenever that beats its route. Only the
    /// side that opened a connection can add paths to it.
    async fn keep_using_nearby(self, peer: PeerId) {
        while self.inner.nearby.has_link(&peer) && !self.inner.endpoint.is_closed() {
            if self.session(&peer).is_some_and(|session| session.route() > Route::Nearby)
                && let Err(err) = self.open_nearby_path(peer).await
            {
                tracing::debug!("nearby path to {}: {err:#}", peer.fmt_short());
            }
            sleep(NEARBY_CHECK).await;
        }
    }

    async fn open_nearby_path(&self, peer: PeerId) -> Result<()> {
        let connecting = self.inner.endpoint.connect(self.address(peer), PATH_ALPN);
        let conn = timeout(DIAL_TIMEOUT, connecting).await.context("timed out")??;
        sleep(PATH_PATIENCE).await;
        conn.close(VarInt::from_u32(0), b"done");
        Ok(())
    }

    /// `peer`, along with the routes that only this device can know about.
    fn address(&self, peer: PeerId) -> EndpointAddr {
        let address = EndpointAddr::new(peer);
        if self.inner.nearby.has_link(&peer) {
            return address.with_addrs([TransportAddr::Custom(nearby::address(&peer))]);
        }
        address
    }

    fn forget(&self, peer: PeerId) {
        if let Err(err) = self.remove_peer(&peer) {
            tracing::error!("forgetting {}: {err:#}", peer.fmt_short());
        }
    }

    async fn dial(&self, peer: PeerId) -> Result<()> {
        let conn = timeout(DIAL_TIMEOUT, self.inner.endpoint.connect(self.address(peer), ALPN))
            .await
            .context("timed out")??;
        let greeted = self.greet(peer, conn.clone()).await;
        if conn.close_reason().as_ref().is_some_and(is_refusal) {
            bail!(Refused);
        }
        greeted
    }

    /// Asks the peer to dial us, then gives it a moment to do so.
    async fn ring(&self, peer: PeerId) -> Result<()> {
        let conn = timeout(DIAL_TIMEOUT, self.inner.endpoint.connect(self.address(peer), RING_ALPN))
            .await
            .context("timed out")??;
        let answer = timeout(DIAL_TIMEOUT, conn.closed()).await.context("no answer")?;
        if is_refusal(&answer) {
            bail!(Refused);
        }
        let _ = timeout(RING_PATIENCE, self.inner.arrived.notified()).await;
        Ok(())
    }

    async fn answer_ring(&self, peer: PeerId, conn: Connection) {
        conn.close(VarInt::from_u32(0), b"on my way");
        // A ring can overtake the session it asked for, so only a session that
        // stopped answering counts as a leftover from before the peer restarted.
        if let Some(session) = self.session(&peer) {
            if matches!(timeout(PING_PATIENCE, session.ping()).await, Ok(Ok(()))) {
                return;
            }
            session.close(VarInt::from_u32(0), "stale");
        }
        self.inner.wake.notify_waiters();
    }

    async fn greet(&self, peer: PeerId, conn: Connection) -> Result<()> {
        let (mut send, mut recv) = conn.open_bi().await?;
        let hello = Frame::Hello { v: PROTOCOL, device: self.inner.device.clone() };
        frame::write(&mut send, &hello).await?;
        let reply = timeout(DIAL_TIMEOUT, frame::read::<Frame>(&mut recv)).await.context("no reply")??;
        self.admit(peer, conn, send, recv, reply)
    }

    async fn accept_loop(self) {
        while let Some(incoming) = self.inner.endpoint.accept().await {
            let node = self.clone();
            tokio::spawn(async move {
                if let Err(err) = node.handle_incoming(incoming).await {
                    tracing::debug!("incoming connection: {err:#}");
                }
            });
        }
    }

    async fn handle_incoming(self, incoming: Incoming) -> Result<()> {
        let conn = incoming.accept()?.await?;
        let peer = conn.remote_id();
        if self.inner.store.get(&peer).is_none() {
            conn.close(CLOSE_NOT_PAIRED, b"not paired");
            return Ok(());
        }
        if conn.alpn() == RING_ALPN {
            self.answer_ring(peer, conn).await;
            return Ok(());
        }
        if conn.alpn() == PATH_ALPN {
            conn.closed().await;
            return Ok(());
        }
        let (mut send, mut recv) = conn.accept_bi().await?;
        let hello = timeout(DIAL_TIMEOUT, frame::read::<Frame>(&mut recv)).await.context("no hello")??;
        let reply = Frame::Hello { v: PROTOCOL, device: self.inner.device.clone() };
        frame::write(&mut send, &reply).await?;
        self.admit(peer, conn, send, recv, hello)
    }

    /// Validates the peer's hello and installs the connection as its session.
    fn admit(
        &self,
        id: PeerId,
        conn: Connection,
        send: SendStream,
        recv: RecvStream,
        hello: Frame,
    ) -> Result<()> {
        let Frame::Hello { v, device } = hello else {
            return Err(anyhow!("expected a hello"));
        };
        ensure!(v == PROTOCOL, "incompatible Orither versions");
        let Some(mut peer) = self.inner.store.get(&id) else {
            conn.close(CLOSE_NOT_PAIRED, b"not paired");
            return Err(anyhow!("peer no longer paired"));
        };
        if peer.device != device {
            peer.device = device;
            self.inner.store.put(peer.clone())?;
        }

        let session = Session::spawn(id, conn, send, recv, self.inner.events.clone());
        let (reporter, events) = (session.clone(), self.inner.events.clone());
        tokio::spawn(async move { reporter.report_routes(events).await });
        let replaced = self.inner.sessions.lock().unwrap().insert(id, session.clone());
        if let Some(old) = replaced {
            old.close(VarInt::from_u32(0), "replaced");
        }
        let _ = self.inner.events.send(Event::Connected(peer));
        self.inner.arrived.notify_waiters();

        let node = self.clone();
        tokio::spawn(async move {
            session.closed().await;
            let mut sessions = node.inner.sessions.lock().unwrap();
            let is_current = sessions.get(&id).is_some_and(|current| !current.is_alive());
            if is_current {
                sessions.remove(&id);
                drop(sessions);
                let _ = node.inner.events.send(Event::Disconnected(id));
            }
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tempfile::TempDir;
    use tokio::io::duplex;

    use super::*;
    use crate::store::DeviceKind;

    async fn offline_node(name: &str, dir: &TempDir) -> (Node, mpsc::UnboundedReceiver<Event>) {
        let device = Device { name: name.into(), kind: DeviceKind::Desktop };
        let config = NodeConfig { data_dir: dir.path().join(name), device, offline: true };
        Node::start(config).await.unwrap()
    }

    fn introduce(node: &Node, other: &Node) {
        (node.saver())(other.id(), other.device().clone());
        node.welcome(other.id()).unwrap();
    }

    async fn next_session(node: &Node, events: &mut mpsc::UnboundedReceiver<Event>) -> Session {
        loop {
            if let Some(Event::Connected(peer)) = events.recv().await {
                return node.session(&peer.id).unwrap();
            }
        }
    }

    #[tokio::test]
    async fn a_byte_stream_alone_can_carry_the_session() {
        let dir = tempfile::tempdir().unwrap();
        let (one, mut one_events) = offline_node("one", &dir).await;
        let (two, mut two_events) = offline_node("two", &dir).await;
        introduce(&one, &two);
        introduce(&two, &one);

        let (near, far) = duplex(1 << 16);
        let (joined_one, joined_two) = tokio::join!(one.add_nearby(near), two.add_nearby(far));
        assert_eq!(joined_one.unwrap(), two.id());
        assert_eq!(joined_two.unwrap(), one.id());

        let (session, _) = timeout(Duration::from_secs(30), async {
            tokio::join!(next_session(&one, &mut one_events), next_session(&two, &mut two_events))
        })
        .await
        .expect("sessions over the stream");

        tokio::spawn(async move {
            while let Some(event) = two_events.recv().await {
                if let Event::Call(call) = event {
                    call.responder.ok(call.body);
                }
            }
        });
        let answer = timeout(Duration::from_secs(10), session.call("echo", "ping", json!("hi"))).await;
        assert_eq!(answer.unwrap().unwrap(), json!("hi"));
        assert_eq!(session.route(), Route::Nearby);

        one.shutdown().await;
        two.shutdown().await;
    }

    #[tokio::test]
    async fn strangers_cannot_add_a_link() {
        let dir = tempfile::tempdir().unwrap();
        let (one, _events) = offline_node("one", &dir).await;
        let (two, _other_events) = offline_node("two", &dir).await;
        let (near, far) = duplex(1 << 16);
        let (mine, theirs) = tokio::join!(one.add_nearby(near), two.add_nearby(far));
        assert!(mine.is_err() && theirs.is_err());
        one.shutdown().await;
        two.shutdown().await;
    }
}
