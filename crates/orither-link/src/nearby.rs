//! A route over any byte stream between two devices that are close to each other,
//! such as a Bluetooth socket. The platform finds and opens the stream; this only
//! moves QUIC packets over it, which keeps them encrypted and authenticated even
//! when the stream itself is neither.

use std::{
    collections::HashMap,
    io::{self, ErrorKind},
    sync::{Arc, Mutex},
    task::{Context, Poll},
};

use anyhow::{Context as _, Result};
use iroh::{
    EndpointId,
    endpoint::transports::{CustomEndpoint, CustomSender, CustomTransport, RecvInfo, Transmit},
};
use iroh_base::CustomAddr;
use n0_watcher::{Direct, Watchable};
use noq_udp::RecvMeta;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, split},
    sync::mpsc,
};

use crate::store::PeerId;

/// Tells these addresses apart from those of any other custom transport.
const TRANSPORT_ID: u64 = u64::from_le_bytes(*b"oritherN");
/// A slow link should lose packets like any network, not hoard them, or the
/// connection's congestion control could not see that it is sending too much.
const QUEUE: usize = 64;

/// The address of `peer` on a link. A link has one device at the other end, so
/// its id says all there is to say.
pub(crate) fn address(peer: &PeerId) -> CustomAddr {
    CustomAddr::from_parts(TRANSPORT_ID, peer.as_bytes())
}

/// Tells the other end of a fresh stream who this is. A liar gains nothing:
/// the connection on top still proves who it is talking to.
pub(crate) async fn introduce(stream: &mut (impl AsyncRead + AsyncWrite + Unpin), me: PeerId) -> Result<PeerId> {
    stream.write_all(me.as_bytes()).await?;
    stream.flush().await?;
    let mut theirs = [0; 32];
    stream.read_exact(&mut theirs).await?;
    EndpointId::from_bytes(&theirs).context("invalid device id")
}

struct Packet {
    from: CustomAddr,
    data: Vec<u8>,
}

type Links = Arc<Mutex<HashMap<CustomAddr, mpsc::Sender<Vec<u8>>>>>;

#[derive(Debug)]
pub(crate) struct Nearby {
    addrs: Watchable<Vec<CustomAddr>>,
    links: Links,
    arriving: mpsc::Sender<Packet>,
    inbox: Mutex<Option<mpsc::Receiver<Packet>>>,
}

impl std::fmt::Debug for Packet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} bytes from {}", self.data.len(), self.from)
    }
}

impl Nearby {
    pub(crate) fn new(me: &PeerId) -> Self {
        let (arriving, inbox) = mpsc::channel(QUEUE);
        Self {
            addrs: Watchable::new(vec![address(me)]),
            links: Links::default(),
            arriving,
            inbox: Mutex::new(Some(inbox)),
        }
    }

    pub(crate) fn has_link(&self, peer: &PeerId) -> bool {
        self.links.lock().unwrap().contains_key(&address(peer))
    }

    /// Starts carrying packets to and from `peer` over `stream`, until it ends.
    pub(crate) fn attach(&self, peer: &PeerId, stream: impl AsyncRead + AsyncWrite + Send + 'static) {
        let addr = address(peer);
        let (queue, mut outgoing) = mpsc::channel::<Vec<u8>>(QUEUE);
        self.links.lock().unwrap().insert(addr.clone(), queue.clone());

        let (links, arriving) = (self.links.clone(), self.arriving.clone());
        let (mut reader, mut writer) = split(stream);
        let from = addr.clone();
        tokio::spawn(async move {
            let sending = async {
                while let Some(frame) = outgoing.recv().await {
                    writer.write_all(&frame).await?;
                    writer.flush().await?;
                }
                io::Result::Ok(())
            };
            let receiving = async {
                loop {
                    let mut data = vec![0; usize::from(reader.read_u16().await?)];
                    reader.read_exact(&mut data).await?;
                    let _ = arriving.try_send(Packet { from: from.clone(), data });
                }
                #[allow(unreachable_code)]
                io::Result::Ok(())
            };
            let ended = tokio::select! { ended = sending => ended, ended = receiving => ended };
            tracing::info!("nearby link ended: {ended:?}");

            let mut links = links.lock().unwrap();
            if links.get(&addr).is_some_and(|current| current.same_channel(&queue)) {
                links.remove(&addr);
            }
        });
    }
}

impl CustomTransport for Nearby {
    fn bind(&self) -> io::Result<Box<dyn CustomEndpoint>> {
        let inbox = self.inbox.lock().unwrap().take();
        let inbox = inbox.ok_or_else(|| io::Error::new(ErrorKind::AddrInUse, "already bound"))?;
        Ok(Box::new(Bound { addrs: self.addrs.clone(), links: self.links.clone(), inbox }))
    }
}

#[derive(Debug)]
struct Bound {
    addrs: Watchable<Vec<CustomAddr>>,
    links: Links,
    inbox: mpsc::Receiver<Packet>,
}

impl CustomEndpoint for Bound {
    fn watch_local_addrs(&self) -> Direct<Vec<CustomAddr>> {
        self.addrs.watch()
    }

    fn create_sender(&self) -> Arc<dyn CustomSender> {
        Arc::new(Sender { links: self.links.clone() })
    }

    fn poll_recv(
        &mut self,
        context: &mut Context,
        buffers: &mut [io::IoSliceMut<'_>],
        metas: &mut [RecvMeta],
        infos: &mut [RecvInfo],
    ) -> Poll<io::Result<usize>> {
        let mut packets = Vec::new();
        if std::task::ready!(self.inbox.poll_recv_many(context, &mut packets, buffers.len())) == 0 {
            return Poll::Ready(Err(io::Error::from(ErrorKind::BrokenPipe)));
        }
        let count = packets.len();
        for (index, packet) in packets.into_iter().enumerate() {
            buffers[index][..packet.data.len()].copy_from_slice(&packet.data);
            metas[index].len = packet.data.len();
            metas[index].stride = packet.data.len();
            infos[index] = RecvInfo::new(packet.from, None);
        }
        Poll::Ready(Ok(count))
    }
}

#[derive(Debug)]
struct Sender {
    links: Links,
}

impl CustomSender for Sender {
    fn is_valid_send_addr(&self, addr: &CustomAddr) -> bool {
        addr.id() == TRANSPORT_ID
    }

    /// With no link to `destination` the packet is lost, as on any other network
    /// that went away, and the path times out by itself.
    fn poll_send(
        &self,
        _context: &mut Context,
        destination: &CustomAddr,
        _source: Option<&CustomAddr>,
        transmit: &Transmit<'_>,
    ) -> Poll<io::Result<()>> {
        if let Some(link) = self.links.lock().unwrap().get(destination) {
            let size = transmit.segment_size.unwrap_or(transmit.contents.len());
            for packet in transmit.contents.chunks(size) {
                let mut frame = Vec::with_capacity(packet.len() + 2);
                frame.extend_from_slice(&(packet.len() as u16).to_be_bytes());
                frame.extend_from_slice(packet);
                let _ = link.try_send(frame);
            }
        }
        Poll::Ready(Ok(()))
    }
}
