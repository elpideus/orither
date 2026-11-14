//! The window itself: video comes in on one thread, input goes out on another,
//! and the event loop in between draws the newest picture.

use std::{
    fs::File,
    io::{self, Read, Write},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        mpsc::{self, Sender},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalSize, PhysicalSize},
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
    window::{Icon, Window, WindowId},
};

use crate::{
    control,
    decode::{Frame, H264},
    input::Input,
    render::{Renderer, Viewport},
    stream::{Packet, Reader},
};

/// Waits for a window drag to settle before the phone redraws at the new size.
const RESIZE_SETTLES: Duration = Duration::from_millis(180);
const START_SIZE: LogicalSize<u32> = LogicalSize::new(420, 880);
const FLEX_START_SIZE: LogicalSize<u32> = LogicalSize::new(1100, 720);
const SHARE_OF_SCREEN: f64 = 0.85;

pub struct Options {
    pub title: String,
    /// What the desktop knows this window by, tying it to its `.desktop` file.
    pub app_id: String,
    pub icon: Option<PathBuf>,
    /// Whether the phone's display follows the size of the window.
    pub flex: bool,
}

impl Options {
    /// How one process asks another for this window, read back by [`Self::from_args`].
    pub fn to_args(&self) -> Vec<String> {
        let icon = self.icon.as_ref().map(|icon| icon.to_string_lossy().into_owned());
        let flex = if self.flex { "flex" } else { "fixed" };
        vec![self.title.clone(), self.app_id.clone(), icon.unwrap_or_default(), flex.to_owned()]
    }

    pub fn from_args(mut args: impl Iterator<Item = String>) -> Option<Self> {
        let (title, app_id, icon, flex) = (args.next()?, args.next()?, args.next()?, args.next()?);
        let icon = (!icon.is_empty()).then(|| icon.into());
        Some(Self { title, app_id, icon, flex: flex == "flex" })
    }
}

enum Wake {
    Picture,
    Size(u32, u32),
    Ended,
}

type Latest = Arc<Mutex<Option<Frame>>>;

fn receive(input: impl Read, latest: &Latest, wake: &EventLoopProxy<Wake>) -> Result<()> {
    let mut reader = Reader::new(input)?;
    let mut decoder = H264::new()?;
    while let Some(packet) = reader.next()? {
        match packet {
            Packet::Session { width, height } => {
                let _ = wake.send_event(Wake::Size(width, height));
            }
            Packet::Media { data, .. } => {
                if let Some(frame) = decoder.decode(&data)? {
                    *latest.lock().unwrap() = Some(frame);
                    let _ = wake.send_event(Wake::Picture);
                }
            }
        }
    }
    Ok(())
}

/// Writes on a thread of its own, so that a slow link never stalls the window.
fn start_control() -> Sender<Vec<u8>> {
    let (sender, messages) = mpsc::channel::<Vec<u8>>();
    thread::spawn(move || {
        let mut output = io::stdout().lock();
        while let Ok(message) = messages.recv() {
            if output.write_all(&message).and_then(|()| output.flush()).is_err() {
                break;
            }
        }
    });
    sender
}

fn load_icon(path: &PathBuf) -> Result<Icon> {
    let mut reader = png::Decoder::new(io::BufReader::new(File::open(path)?)).read_info()?;
    let mut pixels = vec![0; reader.output_buffer_size().context("icon too large")?];
    let info = reader.next_frame(&mut pixels)?;
    pixels.truncate(info.buffer_size());
    Ok(Icon::from_rgba(pixels, info.width, info.height)?)
}

struct Viewer {
    options: Options,
    latest: Latest,
    control: Sender<Vec<u8>>,
    input: Input,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    video: (u32, u32),
    fitted: bool,
    resize_at: Option<Instant>,
    failure: Option<anyhow::Error>,
}

impl Viewer {
    fn viewport(&self) -> Viewport {
        let size = self.window.as_ref().map_or(PhysicalSize::new(1, 1), |window| window.inner_size());
        Viewport::fit(size, self.video)
    }

    fn send(&self, messages: Vec<Vec<u8>>) {
        for message in messages {
            let _ = self.control.send(message);
        }
    }

    fn open(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        let size = if self.options.flex { FLEX_START_SIZE } else { START_SIZE };
        let mut attributes = Window::default_attributes().with_title(&self.options.title).with_inner_size(size);
        if let Some(icon) = &self.options.icon {
            let icon = load_icon(icon).ok();
            // Windows shows the big icon in Alt+Tab, where the small one would be blurred up to size.
            #[cfg(windows)]
            {
                use winit::platform::windows::WindowAttributesExtWindows;
                attributes = attributes.with_taskbar_icon(icon.clone());
            }
            attributes = attributes.with_window_icon(icon);
        }
        #[cfg(target_os = "linux")]
        {
            use winit::platform::{wayland::WindowAttributesExtWayland, x11::WindowAttributesExtX11};
            let name = &self.options.app_id;
            attributes = WindowAttributesExtWayland::with_name(attributes, name, name);
            attributes = WindowAttributesExtX11::with_name(attributes, name, name);
        }
        let window = Arc::new(event_loop.create_window(attributes).context("opening the window")?);
        self.renderer = Some(Renderer::new(window.clone())?);
        self.window = Some(window);
        Ok(())
    }

