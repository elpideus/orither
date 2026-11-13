//! Other systems need their own way to inject input, which Orither has not got yet.

use orither_link::PeerId;
use serde_json::{Value, json};

use super::Input;
use crate::features::Phone;

const UNSUPPORTED: &str = "Only Linux computers can be controlled from the phone so far.";

pub struct Remote;

impl Remote {
    pub fn start(_phone: Phone) -> Self {
        Self
    }

    pub fn state(&self) -> Value {
        json!({ "supported": false, "active": false })
    }

    pub async fn open(&self, _ask: bool) -> Result<(), String> {
        Err(UNSUPPORTED.into())
    }

    pub fn send(&self, _peer: PeerId, _input: Input) {}

    pub fn release(&self, _peer: &PeerId) {}

    pub fn stop(&self) {}
}
