//! A live connection to one paired device.

use std::{
    collections::HashMap,
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use anyhow::{Context, Result, anyhow};
use iroh::endpoint::{Connection, ConnectionError, PathList, RecvStream, SendStream, VarInt};
use n0_future::StreamExt;
use serde_json::Value;
use tokio::{
    io::{AsyncRead, AsyncWrite, AsyncWriteExt, copy},
    sync::{mpsc, oneshot},
};

use crate::{
    frame::{self, Frame, StreamHeader},
    node::{Event, IncomingCall, IncomingStream, Responder},
    route::Route,
    store::PeerId,
};

/// The other device answered a call with an error.
#[derive(Debug)]
pub struct RemoteError(pub String);

impl fmt::Display for RemoteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RemoteError {}

/// A byte stream to the other device.
pub struct Duplex {
    pub send: SendStream,
    pub recv: RecvStream,
}

impl Duplex {
    /// Pumps bytes both ways between this stream and local I/O until both ends
    /// are done, passing each end-of-file along.
    pub async fn bridge(
        mut self,
        mut reader: impl AsyncRead + Unpin,
        mut writer: impl AsyncWrite + Unpin,
    ) -> Result<()> {
        let outgoing = async {
            copy(&mut reader, &mut self.send).await?;
            self.send.finish()?;
            anyhow::Ok(())
        };
        let incoming = async {
            copy(&mut self.recv, &mut writer).await?;
            writer.shutdown().await?;
            anyhow::Ok(())
        };
        let (sent, received) = tokio::join!(outgoing, incoming);
        sent.and(received)
    }
}

fn selected_route(paths: &PathList<'_>) -> Route {
    let selected = paths.iter().find(|path| path.is_selected());
    selected.map_or(Route::Relay, |path| Route::of_transport(path.remote_addr()))
}

type Waiters = Mutex<HashMap<u64, oneshot::Sender<Result<Value, RemoteError>>>>;

#[derive(Clone)]
pub struct Session {
    inner: Arc<Inner>,
}

struct Inner {
    peer: PeerId,
    conn: Connection,
    outbox: mpsc::UnboundedSender<Frame>,
    waiters: Waiters,
    next_id: AtomicU64,
}

impl Session {
    pub fn peer(&self) -> PeerId {
        self.inner.peer
    }

    /// Sends a request and waits for the answer.
    pub async fn call(&self, feature: &str, method: &str, body: Value) -> Result<Value> {
        let (feature, method) = (feature.to_owned(), method.to_owned());
        self.exchange(move |id| Frame::Req { id, f: feature, m: method, b: body }).await
    }

    /// Checks that the other device is really still there.
    pub(crate) async fn ping(&self) -> Result<()> {
        self.exchange(|id| Frame::Ping { id }).await.map(drop)
    }

    async fn exchange(&self, request: impl FnOnce(u64) -> Frame) -> Result<Value> {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.inner.waiters.lock().unwrap().insert(id, tx);
        self.post(request(id))?;
        match rx.await {
            Ok(result) => Ok(result?),
            Err(_) => Err(anyhow!("connection lost")),
        }
    }

    /// Sends a one-way message.
    pub fn notify(&self, feature: &str, method: &str, body: Value) -> Result<()> {
        self.post(Frame::Evt { f: feature.into(), m: method.into(), b: body })
    }

    /// Opens a byte stream handled by the other device's `feature`.
    pub async fn open(&self, feature: &str, method: &str, body: Value) -> Result<Duplex> {
        let (mut send, recv) = self.inner.conn.open_bi().await.context("opening stream")?;
        let header = StreamHeader { f: feature.into(), m: method.into(), b: body };
        frame::write(&mut send, &header).await.context("writing stream header")?;
        Ok(Duplex { send, recv })
    }

    pub fn is_alive(&self) -> bool {
        self.inner.conn.close_reason().is_none()
    }

    /// How data reaches the other device right now.
    pub fn route(&self) -> Route {
        selected_route(&self.inner.conn.paths())
    }

    /// Reports each change of [`Self::route`] until the connection ends.
    pub(crate) async fn report_routes(&self, events: mpsc::UnboundedSender<Event>) {
        let mut reported = None;
        let mut paths = self.inner.conn.paths_stream();
        while let Some(paths) = paths.next().await {
            let route = selected_route(&paths);
            if reported.replace(route) != Some(route) {
                let _ = events.send(Event::Route { peer: self.inner.peer, route });
            }
        }
    }

    pub(crate) fn close(&self, code: VarInt, reason: &'static str) {
        self.inner.conn.close(code, reason.as_bytes());
    }

    pub(crate) async fn closed(&self) -> ConnectionError {
        self.inner.conn.closed().await
    }

    fn post(&self, frame: Frame) -> Result<()> {
        self.inner.outbox.send(frame).map_err(|_| anyhow!("connection lost"))
    }

    /// Starts serving an already introduced connection.
    pub(crate) fn spawn(
        peer: PeerId,
        conn: Connection,
        mut send: SendStream,
        mut recv: RecvStream,
        events: mpsc::UnboundedSender<Event>,
    ) -> Self {
        let (outbox, mut outgoing) = mpsc::unbounded_channel();
        let session = Self {
            inner: Arc::new(Inner {
                peer,
                conn,
                outbox,
                waiters: Mutex::new(HashMap::new()),
                next_id: AtomicU64::new(1),
            }),
        };

        tokio::spawn(async move {
            while let Some(frame) = outgoing.recv().await {
                if frame::write(&mut send, &frame).await.is_err() {
                    break;
                }
            }
        });

        let reader = session.clone();
        let reader_events = events.clone();
        tokio::spawn(async move {
            while let Ok(frame) = frame::read::<Frame>(&mut recv).await {
                reader.dispatch(frame, &reader_events);
            }
            reader.inner.waiters.lock().unwrap().clear();
            reader.close(0u32.into(), "control stream ended");
        });

        let acceptor = session.clone();
        tokio::spawn(async move {
            while let Ok((send, mut recv)) = acceptor.inner.conn.accept_bi().await {
                let events = events.clone();
                tokio::spawn(async move {
                    if let Ok(header) = frame::read::<StreamHeader>(&mut recv).await {
                        let _ = events.send(Event::Stream(IncomingStream {
                            peer,
                            feature: header.f,
                            method: header.m,
                            body: header.b,
                            stream: Duplex { send, recv },
                        }));
                    }
                });
            }
        });

        session
    }

    fn dispatch(&self, frame: Frame, events: &mpsc::UnboundedSender<Event>) {
        let peer = self.inner.peer;
        let event = match frame {
            Frame::Req { id, f, m, b } => Event::Call(IncomingCall {
                peer,
                feature: f,
                method: m,
                body: b,
                responder: Responder::new(id, self.inner.outbox.clone()),
            }),
            Frame::Evt { f, m, b } => Event::Notice { peer, feature: f, method: m, body: b },
            Frame::Res { id, b } => return self.settle(id, Ok(b)),
            Frame::Err { id, e } => return self.settle(id, Err(RemoteError(e))),
            Frame::Ping { id } => {
                let _ = self.post(Frame::Pong { id });
                return;
            }
            Frame::Pong { id } => return self.settle(id, Ok(Value::Null)),
            Frame::Hello { .. } => return,
        };
        let _ = events.send(event);
    }

    fn settle(&self, id: u64, result: Result<Value, RemoteError>) {
        if let Some(waiter) = self.inner.waiters.lock().unwrap().remove(&id) {
            let _ = waiter.send(result);
        }
    }
}
