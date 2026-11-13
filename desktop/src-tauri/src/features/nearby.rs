//! Reaches the phone over Bluetooth when no local network connects the two. The
//! computer makes the call because a phone takes it without asking anyone, while
//! a computer would stop to ask its user. So the two are never paired over
//! Bluetooth, and the link on top keeps the connection encrypted by itself.

#[cfg(target_os = "linux")]
#[path = "nearby/linux.rs"]
mod bluetooth;
#[cfg(not(target_os = "linux"))]
#[path = "nearby/unsupported.rs"]
mod bluetooth;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::Result;
use orither_link::{PeerId, Route};
use serde_json::Value;
use tokio::{sync::Notify, time::timeout};

use super::{Feature, Phone};
use crate::preferences;

/// Calling a phone that is out of range keeps the radio busy for seconds.
const RECHECK: Duration = Duration::from_secs(120);
const ADDRESSES: &str = "nearby-phones";

pub struct Nearby {
    phone: Phone,
    addresses: Mutex<HashMap<String, String>>,
    wake: Notify,
}

impl Nearby {
    pub fn start(phone: Phone) -> Arc<dyn Feature> {
        let addresses = preferences::read(&phone.app, ADDRESSES).unwrap_or_default();
        let nearby = Arc::new(Self { phone, addresses: Mutex::new(addresses), wake: Notify::new() });
        tauri::async_runtime::spawn(nearby.clone().keep_calling());
        nearby
    }

    async fn keep_calling(self: Arc<Self>) {
        loop {
            for address in self.worth_calling() {
                match self.call(&address).await {
                    Ok(peer) => tracing::info!("Bluetooth link to {}", peer.fmt_short()),
                    Err(err) => tracing::debug!("no Bluetooth link: {err:#}"),
                }
            }
            let _ = timeout(RECHECK, self.wake.notified()).await;
        }
    }

    /// The phones with no Bluetooth link yet, unless a local network does better.
    fn worth_calling(&self) -> Vec<String> {
        let node = &self.phone.node;
        let peers = node.peers();
        if peers.iter().any(|status| status.route == Some(Route::Local)) {
            return Vec::new();
        }
        let addresses = self.addresses.lock().unwrap();
        let missing = peers.iter().filter(|status| !node.has_nearby(&status.peer.id));
        missing.filter_map(|status| addresses.get(&status.peer.id.to_string()).cloned()).collect()
    }

    async fn call(&self, address: &str) -> Result<PeerId> {
        self.phone.node.add_nearby(bluetooth::dial(address).await?).await
    }
}

#[async_trait::async_trait]
impl Feature for Nearby {
    fn id(&self) -> &'static str {
        "nearby"
    }

    fn disconnected(&self, _peer: &PeerId) {
        self.wake.notify_one();
    }

    fn notice(&self, peer: &PeerId, method: &str, body: &Value) -> bool {
        let Some(address) = body["address"].as_str().filter(|_| method == "address") else {
            return false;
        };
        let mut addresses = self.addresses.lock().unwrap();
        if addresses.insert(peer.to_string(), address.to_owned()).as_deref() != Some(address) {
            preferences::write(&self.phone.app, ADDRESSES, &*addresses);
        }
        self.wake.notify_one();
        true
    }
}
