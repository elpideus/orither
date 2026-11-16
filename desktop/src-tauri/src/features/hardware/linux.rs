//! Sound passes through named pipes that PipeWire or PulseAudio reads and writes
//! as a virtual device, and video goes to a v4l2loopback device, see [`webcam`].

#[path = "camera_setup.rs"]
mod camera_setup;
#[path = "webcam.rs"]
mod webcam;

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Output, Stdio},
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use orither_link::{Duplex, Route, Session, video};
use orither_viewer::{H264, Packet, Reader, Stats};
use serde_json::{Value, json};
use tauri::async_runtime;
use tokio::{
    io::{self, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader, Lines},
    net::unix::pipe,
    process::{Child, ChildStdout, Command},
    sync::mpsc,
    time::{Instant, interval, sleep_until},
};
use tokio_util::io::SyncIoBridge;
use webcam::{Size, Webcam};

use crate::{
    features::{Phone, helper::ensure_engine},
    preferences,
};

const SPEAKER: &str = "orither_speaker";
const MICROPHONE: &str = "orither_microphone";
const SOUND: &str = "format=s16le rate=48000 channels=2";
/// Keeps a new device from taking over as the default the moment it appears.
const STAY_BACK: &str = "priority.session=1 priority.driver=1";
/// Programs often close a device and open it again a moment later.
const STAY_ON_FOR: Duration = Duration::from_secs(5);
/// The phone's raw sound comes in frames of two 16-bit samples.
const FRAME: usize = 4;
/// A pipe takes a write this size or smaller whole or not at all, so whole frames in
/// writes this size never leave half a frame behind, which would garble all that follows.
const WHOLE_WRITE: usize = 4096;
pub(super) const CAMERA_LABEL: &str = "Orither Camera";
const CAMERA_SIZE: u32 = 1280;
/// A key frame a second, as a camera cannot be asked for one: after the phone drops
/// pictures to keep up, the picture comes back within that.
const CAMERA_KEY_FRAMES: &str = "video_codec_options=i-frame-interval:int=1";
/// The webcam's size until it has seen the phone's pictures. Most phone cameras take
/// them 4 by 3.
const CAMERA_FIRST_SIZE: Size = Size { width: CAMERA_SIZE, height: CAMERA_SIZE / 4 * 3 };
/// Where the size of the phone's pictures is remembered.
const CAMERA_PICTURE_SIZE: &str = "hardware-camera-size";
/// How often black is shown while the phone's camera starts, so programs get pictures
/// at once.
const BLANK_EVERY: Duration = Duration::from_millis(100);

fn sound_folder() -> Result<PathBuf> {
    let runtime = env::var_os("XDG_RUNTIME_DIR").context("no runtime folder")?;
    let folder = Path::new(&runtime).join("orither");
    fs::create_dir_all(&folder)?;
    Ok(folder)
}

async fn pactl(args: &[&str]) -> Result<Output> {
    let output = Command::new("pactl").args(args).kill_on_drop(true).output().await.context("running pactl")?;
    ensure!(output.status.success(), "pactl: {}", String::from_utf8_lossy(&output.stderr).trim());
    Ok(output)
}

/// A sound device, gone again when this is dropped.
struct Module(String);

impl Module {
    async fn load(kind: &str, name: &str, description: &str, file: &Path) -> Result<Self> {
        let _ = fs::remove_file(file);
        let role = if kind == "module-pipe-sink" { "sink" } else { "source" };
        // The quotes keep the properties together. Without node.virtual=false, desktops
        // count the device as virtual, and KDE hides those.
        let properties = format!("device.description=\"{description}\" node.virtual=false {STAY_BACK}");
        let arguments = [
            kind.to_owned(),
            format!("{role}_name={name}"),
            format!("file={}", file.display()),
            format!("{role}_properties='{properties}'"),
        ];
        let arguments = arguments.iter().map(String::as_str).chain(SOUND.split(' '));
        let mut command = vec!["load-module"];
        command.extend(arguments);
        let output = pactl(&command).await?;
        Ok(Self(String::from_utf8_lossy(&output.stdout).trim().to_owned()))
    }
}

impl Drop for Module {
    fn drop(&mut self) {
        unload_module(&self.0);
    }
}

fn unload_module(id: &str) {
    let _ = std::process::Command::new("pactl").args(["unload-module", id]).stderr(Stdio::null()).status();
}

