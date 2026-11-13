//! The v4l2loopback device that programs on this computer see as a webcam. Programs
//! only list it while something has put a picture on it, so it stays open, and shows
//! black while the phone's camera is off.

use std::{
    fs::{self, File},
    io::{self, Write},
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    path::Path,
};

use anyhow::{Context, Result, ensure};
use orither_viewer::Frame;
use serde::{Deserialize, Serialize};
use tokio::io::{Interest, unix::AsyncFd};

/// The size of a picture, in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

impl Size {
    /// How many bytes across and down each plane of a YU12 picture has: brightness, then
    /// the two colour planes at half the size each way.
    fn planes(self) -> [(usize, usize); 3] {
        let (width, height) = (self.width as usize, self.height as usize);
        let half = (width.div_ceil(2), height.div_ceil(2));
        [(width, height), half, half]
    }

    fn bytes(self) -> usize {
        self.planes().iter().map(|(across, down)| across * down).sum()
    }
}

/// Black in YU12, plane by plane: the lowest brightness video uses, and no colour.
const BLACK: [u8; 3] = [16, 128, 128];

/// The webcam, open for showing pictures and for hearing whether a program takes them.
pub struct Webcam {
    device: AsyncFd<File>,
    size: Size,
    black: Vec<u8>,
    /// Older v4l2loopback versions cannot tell, and then the webcam always counts as used.
    follows_use: bool,
    /// Whether a program is taking pictures from the webcam.
    pub in_use: bool,
}

impl Webcam {
    /// Opens the device and puts black on it. The device keeps the size it has if a
    /// program holds on to that, so [`Self::size`] may differ from `size`.
    pub fn open(path: &Path, size: Size) -> Result<Self> {
        let device = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)
            .context("opening the camera device")?;
        let size = set_format(&device, size)?;
        let follows_use = subscribe(&device)?;
        if !follows_use {
            tracing::info!("this v4l2loopback cannot tell when the webcam is used, so the phone films all the while");
        }
        // SAFETY: the file owns its descriptor and keeps it for as long as it lives.
        let device = unsafe { AsyncFd::register_with_interest(device, Interest::PRIORITY) }.map_err(io::Error::from)?;
        let webcam = Self { device, size, black: black(size), follows_use, in_use: !follows_use };
        webcam.blank()?;
        Ok(webcam)
    }

    pub fn size(&self) -> Size {
        self.size
    }

    /// Shows a picture already fitted to [`Self::size`].
    pub fn show(&self, picture: &[u8]) -> Result<()> {
        self.device.get_ref().write_all(picture).context("showing a picture on the camera device")
    }

    /// Shows black, which a program starting to use the webcam gets first.
    pub fn blank(&self) -> Result<()> {
        self.show(&self.black)
    }

    /// Waits until `in_use` changes. Safe to drop at any point, as `select!` does.
    pub async fn change(&mut self) -> Result<()> {
        if !self.follows_use {
            return std::future::pending().await;
        }
        loop {
            let mut ready = self.device.ready(Interest::PRIORITY).await?;
            let was_in_use = self.in_use;
            while let Some(users) = next_usage(ready.get_inner())? {
                self.in_use = users > 0;
            }
            ready.clear_ready();
            if self.in_use != was_in_use {
                return Ok(());
            }
        }
    }
}

