//! scrcpy's video stream: a codec id, then packets that each start with twelve
//! bytes, either a new video size or the length of some encoded data.

use std::io::{ErrorKind, Read};

use anyhow::{Result, bail};

const H264: u32 = u32::from_be_bytes(*b"h264");
const NO_VIDEO: u32 = 0;
const FAILED: u32 = 1;
const SESSION: u8 = 0x80;
const CONFIG: u8 = 0x40;
const KEY: u8 = 0x20;
/// Every flag at once, which scrcpy never sends: the phone dropped pictures to
/// stay live, see `orither_link::video`.
const SKIPPED: u8 = 0xE0;
/// The flags take the top bits, and the time the rest.
const PTS: u64 = (1 << 61) - 1;

#[derive(Debug, PartialEq, Eq)]
pub enum Packet {
    /// The video has a new size from here on.
    Session { width: u32, height: u32 },
    /// The phone dropped pictures to keep up, and sends none until a key frame.
    Skipped,
    /// Encoded data, and when the phone took its picture in microseconds.
    /// Configuration holds no picture of its own.
    Media { config: bool, key: bool, pts: u64, data: Vec<u8> },
}

pub struct Reader<R> {
    input: R,
}

impl<R: Read> Reader<R> {
    pub fn new(mut input: R) -> Result<Self> {
        let mut codec = [0; 4];
        input.read_exact(&mut codec)?;
        Self::with_codec(input, codec)
    }

    /// Reads on from a stream whose codec id was read already.
    pub fn with_codec(input: R, codec: [u8; 4]) -> Result<Self> {
        match u32::from_be_bytes(codec) {
            H264 => Ok(Self { input }),
            NO_VIDEO => bail!("the phone offered no video"),
            FAILED => bail!("the phone could not start its video"),
            other => bail!("unexpected video codec {other:#x}"),
        }
    }

    /// The next packet, or `None` once the stream has ended.
    pub fn next(&mut self) -> Result<Option<Packet>> {
        let mut head = [0; 12];
        match self.input.read_exact(&mut head) {
            Ok(()) => {}
            Err(err) if err.kind() == ErrorKind::UnexpectedEof => return Ok(None),
            Err(err) => return Err(err.into()),
        }
        if head[0] & SKIPPED == SKIPPED {
            return Ok(Some(Packet::Skipped));
        }
        let [_, _, _, _, a, b, c, d, e, f, g, h] = head;
        let pts = u64::from_be_bytes([head[0], head[1], head[2], head[3], a, b, c, d]) & PTS;
        if head[0] & SESSION != 0 {
            let (width, height) = (u32::from_be_bytes([a, b, c, d]), u32::from_be_bytes([e, f, g, h]));
            return Ok(Some(Packet::Session { width, height }));
        }
        let mut data = vec![0; u32::from_be_bytes([e, f, g, h]) as usize];
        self.input.read_exact(&mut data)?;
        Ok(Some(Packet::Media { config: head[0] & CONFIG != 0, key: head[0] & KEY != 0, pts, data }))
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn packets_come_out_in_order() {
        let mut bytes = b"h264".to_vec();
        bytes.extend_from_slice(&[0x80, 0, 0, 0]);
        bytes.extend_from_slice(&720u32.to_be_bytes());
        bytes.extend_from_slice(&1560u32.to_be_bytes());
        bytes.extend_from_slice(&(1u64 << 62).to_be_bytes());
        bytes.extend_from_slice(&2u32.to_be_bytes());
        bytes.extend_from_slice(&[1, 2]);
        bytes.extend_from_slice(&5u64.to_be_bytes());
        bytes.extend_from_slice(&1u32.to_be_bytes());
        bytes.push(3);

        let mut reader = Reader::new(Cursor::new(bytes)).unwrap();
        assert_eq!(reader.next().unwrap(), Some(Packet::Session { width: 720, height: 1560 }));
        assert_eq!(reader.next().unwrap(), Some(Packet::Media { config: true, key: false, pts: 0, data: vec![1, 2] }));
        assert_eq!(reader.next().unwrap(), Some(Packet::Media { config: false, key: false, pts: 5, data: vec![3] }));
        assert_eq!(reader.next().unwrap(), None);
    }

    #[test]
    fn a_drop_is_told_apart_from_a_size() {
        let mut bytes = b"h264".to_vec();
        bytes.extend_from_slice(&[0xE0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        let mut reader = Reader::new(Cursor::new(bytes)).unwrap();
        assert_eq!(reader.next().unwrap(), Some(Packet::Skipped));
        assert_eq!(reader.next().unwrap(), None);
    }

    #[test]
    fn a_failed_capture_is_an_error() {
        assert!(Reader::new(Cursor::new(1u32.to_be_bytes())).is_err());
    }
}