/// Removes Orither's sound devices, including any that a crash or a kill left behind.
pub fn remove_devices() {
    let Ok(output) = std::process::Command::new("pactl").args(["list", "short", "modules"]).output() else {
        return;
    };
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let mut fields = line.splitn(3, '\t');
        let (Some(id), Some(arguments)) = (fields.next(), fields.nth(1)) else { continue };
        let device = arguments.split_whitespace().find_map(|argument| {
            argument.strip_prefix("sink_name=").or_else(|| argument.strip_prefix("source_name="))
        });
        if device.is_some_and(|device| [SPEAKER, MICROPHONE].contains(&device)) {
            unload_module(id);
        }
    }
}

/// Follows whether a program on this computer records from a sound device.
struct Listeners {
    device: u64,
    events: Lines<BufReader<ChildStdout>>,
    _subscription: Child,
    recording: bool,
    /// An event came that may have changed `recording`, and it is not checked yet.
    unchecked: bool,
}

impl Listeners {
    async fn watch(name: &str) -> Result<Self> {
        // Subscribing first means no change goes unseen while this looks.
        let mut subscription =
            Command::new("pactl").arg("subscribe").stdout(Stdio::piped()).kill_on_drop(true).spawn().context("running pactl")?;
        let events = BufReader::new(subscription.stdout.take().context("pactl gave no events")?).lines();
        let sources: Vec<Value> = serde_json::from_slice(&pactl(&["-f", "json", "list", "short", "sources"]).await?.stdout)?;
        let device = sources
            .iter()
            .find(|source| source["name"] == name)
            .and_then(|source| source["index"].as_u64())
            .context("the sound device went away")?;
        Ok(Self { device, events, _subscription: subscription, recording: false, unchecked: true })
    }

    /// Waits until `recording` changes. Safe to drop at any point, as `select!` does.
    async fn change(&mut self) -> Result<()> {
        loop {
            if self.unchecked {
                let recording = self.anyone().await?;
                self.unchecked = false;
                if recording != self.recording {
                    self.recording = recording;
                    return Ok(());
                }
            }
            let event = self.events.next_line().await?.context("the sound server stopped")?;
            self.unchecked = event.contains("source-output");
        }
    }

    /// Level meters, like the ones in the desktop's sound settings, do not count.
    async fn anyone(&self) -> Result<bool> {
        let streams: Vec<Value> = serde_json::from_slice(&pactl(&["-f", "json", "list", "source-outputs"]).await?.stdout)?;
        Ok(streams.iter().any(|stream| {
            stream["source"].as_u64() == Some(self.device)
                && stream["corked"] == false
                && stream["properties"]["stream.monitor"] != "true"
        }))
    }
}

/// Hands the phone's sound to the device while a program records, and throws it away
/// while none does. Returns true once the phone stops sending, and false once nobody has
/// recorded for a while.
async fn pass_sound(sound: &mut (impl AsyncRead + Unpin), pipe: &mut pipe::Sender, listeners: &mut Listeners) -> Result<bool> {
    let mut buffer = [0; WHOLE_WRITE];
    let mut filled = 0;
    let mut was_recording = true;
    let mut quiet_until = Instant::now();
    loop {
        if was_recording && !listeners.recording {
            quiet_until = Instant::now() + STAY_ON_FOR;
        }
        was_recording = listeners.recording;

        let whole = filled - filled % FRAME;
        if listeners.recording && whole > 0 {
            tokio::select! {
                wrote = pipe.write_all(&buffer[..whole]) => wrote?,
                changed = listeners.change() => {
                    changed?;
                    continue;
                }
            }
        }
        buffer.copy_within(whole..filled, 0);
        filled -= whole;

        tokio::select! {
            read = sound.read(&mut buffer[filled..]) => match read? {
                0 => return Ok(true),
                read => filled += read,
            },
            changed = listeners.change() => changed?,
            () = sleep_until(quiet_until), if !listeners.recording => return Ok(false),
        }
    }
}

fn pactl_missing() -> bool {
    env::var_os("PATH").is_none_or(|paths| !env::split_paths(&paths).any(|path| path.join("pactl").exists()))
}

/// Finds the device `modprobe v4l2loopback card_label="Orither Camera"` made.
fn camera_device() -> Option<PathBuf> {
    fs::read_dir("/sys/class/video4linux").ok()?.flatten().find_map(|entry| {
        let name = fs::read_to_string(entry.path().join("name")).ok()?;
        (name.trim() == CAMERA_LABEL).then(|| Path::new("/dev").join(entry.file_name()))
    })
}