    /// Gives a mirrored phone a window of its own shape, once, at a sensible size.
    fn fit_window(&mut self, event_loop: &ActiveEventLoop) {
        let Some(window) = &self.window else { return };
        if self.fitted || self.options.flex || self.video.1 == 0 {
            return;
        }
        self.fitted = true;
        let screen = window.current_monitor().or_else(|| event_loop.primary_monitor());
        let room = screen.map_or(1000, |screen| screen.size().height);
        let height = f64::from(room) * SHARE_OF_SCREEN;
        let width = height * f64::from(self.video.0) / f64::from(self.video.1);
        let _ = window.request_inner_size(PhysicalSize::new(width as u32, height as u32));
    }

    fn draw(&mut self) {
        let viewport = self.viewport();
        let Some(renderer) = &mut self.renderer else { return };
        if let Some(frame) = self.latest.lock().unwrap().take() {
            renderer.upload(&frame);
        }
        if let Err(err) = renderer.draw(viewport) {
            self.failure = Some(err);
        }
    }
}

impl ApplicationHandler<Wake> for Viewer {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none()
            && let Err(err) = self.open(event_loop)
        {
            self.failure = Some(err);
            event_loop.exit();
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, wake: Wake) {
        match wake {
            Wake::Picture => {
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            Wake::Size(width, height) => {
                self.video = (width, height);
                self.fit_window(event_loop);
            }
            Wake::Ended => event_loop.exit(),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let viewport = self.viewport();
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => self.draw(),
            WindowEvent::Resized(size) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size);
                }
                if self.options.flex {
                    self.resize_at = Some(Instant::now() + RESIZE_SETTLES);
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => self.input.modifiers(modifiers.state()),
            WindowEvent::CursorMoved { position, .. } => {
                let messages = self.input.moved(position.x, position.y, viewport);
                self.send(messages);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let messages = self.input.button(button, state, viewport);
                self.send(messages);
            }
            WindowEvent::MouseWheel { delta, .. } => self.send(self.input.wheel(delta, viewport)),
            WindowEvent::KeyboardInput { event, .. } => self.send(self.input.key(&event)),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(when) = self.resize_at else {
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        };
        if Instant::now() < when {
            event_loop.set_control_flow(ControlFlow::WaitUntil(when));
            return;
        }
        self.resize_at = None;
        if let Some(window) = &self.window {
            let size = window.inner_size();
            self.send(vec![control::resize_display(size.width as u16, size.height as u16)]);
        }
    }
}

/// Shows the video from stdin until it ends or the user closes the window.
pub fn run(options: Options) -> Result<()> {
    #[cfg(windows)]
    take_app_id(&options);
    let event_loop = EventLoop::<Wake>::with_user_event().build().context("reaching the display")?;
    let wake = event_loop.create_proxy();
    let latest = Latest::default();

    let incoming = latest.clone();
    thread::spawn(move || {
        if let Err(err) = receive(io::stdin().lock(), &incoming, &wake) {
            eprintln!("video stream: {err:#}");
        }
        let _ = wake.send_event(Wake::Ended);
    });

    let mut viewer = Viewer {
        options,
        latest,
        control: start_control(),
        input: Input::default(),
        window: None,
        renderer: None,
        video: (0, 0),
        fitted: false,
        resize_at: None,
        failure: None,
    };
    event_loop.run_app(&mut viewer).context("running the window")?;
    viewer.failure.map_or(Ok(()), Err)
}

/// Windows groups windows on the taskbar by their process's app id, the way Linux
/// desktops go by `app_id`. An app's window takes the id of the Start-menu shortcut
/// Orither wrote for it, so it gets a button of its own with the shortcut's name and
/// icon, which pins as that shortcut. Only an app has an icon, and a shortcut: the
/// phone's mirror keeps Orither's id, and so Orither's button. This has to happen
/// before the first window opens.
#[cfg(windows)]
fn take_app_id(options: &Options) {
    use windows_sys::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;

    if options.icon.is_none() || options.app_id.is_empty() {
        return;
    }
    let id: Vec<u16> = options.app_id.encode_utf16().chain([0]).collect();
    // SAFETY: `id` ends with a zero and outlives the call, which copies it.
    let result = unsafe { SetCurrentProcessExplicitAppUserModelID(id.as_ptr()) };
    if result < 0 {
        eprintln!("taking the app id {}: error {result:#010x}", options.app_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_survive_the_trip_between_processes() {
        let options = Options { title: "Maps".into(), app_id: "orither-maps".into(), icon: None, flex: true };
        let back = Options::from_args(options.to_args().into_iter()).unwrap();
        assert_eq!((back.title.as_str(), back.app_id.as_str()), ("Maps", "orither-maps"));
        assert!(back.icon.is_none() && back.flex);
    }
}
