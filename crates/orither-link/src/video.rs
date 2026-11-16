//! Live video that stays live: the phone sends a picture only once the computer
//! has taken the ones before it, and drops pictures rather than let them queue,
//! so a slow moment on the link costs a few pictures instead of a delay that
//! never goes away.
//!
//! The video is scrcpy's stream: a four-byte codec id, then packets, each a
//! twelve-byte header (flags and the time the picture was taken, then the data's
//! length) and the data. A session packet, which gives a new size, is the header
//! alone.
//!
//! A receiver asks for pacing when it attaches. A sender that paces starts with
//! [`PACED`] before the codec id, and one that cannot simply starts with the
//! codec id, so the receiver knows whether to confirm. It confirms after every
//! read with the number of bytes it has taken in all, as a big-endian u64. The
//! sender lets the oldest unconfirmed bytes wait little longer than the quickest
//! confirmation it saw lately, and holds everything else back meanwhile. A
//! picture held back too long is dropped, with every other one waiting before
//! the newest key frame. When no key frame waits, the sender sends [`SKIPPED`]
//! and drops every picture until a key frame comes, which the receiver should
//! ask for.

use std::{
    collections::VecDeque,
    sync::Mutex,
    time::Duration,
};

use anyhow::Result;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::Notify,
    time::{Instant, sleep_until},
};

use crate::session::Duplex;

