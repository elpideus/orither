//! Bluetooth has only been written for Linux so far.

use anyhow::{Result, bail};
use tokio::io::DuplexStream;

pub async fn dial(_address: &str) -> Result<DuplexStream> {
    bail!("Bluetooth links are not available on this system yet")
}