/// The frame as YU12 at `size`: centred, with black where it falls short and its edges
/// cut off where it is bigger. At the frame's own size this only takes out the gaps
/// between its rows.
pub fn fitted(frame: &Frame, size: Size) -> Vec<u8> {
    let own = Size { width: frame.width, height: frame.height };
    // Even shifts keep the half-size colour planes lined up with the brightness.
    let shift = |own: u32, wanted: u32| (own.abs_diff(wanted) / 2 & !1) as usize;
    let (across, down) = (shift(own.width, size.width), shift(own.height, size.height));
    let mut bytes = Vec::with_capacity(size.bytes());
    for (index, (plane, (own, wanted))) in frame.planes.iter().zip(own.planes().into_iter().zip(size.planes())).enumerate() {
        let scale = if index == 0 { 1 } else { 2 };
        let (from_x, to_x, columns) = overlap(own.0, wanted.0, across / scale);
        let (from_y, to_y, rows) = overlap(own.1, wanted.1, down / scale);
        let start = bytes.len();
        bytes.resize(start + wanted.0 * wanted.1, BLACK[index]);
        for row in 0..rows {
            let source = &plane.data[(from_y + row) * plane.stride + from_x..][..columns];
            let target = start + (to_y + row) * wanted.0 + to_x;
            bytes[target..target + columns].copy_from_slice(source);
        }
    }
    bytes
}

/// For a line of `own` pixels centred on a line of `wanted`, `shift` apart: where the
/// copy starts in the first, where it lands in the second, and how many pixels it takes.
fn overlap(own: usize, wanted: usize, shift: usize) -> (usize, usize, usize) {
    if own >= wanted { (shift, 0, wanted) } else { (0, shift, own) }
}

fn black(size: Size) -> Vec<u8> {
    let planes = size.planes().into_iter().zip(BLACK);
    planes.flat_map(|((across, down), black)| std::iter::repeat_n(black, across * down)).collect()
}

const VIDEO_OUTPUT: u32 = 2;
const FIELD_NONE: u32 = 1;
const COLORSPACE_REC709: u32 = 3;
const YU12: u32 = u32::from_le_bytes(*b"YU12");
/// `V4L2_EVENT_PRI_CLIENT_USAGE`, which v4l2loopback sends as programs start and stop
/// taking pictures.
const CLIENT_USAGE: u32 = 0x0800_0000 + 0x08e0_0000 + 1;
/// `V4L2_EVENT_SUB_FL_SEND_INITIAL`: tell how things stand straight away, too.
const SEND_INITIAL: u32 = 1;
// The requests, as a 64-bit kernel numbers them.
/// `_IOWR('V', 5, struct v4l2_format)`.
const VIDIOC_S_FMT: libc::c_ulong = 0xc0d0_5605;
/// `_IOR('V', 89, struct v4l2_event)`.
const VIDIOC_DQEVENT: libc::c_ulong = 0x8088_5659;
/// `_IOW('V', 90, struct v4l2_event_subscription)`.
const VIDIOC_SUBSCRIBE_EVENT: libc::c_ulong = 0x4020_565a;

/// The parts of `struct v4l2_format` that matter for raw video.
#[repr(C, align(8))]
struct VideoFormat {
    kind: u32,
    /// The picture's details share a union with structs that hold pointers, so they
    /// start 8 bytes in. Without this gap the kernel reads every field one place off.
    gap: u32,
    width: u32,
    height: u32,
    pixel_format: u32,
    field: u32,
    bytes_per_line: u32,
    image_size: u32,
    color_space: u32,
    rest: [u8; 208 - 36],
}

/// The parts of `struct v4l2_event` that a usage event fills in.
#[repr(C, align(8))]
struct Event {
    kind: u32,
    /// The details start 8 bytes in, as in [`VideoFormat`].
    gap: u32,
    /// How many programs take pictures, which v4l2loopback counts no higher than one.
    users: u32,
    rest: [u8; 136 - 12],
}

/// Sets the size of the pictures this writes, and returns the size the device took.
fn set_format(device: &File, size: Size) -> Result<Size> {
    let mut format = VideoFormat {
        kind: VIDEO_OUTPUT,
        gap: 0,
        width: size.width,
        height: size.height,
        pixel_format: YU12,
        field: FIELD_NONE,
        bytes_per_line: size.width,
        image_size: size.bytes() as u32,
        color_space: COLORSPACE_REC709,
        rest: [0; 172],
    };
    // SAFETY: the file is open and the format has the size this request says.
    let result = unsafe { libc::ioctl(device.as_raw_fd(), VIDIOC_S_FMT, &raw mut format) };
    ensure!(result == 0, "the camera device refused the picture size: {}", io::Error::last_os_error());
    ensure!(format.pixel_format == YU12, "a program holds the camera device in another picture format");
    Ok(Size { width: format.width, height: format.height })
}

