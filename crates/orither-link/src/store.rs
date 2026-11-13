//! Everything that must survive a restart: this device's key and its peers.

use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
    sync::Mutex,
};

use anyhow::{Context, Result};
use iroh::{EndpointId, SecretKey};
use serde::{Deserialize, Serialize};

pub type PeerId = EndpointId;

const KEY_FILE: &str = "identity.key";
const PEERS_FILE: &str = "peers.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceKind {
    Phone,
    Desktop,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub name: String,
    pub kind: DeviceKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Peer {
    pub id: PeerId,
    pub device: Device,
    /// Unix seconds.
    pub paired_at: u64,
}

/// Paired peers, mirrored to disk on every change.
pub(crate) struct Store {
    dir: PathBuf,
    peers: Mutex<BTreeMap<PeerId, Peer>>,
}

impl Store {
    pub fn open(dir: &Path) -> Result<Self> {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        let peers: Vec<Peer> = match fs::read(dir.join(PEERS_FILE)) {
            Ok(bytes) => serde_json::from_slice(&bytes).context("reading paired devices")?,
            Err(err) if err.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(err) => return Err(err).context("reading paired devices"),
        };
        Ok(Self {
            dir: dir.to_owned(),
            peers: Mutex::new(peers.into_iter().map(|peer| (peer.id, peer)).collect()),
        })
    }

    /// Loads this device's key, creating it the first time.
    pub fn secret_key(&self) -> Result<SecretKey> {
        let path = self.dir.join(KEY_FILE);
        match fs::read(&path) {
            Ok(bytes) => {
                let bytes: [u8; 32] = bytes
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("{} is corrupted", path.display()))?;
                Ok(SecretKey::from_bytes(&bytes))
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                let key = SecretKey::generate();
                write_private(&path, &key.to_bytes())?;
                Ok(key)
            }
            Err(err) => Err(err).context("reading device key"),
        }
    }

    pub fn peers(&self) -> Vec<Peer> {
        self.peers.lock().unwrap().values().cloned().collect()
    }

    pub fn get(&self, id: &PeerId) -> Option<Peer> {
        self.peers.lock().unwrap().get(id).cloned()
    }

    pub fn put(&self, peer: Peer) -> Result<()> {
        let mut peers = self.peers.lock().unwrap();
        peers.insert(peer.id, peer);
        self.save(&peers)
    }

    pub fn remove(&self, id: &PeerId) -> Result<bool> {
        let mut peers = self.peers.lock().unwrap();
        let removed = peers.remove(id).is_some();
        self.save(&peers)?;
        Ok(removed)
    }

    fn save(&self, peers: &BTreeMap<PeerId, Peer>) -> Result<()> {
        let list: Vec<&Peer> = peers.values().collect();
        write_private(&self.dir.join(PEERS_FILE), &serde_json::to_vec_pretty(&list)?)
    }
}

/// Writes through a temporary file so a crash never leaves a torn file behind.
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
    }
    fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))
}