/// What a pacing sender writes first.
pub const PACED: [u8; 4] = *b"pace";
/// A packet header with every flag set and nothing after it: pictures were dropped,
/// and the next one is a key frame.
pub const SKIPPED: [u8; HEADER] = [0xE0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

const HEADER: usize = 12;
const SESSION: u8 = 0x80;
const CONFIG: u8 = 0x40;
const KEY: u8 = 0x20;

/// How much longer than the quickest confirmation lately the oldest unconfirmed
/// bytes may take before the sender holds back.
const SLACK: Duration = Duration::from_millis(100);
/// What the sender assumes before the first confirmation.
const FIRST_CONFIRMATION: Duration = Duration::from_millis(200);
/// How long a picture may wait to be sent before the waiting ones are dropped.
const STALE: Duration = Duration::from_millis(150);
/// How far back the quickest confirmation counts, so that a slower route is learnt.
const REMEMBER: Duration = Duration::from_secs(3);
/// What may be on its way at least, however slow the link seemed, so that a
/// quicker one is found out.
const LEAST_IN_FLIGHT: u64 = 32 * 1024;
/// How far back the best delivery rate counts.
const RATE_OVER: Duration = Duration::from_secs(2);
/// How often a sender still waiting for a key frame says so again, in case the
/// receiver's first request for one went nowhere.
const SKIPPED_AGAIN: Duration = Duration::from_millis(400);

struct Waiting {
    bytes: Vec<u8>,
    flags: u8,
    since: Instant,
}

impl Waiting {
    /// A picture, as against the codec id, a size or the codec's configuration,
    /// which every picture after them needs.
    fn picture(&self) -> bool {
        self.flags & (SESSION | CONFIG) == 0
    }

    fn key(&self) -> bool {
        self.picture() && self.flags & KEY != 0
    }
}

/// Decides what goes out when. Time comes in from outside, so it can be tested.
#[derive(Default)]
struct Pacer {
    waiting: VecDeque<Waiting>,
    /// Bytes handed to the link so far, and how many the receiver confirmed.
    sent: u64,
    confirmed: u64,
    /// Where each unconfirmed piece ends, when it was handed over, and how much the
    /// receiver had confirmed by then.
    unconfirmed: VecDeque<(u64, Instant, u64)>,
    /// How long recent confirmations took.
    delays: VecDeque<(Instant, Duration)>,
    /// Recent delivery rates in bytes a second: what the receiver confirmed while a
    /// piece was on its way, over the time it took.
    rates: VecDeque<(Instant, f64)>,
    skipping: bool,
    /// When the receiver last heard that pictures were dropped.
    skipped_at: Option<Instant>,
    ended: bool,
}

impl Pacer {
    /// Takes a packet to send.
    fn arrive(&mut self, flags: u8, bytes: Vec<u8>, now: Instant) {
        let waiting = Waiting { bytes, flags, since: now };
        if self.skipping && waiting.picture() {
            if !waiting.key() {
                if self.skipped_at.is_none_or(|at| now.duration_since(at) >= SKIPPED_AGAIN) {
                    self.skipped(now);
                }
                return;
            }
            self.skipping = false;
        }
        self.waiting.push_back(waiting);
    }

    /// Tells the receiver, before anything else, that pictures were dropped.
    fn skipped(&mut self, now: Instant) {
        self.skipping = true;
        self.skipped_at = Some(now);
        if self.waiting.front().is_none_or(|first| first.bytes != SKIPPED) {
            self.waiting.push_front(Waiting { bytes: SKIPPED.to_vec(), flags: SKIPPED[0], since: now });
        }
    }

    fn confirm(&mut self, total: u64, now: Instant) {
        if total <= self.confirmed {
            return;
        }
        self.confirmed = total;
        let mut handed = None;
        while self.unconfirmed.front().is_some_and(|&(end, _, _)| end <= total) {
            handed = self.unconfirmed.pop_front().map(|(_, at, then)| (at, then));
        }
        if let Some((at, then)) = handed {
            let took = now.duration_since(at);
            self.delays.push_back((now, took));
            if !took.is_zero() {
                self.rates.push_back((now, (total - then) as f64 / took.as_secs_f64()));
            }
        }
        while self.delays.front().is_some_and(|&(when, _)| now.duration_since(when) > REMEMBER) {
            self.delays.pop_front();
        }
        while self.rates.front().is_some_and(|&(when, _)| now.duration_since(when) > RATE_OVER) {
            self.rates.pop_front();
        }
    }

    /// What the link delivers, in bytes a second: the best rate lately, as a moment
    /// with little to send says nothing about the link.
    fn rate(&self) -> Option<f64> {
        self.rates.iter().map(|&(_, rate)| rate).reduce(f64::max)
    }

    /// Whether `next` bytes may go now. One piece may always be on its way. More may
    /// while the oldest of them is not overdue, and while together they are what the
    /// link delivers in that time, twice over so that it can show it carries more.
    fn may_send(&self, now: Instant, next: usize) -> bool {
        let Some(&(_, oldest, _)) = self.unconfirmed.front() else { return true };
        let quickest = self.delays.iter().map(|&(_, delay)| delay).min().unwrap_or(FIRST_CONFIRMATION);
        if now.duration_since(oldest) > quickest + SLACK {
            return false;
        }
        let Some(rate) = self.rate() else { return true };
        let allowed = (2.0 * rate * (quickest + SLACK).as_secs_f64()) as u64;
        self.sent - self.confirmed + next as u64 <= allowed.max(LEAST_IN_FLIGHT)
    }

    /// Drops the pictures that waited too long, up to the newest key frame.
    fn drop_stale(&mut self, now: Instant) {
        let oldest = self.waiting.iter().find(|waiting| waiting.picture());
        if oldest.is_none_or(|oldest| now.duration_since(oldest.since) < STALE) {
            return;
        }
        match self.waiting.iter().rposition(Waiting::key) {
            Some(key) => {
                let mut index = 0;
                self.waiting.retain(|waiting| {
                    index += 1;
                    index > key || !waiting.picture()
                });
            }
            None => {
                self.waiting.retain(|waiting| !waiting.picture());
                self.skipped(now);
            }
        }
    }

    /// The next bytes to hand to the link, if any may go now.
    fn next(&mut self, now: Instant) -> Option<Vec<u8>> {
        self.drop_stale(now);
        if !self.may_send(now, self.waiting.front()?.bytes.len()) {
            return None;
        }
        let waiting = self.waiting.pop_front()?;
        self.sent += waiting.bytes.len() as u64;
        self.unconfirmed.push_back((self.sent, now, self.confirmed));
        Some(waiting.bytes)
    }

    /// When pictures waiting now would go stale, the one change that needs no news.
    fn deadline(&self) -> Option<Instant> {
        self.waiting.iter().find(|waiting| waiting.picture()).map(|oldest| oldest.since + STALE)
    }

    fn done(&self) -> bool {
        self.ended && self.waiting.is_empty()
    }
}

struct Shared {
    pacer: Mutex<Pacer>,
    news: Notify,
}

impl Shared {
    fn change(&self, change: impl FnOnce(&mut Pacer)) {
        change(&mut self.pacer.lock().unwrap());
        self.news.notify_one();
    }
}

/// Sends the video that `source` produces over `stream`, paced, until the video ends
/// or the receiver goes away.
pub async fn send_paced(source: impl AsyncRead + Unpin, stream: Duplex) -> Result<()> {
    let Duplex { mut send, recv } = stream;
    pace(source, &mut send, recv).await?;
    send.finish()?;
    Ok(())
}

/// Everything [`send_paced`] does but ending the stream, with any streams.
async fn pace(
    mut source: impl AsyncRead + Unpin,
    send: &mut (impl AsyncWrite + Unpin),
    mut confirmations: impl AsyncRead + Unpin,
) -> Result<()> {
    let shared = Shared { pacer: Mutex::new(Pacer::default()), news: Notify::new() };
    send.write_all(&PACED).await?;
    // Both sides count the bytes after the codec id, which is never held back.
    let mut codec = [0; 4];
    source.read_exact(&mut codec).await?;
    send.write_all(&codec).await?;

    let reading = async {
        let read = read_packets(&mut source, &shared).await;
        shared.change(|pacer| pacer.ended = true);
        read
    };
    let confirming = async {
        let mut total = [0; 8];
        loop {
            confirmations.read_exact(&mut total).await?;
            shared.change(|pacer| pacer.confirm(u64::from_be_bytes(total), Instant::now()));
        }
    };
    let writing = async {
        loop {
            let news = shared.news.notified();
            let (next, deadline, done) = {
                let mut pacer = shared.pacer.lock().unwrap();
                (pacer.next(Instant::now()), pacer.deadline(), pacer.done())
            };
            if let Some(bytes) = next {
                send.write_all(&bytes).await?;
                send.flush().await?;
                continue;
            }
            if done {
                return anyhow::Ok(());
            }
            match deadline {
                Some(deadline) => tokio::select! {
                    () = news => {}
                    () = sleep_until(deadline) => {}
                },
                None => news.await,
            }
        }
    };
    tokio::select! {
        (read, written) = async { tokio::join!(reading, writing) } => read.and(written),
        // The receiver stopped confirming, so it is gone.
        confirmed = confirming => confirmed,
    }
}

async fn read_packets(source: &mut (impl AsyncRead + Unpin), shared: &Shared) -> Result<()> {
    loop {
        let mut header = [0; HEADER];
        match source.read_exact(&mut header).await {
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(err) => return Err(err.into()),
        }
        let flags = header[0];
        let mut bytes = header.to_vec();
        if flags & SESSION == 0 {
            let length = u32::from_be_bytes([header[8], header[9], header[10], header[11]]) as usize;
            bytes.resize(HEADER + length, 0);
            source.read_exact(&mut bytes[HEADER..]).await?;
        }
        shared.change(|pacer| pacer.arrive(flags, bytes, Instant::now()));
    }
}

/// Whether a video stream is paced, from its first four bytes, and its codec id.
pub async fn start(recv: &mut (impl AsyncRead + Unpin)) -> Result<(bool, [u8; 4])> {
    let mut first = [0; 4];
    recv.read_exact(&mut first).await?;
    if first != PACED {
        return Ok((false, first));
    }
    recv.read_exact(&mut first).await?;
    Ok((true, first))
}

/// Passes a video stream on to `out` after [`start`], confirming every read when
/// it is paced, until the stream ends.
pub async fn receive(
    recv: &mut (impl AsyncRead + Unpin),
    confirm: Option<&mut (impl AsyncWrite + Unpin)>,
    out: &mut (impl AsyncWrite + Unpin),
) -> Result<()> {
    let limit = test_limit();
    let mut buffer = vec![0; if limit.is_some() { 4096 } else { 64 * 1024 }];
    let (mut total, mut allowed) = (0u64, Instant::now());
    let mut confirm = confirm;
    loop {
        let read = recv.read(&mut buffer).await?;
        if read == 0 {
            return Ok(());
        }
        out.write_all(&buffer[..read]).await?;
        out.flush().await?;
        total += read as u64;
        if let Some(bits_per_second) = limit {
            allowed = allowed.max(Instant::now()) + Duration::from_secs_f64(read as f64 * 8.0 / bits_per_second);
            sleep_until(allowed).await;
        }
        // A sender that stopped listening has finished, and the rest still counts.
        if let Some(writer) = confirm.as_deref_mut()
            && writer.write_all(&total.to_be_bytes()).await.is_err()
        {
            confirm = None;
        }
    }
}

/// For trying a slow link without one: `ORITHER_TEST_VIDEO_KBPS` makes the receiver
/// take video no faster than that, which the sender cannot tell from a slow link.
fn test_limit() -> Option<f64> {
    let kbps: f64 = std::env::var("ORITHER_TEST_VIDEO_KBPS").ok()?.parse().ok()?;
    (kbps > 0.0).then_some(kbps * 1000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    fn picture(key: bool, size: usize) -> (u8, Vec<u8>) {
        let flags = if key { KEY } else { 0 };
        let mut bytes = vec![flags, 0, 0, 0, 0, 0, 0, 0];
        bytes.extend_from_slice(&(size as u32).to_be_bytes());
        bytes.resize(HEADER + size, 1);
        (flags, bytes)
    }

    fn arrive(pacer: &mut Pacer, (flags, bytes): (u8, Vec<u8>), now: Instant) {
        pacer.arrive(flags, bytes, now);
    }

    #[test]
    fn a_quick_receiver_gets_everything_in_order() {
        let (mut pacer, start) = (Pacer::default(), Instant::now());
        for frame in 0..10u32 {
            let now = start + MS * (frame * 16);
            arrive(&mut pacer, picture(frame == 0, 100), now);
            let bytes = pacer.next(now).expect("a picture to send");
            assert_eq!(bytes.len(), HEADER + 100);
            pacer.confirm(pacer.sent, now + 5 * MS);
        }
        assert!(pacer.waiting.is_empty());
        assert!(!pacer.skipping);
    }

    #[test]
    fn a_stalled_receiver_holds_pictures_back_then_skips_to_a_key_frame() {
        let (mut pacer, start) = (Pacer::default(), Instant::now());
        arrive(&mut pacer, picture(true, 100), start);
        assert!(pacer.next(start).is_some());
        pacer.confirm(pacer.sent, start + 10 * MS);

        arrive(&mut pacer, picture(false, 100), start + 20 * MS);
        assert!(pacer.next(start + 20 * MS).is_some());
        // Nothing confirms that one, so the next ones wait, then go stale.
        for frame in 2..10u32 {
            arrive(&mut pacer, picture(false, 100), start + MS * (frame * 16));
        }
        assert!(pacer.next(start + 131 * MS).is_none());
        assert_eq!(pacer.waiting.len(), 8);

        pacer.confirm(pacer.sent, start + 250 * MS);
        let bytes = pacer.next(start + 250 * MS).expect("the receiver hears about the drop");
        assert_eq!(bytes, SKIPPED);
        assert!(pacer.waiting.is_empty() && pacer.skipping);

        // Until a key frame, pictures are dropped as they come.
        arrive(&mut pacer, picture(false, 100), start + 260 * MS);
        assert!(pacer.waiting.is_empty());
        arrive(&mut pacer, picture(true, 100), start + 270 * MS);
        assert!(!pacer.skipping);
        pacer.confirm(pacer.sent, start + 271 * MS);
        assert_eq!(pacer.next(start + 271 * MS).map(|bytes| bytes[0]), Some(KEY));
    }

    #[test]
    fn a_sender_without_a_key_frame_says_so_again() {
        let (mut pacer, start) = (Pacer::default(), Instant::now());
        arrive(&mut pacer, picture(true, 100), start);
        assert!(pacer.next(start).is_some());
        arrive(&mut pacer, picture(false, 100), start + 10 * MS);
        pacer.confirm(pacer.sent, start + 300 * MS);
        assert_eq!(pacer.next(start + 300 * MS), Some(SKIPPED.to_vec()));
        pacer.confirm(pacer.sent, start + 301 * MS);
        // Pictures keep coming without a key frame, so the receiver hears again a while later.
        let skipped = start + 300 * MS;
        arrive(&mut pacer, picture(false, 100), skipped + SKIPPED_AGAIN - 50 * MS);
        assert!(pacer.next(skipped + SKIPPED_AGAIN - 50 * MS).is_none());
        arrive(&mut pacer, picture(false, 100), skipped + SKIPPED_AGAIN + 50 * MS);
        assert_eq!(pacer.next(skipped + SKIPPED_AGAIN + 50 * MS), Some(SKIPPED.to_vec()));
    }

    #[test]
    fn a_waiting_key_frame_saves_asking_for_one() {
        let (mut pacer, start) = (Pacer::default(), Instant::now());
        arrive(&mut pacer, picture(true, 100), start);
        assert!(pacer.next(start).is_some());
        arrive(&mut pacer, picture(false, 100), start + 10 * MS);
        arrive(&mut pacer, picture(true, 100), start + 20 * MS);
        arrive(&mut pacer, picture(false, 100), start + 30 * MS);
        pacer.confirm(pacer.sent, start + 300 * MS);
        let next = pacer.next(start + 300 * MS).expect("the newer key frame");
        assert_eq!(next[0], KEY);
        assert_eq!(pacer.waiting.len(), 1);
        assert!(!pacer.skipping);
    }

    #[test]
    fn the_codec_id_and_sizes_are_never_dropped() {
        let (mut pacer, start) = (Pacer::default(), Instant::now());
        pacer.arrive(CONFIG, b"h264".to_vec(), start);
        assert!(pacer.next(start).is_some());
        arrive(&mut pacer, picture(false, 100), start);
        pacer.arrive(SESSION, vec![SESSION; HEADER], start + MS);
        arrive(&mut pacer, picture(false, 100), start + 2 * MS);
        pacer.confirm(pacer.sent, start + 500 * MS);
        assert_eq!(pacer.next(start + 500 * MS), Some(SKIPPED.to_vec()));
        assert_eq!(pacer.next(start + 500 * MS).map(|bytes| bytes[0]), Some(SESSION));
        assert!(pacer.next(start + 500 * MS).is_none());
    }

    #[test]
    fn what_is_on_its_way_follows_what_the_link_delivers() {
        let (mut pacer, start) = (Pacer::default(), Instant::now());
        // A link that takes 100 kB a second, confirming 10 kB each tenth of a second.
        let mut now = start;
        for _ in 0..12 {
            arrive(&mut pacer, picture(false, 10_000 - HEADER), now);
            assert!(pacer.next(now).is_some());
            now += 100 * MS;
            pacer.confirm(pacer.sent, now);
        }
        assert!(pacer.unconfirmed.is_empty());
        // Big pictures now: one goes, and the next waits, as both are more than the link
        // delivers in the time allowed.
        arrive(&mut pacer, picture(false, 30_000), now);
        arrive(&mut pacer, picture(false, 30_000), now);
        assert!(pacer.next(now).is_some());
        assert!(pacer.next(now + MS).is_none());
    }

    #[test]
    fn a_slower_route_is_learnt() {
        let (mut pacer, start) = (Pacer::default(), Instant::now());
        // Quick confirmations at first, then a route that takes 300 ms.
        for frame in 0..5u32 {
            let now = start + MS * (frame * 10);
            arrive(&mut pacer, picture(frame == 0, 10), now);
            pacer.next(now);
            pacer.confirm(pacer.sent, now + 5 * MS);
        }
        let mut now = start + Duration::from_secs(1);
        for _ in 0..40 {
            arrive(&mut pacer, picture(true, 10), now);
            if pacer.next(now).is_some() {
                pacer.confirm(pacer.sent, now + 300 * MS);
            }
            now += 300 * MS;
        }
        assert!(pacer.may_send(now + 350 * MS, 10) || pacer.unconfirmed.is_empty());
        let quickest = pacer.delays.iter().map(|&(_, delay)| delay).min().unwrap();
        assert_eq!(quickest, 300 * MS);
    }

    /// scrcpy's stream: the codec id, a size, then a key frame and `count - 1` others.
    fn scrcpy(count: usize, size: usize) -> Vec<u8> {
        let mut stream = b"h264".to_vec();
        stream.extend_from_slice(&[SESSION, 0, 0, 0, 0, 0, 2, 208, 0, 0, 6, 24]);
        for frame in 0..count {
            stream.extend(picture(frame == 0, size).1);
        }
        stream
    }

    #[tokio::test]
    async fn a_quick_receiver_gets_the_whole_stream() {
        let (mut phone, scrcpy_out) = tokio::io::duplex(1 << 20);
        let (mut to_computer, mut from_phone) = tokio::io::duplex(1 << 20);
        let (mut confirm, confirmations) = tokio::io::duplex(1 << 10);
        let sent = scrcpy(30, 1000);
        let expected = sent.clone();
        tokio::spawn(async move {
            phone.write_all(&sent).await.unwrap();
        });
        let sending = tokio::spawn(async move { pace(scrcpy_out, &mut to_computer, confirmations).await });
        let (paced, codec) = start(&mut from_phone).await.unwrap();
        assert!(paced);
        let mut out = codec.to_vec();
        receive(&mut from_phone, Some(&mut confirm), &mut out).await.unwrap();
        assert_eq!(out, expected);
        sending.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn a_slow_receiver_gets_fewer_pictures_and_no_delay() {
        let (mut phone, scrcpy_out) = tokio::io::duplex(1 << 20);
        let (mut to_computer, mut from_phone) = tokio::io::duplex(1 << 20);
        let (mut confirm, confirmations) = tokio::io::duplex(1 << 10);
        let filmed = Instant::now();
        tokio::spawn(async move {
            phone.write_all(b"h264").await.unwrap();
            for frame in 0..60 {
                phone.write_all(&picture(frame % 30 == 0, 2000).1).await.unwrap();
                tokio::time::sleep(16 * MS).await;
            }
        });
        tokio::spawn(async move { pace(scrcpy_out, &mut to_computer, confirmations).await });
        assert!(start(&mut from_phone).await.unwrap().0);

        let (mut flags, mut total) = (Vec::new(), 0u64);
        loop {
            let mut header = [0; HEADER];
            if from_phone.read_exact(&mut header).await.is_err() {
                break;
            }
            let length = if header == SKIPPED { 0 } else { u32::from_be_bytes([header[8], header[9], header[10], header[11]]) };
            from_phone.read_exact(&mut vec![0; length as usize]).await.unwrap();
            total += (HEADER + length as usize) as u64;
            let _ = confirm.write_all(&total.to_be_bytes()).await;
            flags.push(header[0]);
            // Five times slower than the pictures come.
            tokio::time::sleep(80 * MS).await;
        }
        assert!(flags.contains(&SKIPPED[0]), "{flags:?}");
        assert!(flags.len() < 30, "{flags:?}");
        for pair in flags.windows(2) {
            if pair[0] == SKIPPED[0] {
                assert_ne!(pair[1] & KEY, 0, "a picture after a drop needs no picture before it");
            }
        }
        // The last picture was filmed a second in, and arrives soon after.
        assert!(filmed.elapsed() < Duration::from_millis(2000), "{:?}", filmed.elapsed());
    }

    /// As on a phone: key frames come only when the receiver asks for one, with a
    /// cooldown between requests like the window's.
    #[tokio::test]
    async fn a_slow_receiver_that_asks_for_key_frames_keeps_getting_pictures() {
        let (mut phone, scrcpy_out) = tokio::io::duplex(1 << 20);
        let (mut to_computer, mut from_phone) = tokio::io::duplex(1 << 20);
        let (mut confirm, confirmations) = tokio::io::duplex(1 << 10);
        let (ask, mut asked) = tokio::sync::mpsc::unbounded_channel::<()>();
        tokio::spawn(async move {
            phone.write_all(b"h264").await.unwrap();
            for frame in 0..150 {
                let key = frame == 0 || asked.try_recv().is_ok();
                phone.write_all(&picture(key, if key { 6000 } else { 2000 }).1).await.unwrap();
                tokio::time::sleep(16 * MS).await;
            }
        });
        tokio::spawn(async move { pace(scrcpy_out, &mut to_computer, confirmations).await });
        assert!(start(&mut from_phone).await.unwrap().0);

        let (mut pictures, mut asked_at, mut last) = (0, None::<Instant>, Instant::now());
        let mut total = 0u64;
        loop {
            let mut header = [0; HEADER];
            if from_phone.read_exact(&mut header).await.is_err() {
                break;
            }
            if header == SKIPPED {
                if asked_at.is_none_or(|at| at.elapsed() >= 500 * MS) {
                    asked_at = Some(Instant::now());
                    let _ = ask.send(());
                }
            } else {
                let length = u32::from_be_bytes([header[8], header[9], header[10], header[11]]) as usize;
                from_phone.read_exact(&mut vec![0; length]).await.unwrap();
                total += length as u64;
                pictures += 1;
                last = Instant::now();
            }
            total += HEADER as u64;
            let _ = confirm.write_all(&total.to_be_bytes()).await;
            tokio::time::sleep(60 * MS).await;
        }
        // Two and a half seconds of pictures at a sixth of their pace: a picture now
        // and then all the way through, rather than none after the first drop.
        assert!(pictures >= 8, "only {pictures} pictures");
        assert!(last.elapsed() < Duration::from_millis(500));
    }

    #[tokio::test]
    async fn a_paced_stream_announces_itself() {
        let mut paced = std::io::Cursor::new([PACED.as_slice(), b"h264"].concat());
        assert_eq!(start(&mut paced).await.unwrap(), (true, *b"h264"));
        let mut plain = std::io::Cursor::new(b"h264".to_vec());
        assert_eq!(start(&mut plain).await.unwrap(), (false, *b"h264"));
    }
}