/// Asks the device to say when programs start and stop using it. False if it cannot.
fn subscribe(device: &File) -> Result<bool> {
    // `struct v4l2_event_subscription`: the event, an id, flags, and room to spare.
    let mut subscription = [CLIENT_USAGE, 0, SEND_INITIAL, 0, 0, 0, 0, 0];
    // SAFETY: the file is open and the subscription has the size this request says.
    if unsafe { libc::ioctl(device.as_raw_fd(), VIDIOC_SUBSCRIBE_EVENT, subscription.as_mut_ptr()) } == 0 {
        return Ok(true);
    }
    match io::Error::last_os_error() {
        err if err.raw_os_error() == Some(libc::EINVAL) => Ok(false),
        err => Err(err).context("asking the camera device to say when it is used"),
    }
}

/// How many programs the next waiting usage event counts, if one waits.
fn next_usage(device: &File) -> Result<Option<u32>> {
    let mut event = Event { kind: 0, gap: 0, users: 0, rest: [0; 124] };
    // SAFETY: the file is open and the event has the size this request says.
    if unsafe { libc::ioctl(device.as_raw_fd(), VIDIOC_DQEVENT, &raw mut event) } == 0 {
        return Ok(Some(event.users));
    }
    match io::Error::last_os_error() {
        err if err.raw_os_error() == Some(libc::ENOENT) => Ok(None),
        err => Err(err).context("hearing from the camera device"),
    }
}

#[cfg(test)]
mod tests {
    use orither_viewer::Plane;

    use super::*;

    fn frame(width: u32, height: u32, planes: [(&[u8], usize); 3]) -> Frame {
        let planes = planes.map(|(data, stride)| Plane { data: data.to_vec(), stride });
        Frame { width, height, planes }
    }

    #[test]
    fn the_requests_have_the_layout_the_kernel_expects() {
        assert_eq!(size_of::<VideoFormat>(), 208);
        assert_eq!(std::mem::offset_of!(VideoFormat, width), 8);
        assert_eq!(std::mem::offset_of!(VideoFormat, color_space), 32);
        assert_eq!(size_of::<Event>(), 136);
        assert_eq!(std::mem::offset_of!(Event, users), 8);
    }

    #[test]
    fn fitting_at_the_same_size_drops_row_padding() {
        let frame = frame(2, 2, [(&[1, 2, 0, 3, 4, 0], 3), (&[5, 0], 2), (&[6, 0], 2)]);
        assert_eq!(fitted(&frame, Size { width: 2, height: 2 }), [1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn fitting_a_narrow_picture_centres_it_on_black() {
        let frame = frame(2, 2, [(&[1, 2, 3, 4], 2), (&[5], 1), (&[6], 1)]);
        let fitted = fitted(&frame, Size { width: 6, height: 2 });
        let expected = [16, 16, 1, 2, 16, 16, 16, 16, 3, 4, 16, 16, 128, 5, 128, 128, 6, 128];
        assert_eq!(fitted, expected);
    }

    #[test]
    fn fitting_a_wide_picture_cuts_its_edges() {
        let brightness: Vec<u8> = (0..12).collect();
        let frame = frame(6, 2, [(&brightness, 6), (&[12, 13, 14], 3), (&[15, 16, 17], 3)]);
        assert_eq!(fitted(&frame, Size { width: 2, height: 2 }), [2, 3, 8, 9, 13, 16]);
    }

    #[test]
    fn black_fills_every_plane() {
        assert_eq!(black(Size { width: 2, height: 2 }), [16, 16, 16, 16, 128, 128]);
    }
}
