//! Turns H.264 into pictures.

use anyhow::{Context, Result};
use openh264::{decoder::Decoder, formats::YUVSource, nal_units};

/// One plane of a picture, whose rows sit `stride` bytes apart.
pub struct Plane {
    pub data: Vec<u8>,
    pub stride: usize,
}

/// A decoded picture: full size luma, then both chroma planes at half size.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub planes: [Plane; 3],
}

impl Frame {
    fn of(picture: &impl YUVSource) -> Self {
        let (width, height) = picture.dimensions();
        let (luma, blue, red) = picture.strides();
        let plane = |data: &[u8], stride| Plane { data: data.to_vec(), stride };
        Self {
            width: width as u32,
            height: height as u32,
            planes: [plane(picture.y(), luma), plane(picture.u(), blue), plane(picture.v(), red)],
        }
    }
}

pub struct H264 {
    decoder: Decoder,
}

impl H264 {
    pub fn new() -> Result<Self> {
        Ok(Self { decoder: Decoder::new().context("starting the video decoder")? })
    }

    /// Decodes one packet, returning its picture if it completes one.
    pub fn decode(&mut self, packet: &[u8]) -> Result<Option<Frame>> {
        let mut frame = None;
        for unit in nal_units(packet) {
            if let Some(picture) = self.decoder.decode(unit).context("decoding video")? {
                frame = Some(Frame::of(&picture));
            }
        }
        Ok(frame)
    }
}
