//! A native window for a phone's screen.
//!
//! The window runs as its own process, because a process has one identity on
//! Wayland and every Integrated Mode app needs to look like an app of its own.
//! It takes the video on stdin and writes input as control messages on stdout,
//! so it knows nothing of how the two reach the phone.

pub mod control;
mod decode;
mod input;
mod render;
mod stream;
mod window;

pub use decode::{Frame, H264, Plane};
pub use stream::{Packet, Reader};
pub use window::{Options, run};
