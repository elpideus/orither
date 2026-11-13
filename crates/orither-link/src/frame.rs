//! Wire format: a big-endian `u32` length followed by a JSON document.

use std::io;

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::store::Device;

const MAX_FRAME: usize = 16 * 1024 * 1024;

/// Bumped only for breaking changes to this module.
pub(crate) const PROTOCOL: u32 = 1;

/// Traffic on a session's control stream.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
pub(crate) enum Frame {
    Hello { v: u32, device: Device },
    Req { id: u64, f: String, m: String, b: Value },
    Res { id: u64, b: Value },
    Err { id: u64, e: String },
    Evt { f: String, m: String, b: Value },
    Ping { id: u64 },
    Pong { id: u64 },
}

/// The first frame on every byte stream, naming what the stream carries.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct StreamHeader {
    pub f: String,
    pub m: String,
    pub b: Value,
}

pub(crate) async fn write<T: Serialize>(
    out: &mut (impl AsyncWrite + Unpin),
    frame: &T,
) -> io::Result<()> {
    let body = serde_json::to_vec(frame)?;
    if body.len() > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "frame too large"));
    }
    out.write_all(&(body.len() as u32).to_be_bytes()).await?;
    out.write_all(&body).await
}

pub(crate) async fn read<T: DeserializeOwned>(
    input: &mut (impl AsyncRead + Unpin),
) -> io::Result<T> {
    let len = input.read_u32().await? as usize;
    if len > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large"));
    }
    let mut body = vec![0; len];
    input.read_exact(&mut body).await?;
    Ok(serde_json::from_slice(&body)?)
}
