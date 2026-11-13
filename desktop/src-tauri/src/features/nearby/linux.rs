//! Calls the phone over plain Bluetooth sockets. BlueZ is left out of it because
//! it only connects to devices it has already discovered.

use std::{
    os::fd::{AsRawFd, BorrowedFd},
    time::Duration,
};

use anyhow::{Context, Result};
use bluer::{Address, AddressType, Uuid, l2cap, rfcomm};
use tokio::{
    io::{Interest, unix::AsyncFd},
    time::timeout,
};

/// Must match the service the phone offers.
const SERVICE: Uuid = Uuid::from_u128(0x6f726974_6865_7200_8000_00805f9b34fb);
const SDP_PSM: u16 = 1;
const SDP_REPLY_SIZE: usize = 512;
const DIAL_TIMEOUT: Duration = Duration::from_secs(20);

pub async fn dial(address: &str) -> Result<rfcomm::Stream> {
    let address: Address = address.parse()?;
    timeout(DIAL_TIMEOUT, async {
        let channel = find_channel(address).await?;
        let stream = rfcomm::Stream::connect(rfcomm::SocketAddr::new(address, channel)).await?;
        settled(&stream).await?;
        stream.peer_addr()?;
        Ok(stream)
    })
    .await
    .context("timed out")?
}

/// Waits for a connection to succeed or fail. bluer's `connect` returns as soon as
/// it starts, since a fresh socket counts as writable, but a socket watched only
/// from here on does not become writable before it is connected.
async fn settled(socket: &impl AsRawFd) -> Result<()> {
    // Safety: `socket` stays open for this borrow, which ends with this line.
    let copy = unsafe { BorrowedFd::borrow_raw(socket.as_raw_fd()) }.try_clone_to_owned()?;
    // Safety: the `AsyncFd` owns the copy, so it stays open as long as that is needed.
    let copy = unsafe { AsyncFd::register_with_interest(copy, Interest::WRITABLE) }?;
    drop(copy.writable().await?);
    Ok(())
}

/// Asks the phone's service directory, since Android picks the channel itself.
async fn find_channel(address: Address) -> Result<u8> {
    let directory = l2cap::SocketAddr::new(address, AddressType::BrEdr, SDP_PSM);
    let directory = l2cap::SeqPacket::connect(directory).await?;
    settled(&directory).await?;
    directory.peer_addr()?;
    directory.send(&channel_request()).await?;
    let mut reply = [0; SDP_REPLY_SIZE];
    let size = directory.recv(&mut reply).await?;
    channel_in(&reply[..size]).context("the phone offers no Orither service")
}

/// An SDP request for the service's protocol list, which holds its channel.
fn channel_request() -> Vec<u8> {
    let mut request = vec![
        0x06, 0, 1, // search for attributes, as transaction 1
        0, 27, // the size of everything below
        0x35, 17, 0x1c, // a list of one 128-bit UUID
    ];
    request.extend_from_slice(SERVICE.as_bytes());
    request.extend_from_slice(&[
        2, 0, // the longest answer to send
        0x35, 3, 0x09, 0, 4, // a list of one attribute, the protocol list
        0, // no continuation
    ]);
    request
}

/// In a protocol list, RFCOMM is the 16-bit UUID 3, then its channel as one byte.
fn channel_in(reply: &[u8]) -> Option<u8> {
    reply.windows(5).find(|bytes| bytes[..4] == [0x19, 0, 3, 0x08]).map(|bytes| bytes[4])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_states_its_own_size() {
        let request = channel_request();
        assert_eq!(usize::from(u16::from_be_bytes([request[3], request[4]])), request.len() - 5);
    }

    #[test]
    fn the_channel_comes_from_the_rfcomm_entry() {
        let protocols = [0x35, 0x0c, 0x35, 0x03, 0x19, 0x01, 0x00, 0x35, 0x05, 0x19, 0x00, 0x03, 0x08, 0x07];
        assert_eq!(channel_in(&protocols), Some(7));
        assert_eq!(channel_in(&protocols[..7]), None);
    }
}
