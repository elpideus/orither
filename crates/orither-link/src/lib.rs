//! The encrypted link between an Orither phone and desktop.
//!
//! This crate knows nothing about SMS, screens or clipboards. It gives both
//! apps one small vocabulary and every feature is built on top of it:
//!
//! - [`Session::call`]: ask the other device something and await the answer.
//! - [`Session::notify`]: tell the other device something, without an answer.
//! - [`Session::open`]: open a raw byte stream for files, video or audio.
//!
//! Each of these is addressed by a `feature` and a `method`, so a new feature
//! needs no change here. [`Node`] owns the rest: identity, pairing, staying
//! connected across networks, and reporting what arrives as [`Event`]s.

mod frame;
mod nearby;
mod node;
mod pairing;
mod route;
mod session;
mod store;
pub mod video;

#[cfg(target_os = "android")]
pub use iroh::dns::install_android_jni_context;
pub use iroh::endpoint::{RecvStream, SendStream};
pub use node::{Event, IncomingCall, IncomingStream, Node, NodeConfig, PeerStatus, Responder};
pub use pairing::Invite;
pub use route::Route;
pub use session::{Duplex, RemoteError, Session};
pub use store::{Device, DeviceKind, Peer, PeerId};
