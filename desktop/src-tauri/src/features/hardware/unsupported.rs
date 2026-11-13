//! Other systems need a signed virtual device driver, which Orither has not got.

use anyhow::{Result, bail};
use orither_link::Session;

use crate::features::Phone;

pub fn blocked(_id: &str) -> Option<String> {
    Some("Not available on this system yet. It needs a device driver that Orither does not have so far.".into())
}

pub fn remove_devices() {}

pub fn can_set_up(_id: &str) -> bool {
    false
}

pub async fn set_up(_id: &str) -> Result<()> {
    bail!("not available on this system")
}

pub async fn run(_id: &str, _phone: &Phone, _session: Session) -> Result<()> {
    bail!("not available on this system")
}
