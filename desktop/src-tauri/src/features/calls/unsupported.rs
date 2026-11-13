//! Taking calls through the computer has only been written for Linux so far.

use anyhow::{Result, bail};
use tokio::sync::mpsc::UnboundedSender;

use super::Facts;

const UNSUPPORTED: &str = "Calls through this computer only work on Linux so far";

pub fn facts(_address: Option<&str>) -> Facts {
    Facts { unsupported: Some(UNSUPPORTED.into()), ..Facts::default() }
}

pub async fn pair(_address: &str) -> Result<()> {
    bail!(UNSUPPORTED)
}

pub fn connect(_address: &str) -> Result<()> {
    bail!(UNSUPPORTED)
}

pub fn disconnect(_address: &str) -> Result<()> {
    Ok(())
}

pub fn pull(_address: &str) {}

pub fn watch(_changes: UnboundedSender<()>) {}

pub fn tidy() {}

pub struct Bridge;

pub async fn bridge(_address: &str) -> Result<Bridge> {
    Ok(Bridge)
}

pub async fn carry(_session: orither_link::Session, _id: &str) -> Result<()> {
    anyhow::bail!("call sound over the link is only built for Linux so far")
}