pub fn blocked(id: &str) -> Option<String> {
    match id {
        "camera" if camera_device().is_none() => Some(camera_setup::missing().explain()),
        "microphone" | "speaker" if pactl_missing() => {
            Some("Needs PipeWire or PulseAudio, with the pactl command installed.".into())
        }
        _ => None,
    }
}

/// Whether one setup here, which asks for the password, would unblock the device.
pub fn can_set_up(id: &str) -> bool {
    id == "camera" && camera_device().is_none() && camera_setup::missing() == camera_setup::Missing::Setup
}

pub async fn set_up(id: &str) -> Result<()> {
    ensure!(id == "camera", "{id} needs no setup");
    camera_setup::set_up().await?;
    // udev creates the device a moment after the driver loads.
    for _ in 0..20 {
        if camera_device().is_some() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Ok(())
}

pub async fn run(id: &str, phone: &Phone, session: Session) -> Result<()> {
    match id {
        "microphone" => microphone(phone, session).await,
        "speaker" => speaker(session).await,
        "camera" => camera(phone, session).await,
        other => bail!("no device {other}"),
    }
}

async fn capture(phone: &Phone, session: &Session, options: Vec<String>) -> Result<Value> {
    ensure_engine(phone, session).await?;
    session.call("screen", "start", json!({ "options": options, "channels": 1 })).await
}

/// The phone stops capturing when this is dropped, however the device ended.
struct Capture {
    session: Session,
    id: Value,
}

impl Drop for Capture {
    fn drop(&mut self) {
        let (session, id) = (self.session.clone(), self.id.clone());
        async_runtime::spawn(async move {
            let _ = session.call("screen", "stop", json!({ "session": id })).await;
        });
    }
}

/// The device stays listed, and the phone only listens while a program records from it,
/// as a built-in microphone would.
async fn microphone(phone: &Phone, session: Session) -> Result<()> {
    let file = sound_folder()?.join("microphone");
    let options = ["video=false", "audio=true", "audio_source=mic", "audio_codec=raw", "raw_stream=true", "control=false"];
    loop {
        // PipeWire keeps the last moment of sound a source passed on and plays it when the
        // next recording starts, so each time the phone listens gets a fresh device.
        let _device = Module::load("module-pipe-source", MICROPHONE, "Orither Microphone", &file).await?;
        let mut pipe = pipe::OpenOptions::new().read_write(true).open_sender(&file)?;
        let mut listeners = Listeners::watch(MICROPHONE).await?;
        while !listeners.recording {
            listeners.change().await?;
        }
        tracing::info!("a program records from the microphone, so the phone listens");
        let id = capture(phone, &session, options.map(str::to_owned).to_vec()).await?;
        let capture = Capture { session: session.clone(), id };
        let mut sound = capture.session.open("screen", "attach", json!({ "session": capture.id, "channel": 0 })).await?;
        if pass_sound(&mut sound.recv, &mut pipe, &mut listeners).await? {
            return Ok(());
        }
        tracing::info!("nothing records from the microphone now, so the phone stops listening");
    }
}

async fn speaker(session: Session) -> Result<()> {
    let file = sound_folder()?.join("speaker");
    let _device = Module::load("module-pipe-sink", SPEAKER, "Orither Speaker", &file).await?;
    let mut pipe = pipe::OpenOptions::new().read_write(true).open_receiver(&file)?;
    let mut stream = session.open("speaker", "play", json!({})).await?;
    io::copy(&mut pipe, &mut stream.send).await?;
    stream.send.flush().await?;
    Ok(())
}

/// A picture from the phone, fitted to the webcam.
struct Picture {
    /// The size the phone took it at.
    size: Size,
    bytes: Vec<u8>,
}

/// How a spell of filming ended.
enum Filmed {
    /// The phone stopped sending.
    Stopped,
    /// Nothing has used the webcam for a while. Holds the phone's picture size, if a
    /// picture came.
    Unused(Option<Size>),
}

/// The webcam stays listed, and the phone only films while a program uses it, as a
/// built-in webcam would.
async fn camera(phone: &Phone, session: Session) -> Result<()> {
    let path = camera_device().context("the camera device went away")?;
    let size = preferences::read(&phone.app, CAMERA_PICTURE_SIZE).unwrap_or(CAMERA_FIRST_SIZE);
    let mut webcam = Webcam::open(&path, size)?;
    loop {
        while !webcam.in_use {
            webcam.change().await?;
        }
        tracing::info!("a program uses the webcam, so the phone films");
        let id = capture(phone, &session, camera_options(session.route())).await?;
        let capture = Capture { session: session.clone(), id };
        let attach = json!({ "session": capture.id, "channel": 0, "paced": true });
        let Duplex { mut send, mut recv } = capture.session.open("screen", "attach", attach).await?;
        let (paced, codec) = video::start(&mut recv).await.context("the phone's camera stopped as it started")?;
        // Pictures pass through here so that the phone hears what arrived, which keeps them live.
        let (mut to_film, from_link) = tokio::io::duplex(64 * 1024);
        async_runtime::spawn(async move { video::receive(&mut recv, paced.then_some(&mut send), &mut to_film).await });
        let (sender, mut pictures) = mpsc::channel(1);
        let (reader, size) = (SyncIoBridge::new(from_link), webcam.size());
        let filming = async_runtime::spawn_blocking(move || film(reader, codec, size, sender));
        let Filmed::Unused(filmed_size) = pass_pictures(&mut pictures, &mut webcam).await? else {
            return filming.await.context("the camera stopped")?;
        };
        tracing::info!("nothing uses the webcam now, so the phone stops filming");
        // The next program to start gets the last picture first, which is old by then.
        webcam.blank()?;
        // Programs keep the size the webcam had when they started, so it only changes
        // while none uses it.
        if let Some(size) = filmed_size.filter(|&size| size != webcam.size()) {
            preferences::write(&phone.app, CAMERA_PICTURE_SIZE, &size);
            drop(webcam);
            webcam = Webcam::open(&path, size)?;
        }
    }
}

/// What the phone films with, for what the route can carry. Bluetooth moves about
/// 500 kbit/s, and a picture that outruns the link arrives ever later.
fn camera_options(route: Route) -> Vec<String> {
    let bit_rate = match route {
        Route::Local => 4_000_000,
        Route::Internet | Route::Relay => 2_500_000,
        Route::Nearby => 350_000,
    };
    [
        "video=true",
        "audio=false",
        "video_source=camera",
        "video_codec=h264",
        "control=false",
        "send_device_meta=false",
        CAMERA_KEY_FRAMES,
    ]
    .map(str::to_owned)
    .into_iter()
    .chain([format!("video_bit_rate={bit_rate}"), format!("max_size={CAMERA_SIZE}")])
    .collect()
}

/// Shows the phone's pictures on the webcam, and black until the first one comes.
async fn pass_pictures(pictures: &mut mpsc::Receiver<Picture>, webcam: &mut Webcam) -> Result<Filmed> {
    let mut filmed_size = None;
    let mut blank = interval(BLANK_EVERY);
    let mut was_in_use = true;
    let mut quiet_until = Instant::now();
    loop {
        if was_in_use && !webcam.in_use {
            quiet_until = Instant::now() + STAY_ON_FOR;
        }
        was_in_use = webcam.in_use;

        tokio::select! {
            picture = pictures.recv() => {
                let Some(picture) = picture else { return Ok(Filmed::Stopped) };
                filmed_size = Some(picture.size);
                webcam.show(&picture.bytes)?;
            }
            _ = blank.tick(), if filmed_size.is_none() => webcam.blank()?,
            changed = webcam.change() => changed?,
            () = sleep_until(quiet_until), if !webcam.in_use => return Ok(Filmed::Unused(filmed_size)),
        }
    }
}

/// Decodes the phone's video into pictures the webcam's size, until the phone stops
/// sending or the webcam stops taking them.
fn film(stream: impl std::io::Read, codec: [u8; 4], size: Size, pictures: mpsc::Sender<Picture>) -> Result<()> {
    let mut reader = Reader::with_codec(stream, codec)?;
    let mut decoder = H264::new()?;
    let mut stats = Stats::new("camera");
    while let Some(packet) = reader.next()? {
        let Packet::Media { config, key, pts, data } = packet else { continue };
        if !config {
            stats.arrived(pts, key, data.len());
        }
        let decoding = std::time::Instant::now();
        let decoded = decoder.decode(&data)?;
        stats.decoded(decoding.elapsed());
        let Some(frame) = decoded else { continue };
        let own = Size { width: frame.width, height: frame.height };
        if pictures.blocking_send(Picture { size: own, bytes: webcam::fitted(&frame, size) }).is_err() {
            break;
        }
    }
    Ok(())
}
