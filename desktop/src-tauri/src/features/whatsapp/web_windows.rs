//! WhatsApp Web on Windows: WhatsApp's own site, in a WebView2 view laid over the
//! WhatsApp page. It loads web.whatsapp.com as Edge would, with the engine's own
//! user agent, and adds nothing to the page: no script, and no style sheet either,
//! since WebView2 has no user style sheets, so WhatsApp Web keeps its own look.
//! Only its light or dark follows Orither's, through WebView2's own setting.
//!
//! The view is made with WebView2 itself rather than as a Tauri webview, since
//! Tauri adds its own scripts to every page it opens, to talk to the app. As on
//! Linux, it sits in a window of its own over the main window's content, moves
//! where the page keeps room, and hides when the page keeps none. Its login and
//! cache live in a profile folder of its own. Everything here runs on the main
//! thread, where WebView2 calls back.

use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    path::PathBuf,
    rc::Rc,
};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};
use tauri::{AppHandle, Listener, Manager, Url};
use tauri_plugin_opener::OpenerExt;
use webview2_com::{
    ContextMenuRequestedEventHandler, CoreWebView2EnvironmentOptions, CreateCoreWebView2ControllerCompletedHandler,
    CreateCoreWebView2EnvironmentCompletedHandler, DocumentTitleChangedEventHandler, DownloadStartingEventHandler,
    FocusChangedEventHandler, Microsoft::Web::WebView2::Win32::*, NavigationCompletedEventHandler,
    NavigationStartingEventHandler, NewWindowRequestedEventHandler, NotificationCloseRequestedEventHandler,
    NotificationReceivedEventHandler, PermissionRequestedEventHandler, ProcessFailedEventHandler,
    SetPermissionStateCompletedHandler, StateChangedEventHandler, take_pwstr,
};
use windows::{
    Win32::{
        Foundation::{E_POINTER, HWND, LPARAM, LRESULT, RECT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            HiDpi::GetDpiForWindow,
            Input::KeyboardAndMouse::SetFocus,
            Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DestroyWindow, GetForegroundWindow, HWND_TOP, RegisterClassExW,
                SW_HIDE, SW_SHOWNA, SWP_NOACTIVATE, SetWindowPos, ShowWindow, USER_DEFAULT_SCREEN_DPI, WA_ACTIVE,
                WA_INACTIVE, WINDOW_EX_STYLE, WM_ACTIVATE, WM_DESTROY, WM_DPICHANGED, WM_MOVE, WM_NCDESTROY, WNDCLASSEXW,
                WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS,
            },
        },
    },
    core::{BOOL, HSTRING, IUnknown, Interface, PCWSTR, PWSTR, w},
};

use super::{
    Chat,
    common::{
        FOLDER, HOME, HOST, OUTSIDE, QUIET_KEPT, START_AFTER, State, Status, free_name, is_about, on_main, phone_number,
        reveal, unread,
    },
};
use crate::{
    features::{Phone, notifications},
    theme,
};

/// What a WebView2 event handler gives back.
type Handled = windows::core::Result<()>;

/// WhatsApp Web's origin, as WebView2 keeps permissions by it.
const ORIGIN: &str = "https://web.whatsapp.com";
/// The same as Tauri gives Orither's own view: no Edge menu over selected text,
/// and no SmartScreen.
const BROWSER_ARGUMENTS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection";
/// What WhatsApp gets when it asks: to notify, the microphone and camera for
/// voice messages and photos, several downloads at once and sound before a
/// click, which the Linux view has without asking.
const ALLOWED: [COREWEBVIEW2_PERMISSION_KIND; 5] = [
    COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS,
    COREWEBVIEW2_PERMISSION_KIND_MICROPHONE,
    COREWEBVIEW2_PERMISSION_KIND_CAMERA,
    COREWEBVIEW2_PERMISSION_KIND_MULTIPLE_AUTOMATIC_DOWNLOADS,
    COREWEBVIEW2_PERMISSION_KIND_AUTOPLAY,
];
/// What the context menu leaves out: there are no other windows, and no history.
const LEFT_OUT: [&str; 3] = ["back", "forward", "openLinkInNewWindow"];
/// The class of the window that holds the view.
const FRAME_CLASS: PCWSTR = w!("OritherWhatsAppWeb");
/// How the view follows the main window, as one of its subclasses.
const FOLLOW: usize = 0x5741_5742;

/// Where the page keeps room for the view, in its CSS pixels. The page sends its
/// corners' radius too, but the view's corners stay square: a window can only be
/// clipped round without smoothing, and nothing is painted over WebView2's.
#[derive(Deserialize)]
struct Place {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    /// The colour around it, in CSS.
    behind: String,
}

/// What WebView2 made: the window that holds the view, and the view.
struct View {
    frame: HWND,
    controller: ICoreWebView2Controller,
    core: ICoreWebView2,
}

impl View {
    fn new(frame: HWND, controller: ICoreWebView2Controller) -> Result<Self> {
        let core = unsafe { controller.CoreWebView2()? };
        // Hidden until WhatsApp Web is there and the page keeps room for it.
        unsafe { controller.SetIsVisible(false)? };
        Ok(Self { frame, controller, core })
    }

    fn close(&self) {
        if let Err(err) = unsafe { self.controller.Close() } {
            tracing::warn!("closing WhatsApp Web: {err}");
        }
        destroy(self.frame);
    }
}

/// One of WhatsApp Web's notifications that the phone's own stood in for. A click
/// on the phone's opens the chat through it, as WhatsApp would.
struct Quiet {
    serial: u64,
    notification: ICoreWebView2Notification,
    title: String,
    tag: String,
}

struct Web {
    phone: Phone,
    /// The main window, which the view sits in.
    window: HWND,
    /// Once WebView2 has made it.
    view: RefCell<Option<View>>,
    making: Cell<bool>,
    /// What the view loads once made, when not WhatsApp Web's start.
    next: RefCell<Option<String>>,
    status: RefCell<Status>,
    /// Where the page keeps room for the view, while it does.
    place: RefCell<Option<Place>>,
    shown: Cell<bool>,
    /// Whether typing goes to WhatsApp Web.
    focused: Cell<bool>,
    /// Whether it goes back there once the window is in front again.
    refocus: Cell<bool>,
    /// The page load under way, by WebView2's id for it.
    loading: Cell<Option<u64>>,
    quiet: RefCell<VecDeque<Quiet>>,
    /// The last number given to a kept notification.
    serial: Cell<u64>,
}

thread_local! {
    static WEB: RefCell<Option<Rc<Web>>> = const { RefCell::new(None) };
}

/// WhatsApp Web, once started. Window messages ask too, so this never fails,
/// even as the thread ends.
fn current() -> Option<Rc<Web>> {
    WEB.try_with(|web| web.try_borrow().ok().and_then(|web| web.clone())).ok().flatten()
}

/// Starts WhatsApp Web a little after Orither, if it was used before.
pub fn install(phone: &Phone) {
    if !folder(&phone.app).is_ok_and(|folder| folder.exists()) {
        return;
    }
    let phone = phone.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(START_AFTER).await;
        let started: Result<()> = async { on_main(&phone, |phone| start(phone).map(drop))?.await? }.await;
        if let Err(err) = started {
            tracing::warn!("starting WhatsApp Web: {err:#}");
        }
    });
}

/// WhatsApp Web's own profile: its login, cookies and cache, which WebView2 keeps
/// together. It is under the local app data, as Orither's own view's is, since a
/// cache has no place in a roaming profile.
fn folder(app: &AppHandle) -> Result<PathBuf> {
    Ok(app.path().app_local_data_dir()?.join(FOLDER))
}

/// WhatsApp Web, started the first time it is needed. WebView2 makes its view a
/// moment later.
fn start(phone: &Phone) -> Result<Rc<Web>> {
    if let Some(web) = current() {
        return Ok(web);
    }
    let window = phone.app.get_webview_window("main").context("WhatsApp Web has no window to sit in")?.hwnd()?;
    let web = Rc::new(Web {
        phone: phone.clone(),
        window,
        view: RefCell::default(),
        making: Cell::default(),
        next: RefCell::default(),
        status: RefCell::new(Status::OFF),
        place: RefCell::default(),
        shown: Cell::default(),
        focused: Cell::default(),
        refocus: Cell::default(),
        loading: Cell::default(),
        quiet: RefCell::default(),
        serial: Cell::default(),
    });
    WEB.set(Some(web.clone()));
    if !unsafe { SetWindowSubclass(window, Some(follow), FOLLOW, 0) }.as_bool() {
        tracing::warn!("WhatsApp Web cannot follow Orither's window");
    }

    // Orither's colours follow the desktop's, and WhatsApp Web's light or dark follows them.
    let app = phone.app.clone();
    phone.app.listen_any(theme::CHANGED, move |_| {
        let _ = app.run_on_main_thread(|| {
            if let Some(web) = current() {
                web.follow_theme();
            }
        });
    });
    web.make();
    Ok(web)
}

impl Web {
    /// Has WebView2 make the view, unless it is there or on its way.
    fn make(self: &Rc<Self>) {
        if self.making.get() || self.core().is_some() {
            return;
        }
        self.making.set(true);
        self.update(|status| {
            status.state = State::Opening;
            status.error = None;
        });
        if let Err(err) = self.make_browser() {
            self.unmade(&err);
        }
    }

    fn unmade(self: &Rc<Self>, err: &anyhow::Error) {
        self.making.set(false);
        self.failed(format!("WhatsApp Web could not start: {err:#}"));
    }

    /// A WebView2 browser for WhatsApp Web alone, on its own profile.
    fn make_browser(self: &Rc<Self>) -> Result<()> {
        let folder = folder(&self.phone.app)?;
        std::fs::create_dir_all(&folder)?;
        let options = CoreWebView2EnvironmentOptions::default();
        unsafe { options.set_additional_browser_arguments(BROWSER_ARGUMENTS.to_owned()) };
        let web = Rc::downgrade(self);
        let made = CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(move |result, browser| {
            let Some(web) = web.upgrade() else { return Ok(()) };
            let browser = result.and_then(|()| browser.ok_or_else(|| E_POINTER.into()));
            if let Err(err) = browser.map_err(anyhow::Error::from).and_then(|browser| web.make_view(&browser)) {
                web.unmade(&err);
            }
            Ok(())
        }));
        let options = ICoreWebView2EnvironmentOptions::from(options);
        unsafe { CreateCoreWebView2EnvironmentWithOptions(PCWSTR::null(), &HSTRING::from(folder.as_path()), &options, &made)? };
        Ok(())
    }

    /// The view, in a window of its own over the main window's content.
    fn make_view(self: &Rc<Self>, browser: &ICoreWebView2Environment) -> Result<()> {
        let frame = frame(self.window)?;
        let web = Rc::downgrade(self);
        let made = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(move |result, controller| {
            let controller = result.and_then(|()| controller.ok_or_else(|| E_POINTER.into()));
            match web.upgrade() {
                Some(web) => web.made(frame, controller.map_err(anyhow::Error::from)),
                None => destroy(frame),
            }
            Ok(())
        }));
        if let Err(err) = unsafe { browser.CreateCoreWebView2Controller(frame, &made) } {
            destroy(frame);
            return Err(err.into());
        }
        Ok(())
    }

    /// Sets the view up, then loads WhatsApp Web, or the chat asked for meanwhile.
    fn made(self: &Rc<Self>, frame: HWND, controller: Result<ICoreWebView2Controller>) {
        let view = match controller.and_then(|controller| View::new(frame, controller)) {
            Ok(view) => view,
            Err(err) => {
                destroy(frame);
                return self.unmade(&err);
            }
        };
        if let Err(err) = self.listen(&view) {
            view.close();
            return self.unmade(&anyhow::Error::from(err));
        }
        self.making.set(false);
        if let Some(behind) = colour(&theme::current().window) {
            background(&view.controller, behind);
        }
        self.view.replace(Some(view));
        self.follow_theme();
        self.relay();
        let next = self.next.take();
        self.load(next.as_deref().unwrap_or(HOME));
    }

    fn listen(self: &Rc<Self>, view: &View) -> Handled {
        let (core, controller, mut token) = (&view.core, &view.controller, 0);
        unsafe {
            core.add_NavigationStarting(&NavigationStartingEventHandler::create(handler(self, Self::navigating)), &mut token)?;
            core.add_NavigationCompleted(&NavigationCompletedEventHandler::create(handler(self, Self::navigated)), &mut token)?;
            core.add_DocumentTitleChanged(&DocumentTitleChangedEventHandler::create(handler(self, Self::titled)), &mut token)?;
            core.add_NewWindowRequested(&NewWindowRequestedEventHandler::create(handler(self, Self::new_window)), &mut token)?;
            core.add_PermissionRequested(&PermissionRequestedEventHandler::create(handler(self, Self::asked)), &mut token)?;
            core.add_ProcessFailed(&ProcessFailedEventHandler::create(handler(self, Self::crashed)), &mut token)?;
            controller.add_GotFocus(&FocusChangedEventHandler::create(handler(self, Self::focus_in)), &mut token)?;
            controller.add_LostFocus(&FocusChangedEventHandler::create(handler(self, Self::focus_out)), &mut token)?;
        }
        // Newer parts of WebView2, which an old runtime may lack. Without one,
        // only what it does is missing.
        let newer = [
            ("saving downloads", self.save_downloads(core)),
            ("holding back notifications", self.hold_notifications(core)),
            ("trimming the context menu", self.trim_menu(core)),
            ("letting WhatsApp notify", let_notify(core)),
        ];
        for (what, done) in newer {
            if let Err(err) = done {
                tracing::warn!("WhatsApp Web, {what}: {err}");
            }
        }
        Ok(())
    }

    fn save_downloads(self: &Rc<Self>, core: &ICoreWebView2) -> Handled {
        let core = core.cast::<ICoreWebView2_4>()?;
        unsafe { core.add_DownloadStarting(&DownloadStartingEventHandler::create(handler(self, Self::downloading)), &mut 0) }
    }

    fn hold_notifications(self: &Rc<Self>, core: &ICoreWebView2) -> Handled {
        let core = core.cast::<ICoreWebView2_24>()?;
        let notified = NotificationReceivedEventHandler::create(handler(self, Self::notified));
        unsafe { core.add_NotificationReceived(&notified, &mut 0) }
    }

    fn trim_menu(self: &Rc<Self>, core: &ICoreWebView2) -> Handled {
        let core = core.cast::<ICoreWebView2_11>()?;
        unsafe { core.add_ContextMenuRequested(&ContextMenuRequestedEventHandler::create(handler(self, Self::menu)), &mut 0) }
    }

    fn core(&self) -> Option<ICoreWebView2> {
        self.view.try_borrow().ok()?.as_ref().map(|view| view.core.clone())
    }

    /// The window that holds the view, and the view's controls. Window messages
    /// ask too, so this never fails.
    fn parts(&self) -> Option<(HWND, ICoreWebView2Controller)> {
        self.view.try_borrow().ok()?.as_ref().map(|view| (view.frame, view.controller.clone()))
    }

    /// Loads WhatsApp Web, or a page of it, once the view is there.
    fn load(self: &Rc<Self>, url: &str) {
        let Some(core) = self.core() else {
            self.next.replace(Some(url.to_owned()));
            return self.make();
        };
        if let Err(err) = unsafe { core.Navigate(&HSTRING::from(url)) } {
            self.failed(format!("WhatsApp Web did not load: {err}"));
        }
    }

    fn update(self: &Rc<Self>, change: impl FnOnce(&mut Status)) {
        let changed = {
            let mut status = self.status.borrow_mut();
            let before = status.clone();
            change(&mut status);
            (*status != before).then(|| status.clone())
        };
        if let Some(status) = changed {
            self.phone.tell_ui("whatsapp", "web", json!(status));
            self.show_if_ready();
        }
    }

    fn failed(self: &Rc<Self>, error: String) {
        tracing::warn!("WhatsApp Web: {error}");
        self.update(|status| {
            status.state = State::Failed;
            status.error = Some(error);
        });
    }

    /// Lets go of a view whose browser is gone. Trying again makes another.
    fn lose(self: &Rc<Self>, error: &str) {
        self.conceal();
        if let Some(view) = self.view.take() {
            view.close();
        }
        self.loading.set(None);
        self.quiet.borrow_mut().clear();
        self.failed(error.to_owned());
    }

    /// Lays the view over the page's room. The page measures in CSS pixels, and
    /// the window in the screen's, which are as many more as the screen's scale.
    fn lay(&self, place: &Place) {
        let Some((frame, controller)) = self.parts() else { return };
        let dpi = unsafe { GetDpiForWindow(self.window) };
        let scale = if dpi == 0 { 1.0 } else { f64::from(dpi) / f64::from(USER_DEFAULT_SCREEN_DPI) };
        let [x, y, width, height] =
            [place.x, place.y, place.width, place.height].map(|length| (f64::from(length) * scale).round() as i32);
        let (width, height) = (width.max(0), height.max(0));
        unsafe {
            if let Err(err) = SetWindowPos(frame, Some(HWND_TOP), x, y, width, height, SWP_NOACTIVATE) {
                tracing::warn!("placing WhatsApp Web: {err}");
            }
            if let Err(err) = controller.SetBounds(RECT { left: 0, top: 0, right: width, bottom: height }) {
                tracing::warn!("sizing WhatsApp Web: {err}");
            }
            let _ = controller.NotifyParentWindowPositionChanged();
        }
        if let Some(behind) = colour(&place.behind) {
            background(&controller, behind);
        }
    }

    /// Lays the view again where the page last kept room, as when the window
    /// moves to a screen of another scale.
    fn relay(&self) {
        if let Ok(place) = self.place.try_borrow()
            && let Some(place) = place.as_ref()
        {
            self.lay(place);
        }
    }

    /// Shows the view where the page keeps room, once WhatsApp Web is there.
    fn show_if_ready(self: &Rc<Self>) {
        let ready = self.place.borrow().is_some() && self.status.borrow().state == State::Open;
        if ready == self.shown.get() {
            return;
        }
        if !ready {
            return self.conceal();
        }
        let Some((frame, controller)) = self.parts() else { return };
        self.shown.set(true);
        unsafe {
            let _ = controller.SetIsVisible(true);
            let _ = ShowWindow(frame, SW_SHOWNA);
        }
        // Typing goes to WhatsApp Web as its page opens, or once the window is in front.
        if self.in_front() {
            self.focus();
        } else {
            self.refocus.set(true);
        }
    }

    /// Hides the view. Typing goes back to Orither, not to a view that is gone.
    fn conceal(&self) {
        self.shown.set(false);
        self.refocus.set(false);
        let Some((frame, controller)) = self.parts() else { return };
        unsafe {
            if self.focused.get() && self.in_front() {
                // The window hands it on to Orither's own view.
                let _ = SetFocus(Some(self.window));
            }
            let _ = ShowWindow(frame, SW_HIDE);
            let _ = controller.SetIsVisible(false);
        }
    }

    fn in_front(&self) -> bool {
        unsafe { GetForegroundWindow() == self.window }
    }

    fn focus(&self) {
        if let Some((_, controller)) = self.parts() {
            let _ = unsafe { controller.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC) };
        }
    }

    /// WhatsApp Web's light or dark, as Orither's colours have it. It is
    /// WebView2's own setting, as a browser has one, and WhatsApp Web follows it
    /// unless its own settings say otherwise.
    fn follow_theme(&self) {
        let Some(core) = self.core() else { return };
        let scheme = match colour(&theme::current().window).map(dark) {
            Some(true) => COREWEBVIEW2_PREFERRED_COLOR_SCHEME_DARK,
            Some(false) => COREWEBVIEW2_PREFERRED_COLOR_SCHEME_LIGHT,
            None => COREWEBVIEW2_PREFERRED_COLOR_SCHEME_AUTO,
        };
        let set = core.cast::<ICoreWebView2_13>().and_then(|core| unsafe { core.Profile()?.SetPreferredColorScheme(scheme) });
        if let Err(err) = set {
            tracing::warn!("WhatsApp Web's colour scheme: {err}");
        }
    }

    /// A page load starting, or a click on a link to another site, which opens in
    /// the computer's browser instead.
    fn navigating(self: &Rc<Self>, _: Option<ICoreWebView2>, args: Option<ICoreWebView2NavigationStartingEventArgs>) -> Handled {
        let Some(args) = args else { return Ok(()) };
        let uri = text(|value| unsafe { args.Uri(value) });
        let (mut user, mut redirected, mut id) = (BOOL::default(), BOOL::default(), 0);
        unsafe {
            args.IsUserInitiated(&mut user)?;
            args.IsRedirected(&mut redirected)?;
            args.NavigationId(&mut id)?;
        }
        if let Some(url) = leads_out(&uri, user.as_bool(), redirected.as_bool()) {
            unsafe { args.SetCancel(true)? };
            open_outside(&self.phone, &url);
            return Ok(());
        }
        self.loading.set(Some(id));
        self.update(|status| {
            status.state = State::Opening;
            status.error = None;
        });
        Ok(())
    }

    fn navigated(self: &Rc<Self>, _: Option<ICoreWebView2>, args: Option<ICoreWebView2NavigationCompletedEventArgs>) -> Handled {
        let Some(args) = args else { return Ok(()) };
        let (mut id, mut success, mut error) = (0, BOOL::default(), COREWEBVIEW2_WEB_ERROR_STATUS::default());
        unsafe {
            args.NavigationId(&mut id)?;
            args.IsSuccess(&mut success)?;
            args.WebErrorStatus(&mut error)?;
        }
        // A load that gave way to another says nothing of WhatsApp Web.
        if self.loading.get() != Some(id) {
            return Ok(());
        }
        self.loading.set(None);
        let mut code = 0;
        let answered = args
            .cast::<ICoreWebView2NavigationCompletedEventArgs2>()
            .and_then(|args| unsafe { args.HttpStatusCode(&mut code) })
            .is_ok_and(|()| code > 0);
        match failure(success.as_bool(), error, answered) {
            Some(error) => self.failed(error),
            None => self.update(|status| {
                if status.state == State::Opening {
                    status.state = State::Open;
                }
            }),
        }
        Ok(())
    }

    fn titled(self: &Rc<Self>, core: Option<ICoreWebView2>, _: Option<IUnknown>) -> Handled {
        let Some(core) = core else { return Ok(()) };
        let unread = unread(&text(|value| unsafe { core.DocumentTitle(value) }));
        self.update(|status| status.unread = unread);
        Ok(())
    }

    /// Anything meant for another window opens in the computer's browser: there
    /// are no other windows.
    fn new_window(self: &Rc<Self>, _: Option<ICoreWebView2>, args: Option<ICoreWebView2NewWindowRequestedEventArgs>) -> Handled {
        let Some(args) = args else { return Ok(()) };
        unsafe { args.SetHandled(true)? };
        if let Ok(url) = Url::parse(&text(|value| unsafe { args.Uri(value) })) {
            open_outside(&self.phone, &url);
        }
        Ok(())
    }

    /// Only WhatsApp gets what it asks a browser for, and nobody gets more, so
    /// WebView2 never asks the user.
    fn asked(self: &Rc<Self>, _: Option<ICoreWebView2>, args: Option<ICoreWebView2PermissionRequestedEventArgs>) -> Handled {
        let Some(args) = args else { return Ok(()) };
        let mut kind = COREWEBVIEW2_PERMISSION_KIND::default();
        unsafe { args.PermissionKind(&mut kind)? };
        let allowed = allowed(&text(|value| unsafe { args.Uri(value) }), kind);
        let state = if allowed { COREWEBVIEW2_PERMISSION_STATE_ALLOW } else { COREWEBVIEW2_PERMISSION_STATE_DENY };
        unsafe { args.SetState(state) }
    }

    fn crashed(self: &Rc<Self>, _: Option<ICoreWebView2>, args: Option<ICoreWebView2ProcessFailedEventArgs>) -> Handled {
        let Some(args) = args else { return Ok(()) };
        let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
        unsafe { args.ProcessFailedKind(&mut kind)? };
        let mut reason = COREWEBVIEW2_PROCESS_FAILED_REASON::default();
        let memory = args
            .cast::<ICoreWebView2ProcessFailedEventArgs2>()
            .and_then(|args| unsafe { args.Reason(&mut reason) })
            .is_ok_and(|()| reason == COREWEBVIEW2_PROCESS_FAILED_REASON_OUT_OF_MEMORY);
        let error = if memory { "WhatsApp Web ran out of memory" } else { "WhatsApp Web stopped" };
        match kind {
            // The browser behind the view is gone, and the view with it.
            COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED => self.lose(error),
            // The page is gone, and loading it again brings it back.
            COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED => self.failed(error.to_owned()),
            // A frame's, the GPU's or a helper's, which WebView2 starts again by itself.
            _ => {}
        }
        Ok(())
    }

    fn focus_in(self: &Rc<Self>, _: Option<ICoreWebView2Controller>, _: Option<IUnknown>) -> Handled {
        self.focused.set(true);
        Ok(())
    }

    fn focus_out(self: &Rc<Self>, _: Option<ICoreWebView2Controller>, _: Option<IUnknown>) -> Handled {
        self.focused.set(false);
        Ok(())
    }

    /// Saves what WhatsApp Web downloads in Downloads, under a name not taken
    /// yet, without WebView2's download bubble.
    fn downloading(self: &Rc<Self>, _: Option<ICoreWebView2>, args: Option<ICoreWebView2DownloadStartingEventArgs>) -> Handled {
        let Some(args) = args else { return Ok(()) };
        if let Ok(downloads) = self.phone.app.path().download_dir() {
            let suggested = text(|value| unsafe { args.ResultFilePath(value) });
            let path = free_name(&downloads, &suggested);
            unsafe { args.SetResultFilePath(&HSTRING::from(path.as_path()))? };
        }
        let phone = self.phone.clone();
        let changed = StateChangedEventHandler::create(Box::new(move |download, _| match download {
            Some(download) => saved(&phone, &download),
            None => Ok(()),
        }));
        unsafe {
            args.SetHandled(true)?;
            args.DownloadOperation()?.add_StateChanged(&changed, &mut 0)
        }
    }

    /// While the phone's notifications show on the computer, WhatsApp Web's own
    /// are held back, since the phone's say the same. Otherwise WebView2 shows
    /// them, as Edge would.
    fn notified(self: &Rc<Self>, _: Option<ICoreWebView2>, args: Option<ICoreWebView2NotificationReceivedEventArgs>) -> Handled {
        let Some(args) = args else { return Ok(()) };
        if !notifications::mirrored(&self.phone) {
            return Ok(());
        }
        let notification = unsafe {
            args.SetHandled(true)?;
            let notification = args.Notification()?;
            // As far as WhatsApp knows, it showed, as the phone's did.
            notification.ReportShown()?;
            notification
        };
        self.keep_quiet(notification)
    }

    fn keep_quiet(self: &Rc<Self>, notification: ICoreWebView2Notification) -> Handled {
        let serial = self.serial.get() + 1;
        self.serial.set(serial);
        let title = text(|value| unsafe { notification.Title(value) });
        let tag = text(|value| unsafe { notification.Tag(value) });
        let web = Rc::downgrade(self);
        let closed = NotificationCloseRequestedEventHandler::create(Box::new(move |_, _| {
            if let Some(web) = web.upgrade() {
                web.quiet.borrow_mut().retain(|kept| kept.serial != serial);
            }
            Ok(())
        }));
        unsafe { notification.add_CloseRequested(&closed, &mut 0)? };
        let mut quiet = self.quiet.borrow_mut();
        if quiet.len() == QUIET_KEPT {
            quiet.pop_front();
        }
        quiet.push_back(Quiet { serial, notification, title, tag });
        Ok(())
    }

    /// The latest of WhatsApp Web's notifications from this chat.
    fn take_quiet(&self, chat: &Chat) -> Option<ICoreWebView2Notification> {
        let mut quiet = self.quiet.borrow_mut();
        let found = quiet.iter().rposition(|kept| is_about(chat, said(&kept.tag), said(&kept.title)))?;
        quiet.remove(found).map(|kept| kept.notification)
    }

    fn menu(self: &Rc<Self>, _: Option<ICoreWebView2>, args: Option<ICoreWebView2ContextMenuRequestedEventArgs>) -> Handled {
        let Some(args) = args else { return Ok(()) };
        let items = unsafe { args.MenuItems()? };
        let mut count = 0;
        unsafe { items.Count(&mut count)? };
        for index in (0..count).rev() {
            let item = unsafe { items.GetValueAtIndex(index)? };
            if LEFT_OUT.contains(&text(|value| unsafe { item.Name(value) }).as_str()) {
                unsafe { items.RemoveValueAtIndex(index)? };
            }
        }
        Ok(())
    }
}

/// What WebView2 calls on an event, which reaches WhatsApp Web while it is there.
fn handler<S: 'static, A: 'static>(
    web: &Rc<Web>,
    handle: fn(&Rc<Web>, Option<S>, Option<A>) -> Handled,
) -> Box<dyn FnMut(Option<S>, Option<A>) -> Handled> {
    let web = Rc::downgrade(web);
    Box::new(move |sender, args| web.upgrade().map_or(Ok(()), |web| handle(&web, sender, args)))
}

/// A string WebView2 hands over, empty when it has none.
fn text(read: impl FnOnce(*mut PWSTR) -> windows::core::Result<()>) -> String {
    let mut value = PWSTR::null();
    match read(&mut value) {
        Ok(()) => take_pwstr(value),
        Err(_) => String::new(),
    }
}

fn said(text: &str) -> Option<&str> {
    (!text.is_empty()).then_some(text)
}

/// The window that holds the view, over the main window's content, hidden
/// until the view shows.
fn frame(window: HWND) -> Result<HWND> {
    unsafe {
        let instance = GetModuleHandleW(PCWSTR::null())?.into();
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(frame_messages),
            hInstance: instance,
            lpszClassName: FRAME_CLASS,
            ..Default::default()
        };
        // Once per run, after which it only fails for being done already.
        RegisterClassExW(&class);
        let style = WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS;
        let frame =
            CreateWindowExW(WINDOW_EX_STYLE::default(), FRAME_CLASS, PCWSTR::null(), style, 0, 0, 0, 0, Some(window), None, Some(instance), None)?;
        Ok(frame)
    }
}

unsafe extern "system" fn frame_messages(frame: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(frame, message, wparam, lparam) }
}

fn destroy(frame: HWND) {
    if let Err(err) = unsafe { DestroyWindow(frame) } {
        tracing::warn!("closing WhatsApp Web's window: {err}");
    }
}

/// Follows the main window: when it moves, for WebView2's popups, when it moves
/// to a screen of another scale, and when it closes. Once the window is back in
/// front from the keyboard, typing goes back to WhatsApp Web if it went there
/// before, as Orither's own view takes it whenever the window comes back. A
/// click decides for itself.
unsafe extern "system" fn follow(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM, _: usize, _: usize) -> LRESULT {
    let web = current();
    let activation = (wparam.0 & 0xffff) as u32;
    if let Some(web) = &web
        && message == WM_ACTIVATE
        && activation == WA_INACTIVE
    {
        web.refocus.set(web.shown.get() && web.focused.get());
    }
    // As Orither quits, WhatsApp Web's browser closes properly, and keeps its login.
    if message == WM_DESTROY
        && let Some(web) = &web
        && let Some(view) = web.view.try_borrow_mut().ok().and_then(|mut view| view.take())
    {
        view.close();
    }
    if message == WM_NCDESTROY {
        let _ = unsafe { RemoveWindowSubclass(window, Some(follow), FOLLOW) };
    }
    let answer = unsafe { DefSubclassProc(window, message, wparam, lparam) };
    if let Some(web) = web {
        match message {
            WM_ACTIVATE if activation == WA_ACTIVE && web.refocus.take() && web.shown.get() => web.focus(),
            WM_DPICHANGED => web.relay(),
            WM_MOVE => {
                if let Some((_, controller)) = web.parts() {
                    let _ = unsafe { controller.NotifyParentWindowPositionChanged() };
                }
            }
            _ => {}
        }
    }
    answer
}

/// The colour the view shows before WhatsApp Web paints: the one around it.
fn background(controller: &ICoreWebView2Controller, [red, green, blue]: [u8; 3]) {
    let colour = COREWEBVIEW2_COLOR { A: 255, R: red, G: green, B: blue };
    let set = controller.cast::<ICoreWebView2Controller2>().and_then(|controller| unsafe { controller.SetDefaultBackgroundColor(colour) });
    if let Err(err) = set {
        tracing::warn!("WhatsApp Web's background: {err}");
    }
}

/// WhatsApp may notify from the start, as a browser remembers a site the user
/// let notify.
fn let_notify(core: &ICoreWebView2) -> Handled {
    let profile = unsafe { core.cast::<ICoreWebView2_13>()?.Profile()? }.cast::<ICoreWebView2Profile4>()?;
    let done = SetPermissionStateCompletedHandler::create(Box::new(|result| {
        if let Err(err) = result {
            tracing::warn!("letting WhatsApp Web notify: {err}");
        }
        Ok(())
    }));
    let (kind, state) = (COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS, COREWEBVIEW2_PERMISSION_STATE_ALLOW);
    unsafe { profile.SetPermissionState(kind, &HSTRING::from(ORIGIN), state, &done) }
}

/// Tells the page of a download once it is saved, or why it was not.
fn saved(phone: &Phone, download: &ICoreWebView2DownloadOperation) -> Handled {
    let mut state = COREWEBVIEW2_DOWNLOAD_STATE::default();
    unsafe { download.State(&mut state)? };
    if state == COREWEBVIEW2_DOWNLOAD_STATE_COMPLETED {
        let path = PathBuf::from(text(|value| unsafe { download.ResultFilePath(value) }));
        let name = path.file_name().map(|name| name.to_string_lossy().into_owned());
        phone.tell_ui("whatsapp", "saved", json!({ "name": name, "path": path }));
    } else if state == COREWEBVIEW2_DOWNLOAD_STATE_INTERRUPTED {
        let mut reason = COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON::default();
        unsafe { download.InterruptReason(&mut reason)? };
        if reason != COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_USER_CANCELED {
            phone.tell_ui("whatsapp", "failed", json!({ "error": format!("Saving a download: {}", interrupted(reason)) }));
        }
    }
    Ok(())
}

fn interrupted(reason: COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON) -> String {
    match reason {
        COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_FILE_NO_SPACE | COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_FILE_TOO_LARGE => {
            "the disk is full".into()
        }
        COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_FILE_ACCESS_DENIED => "Downloads cannot be written to".into(),
        COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_FILE_BLOCKED_BY_POLICY
        | COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_FILE_MALICIOUS
        | COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_FILE_SECURITY_CHECK_FAILED => "Windows blocked the file".into(),
        COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_NETWORK_DISCONNECTED
        | COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_NETWORK_FAILED
        | COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_NETWORK_TIMEOUT => "the connection broke off".into(),
        _ => format!("WebView2 stopped it ({})", reason.0),
    }
}

/// Where a link that leaves WhatsApp Web goes: a click on one to another site.
/// What WhatsApp Web loads by itself, and its server's redirects, stay.
fn leads_out(uri: &str, user: bool, redirected: bool) -> Option<Url> {
    let url = Url::parse(uri).ok()?;
    (user && !redirected && url.host_str() != Some(HOST)).then_some(url)
}

fn open_outside(phone: &Phone, url: &Url) {
    if OUTSIDE.contains(&url.scheme())
        && let Err(err) = phone.app.opener().open_url(url.as_str(), None::<&str>)
    {
        tracing::warn!("opening a link from WhatsApp Web: {err}");
    }
}

fn allowed(uri: &str, kind: COREWEBVIEW2_PERMISSION_KIND) -> bool {
    let whatsapp = Url::parse(uri).ok().is_some_and(|url| url.host_str() == Some(HOST));
    whatsapp && ALLOWED.contains(&kind)
}

/// Why a page load failed, if it did. One cancelled, as for another, did not,
/// and nor did one the server answered with a page of its own, which shows as
/// it would in a browser.
fn failure(success: bool, error: COREWEBVIEW2_WEB_ERROR_STATUS, answered: bool) -> Option<String> {
    let cancelled = error == COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED;
    let answered = answered && error == COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN;
    (!(success || cancelled || answered)).then(|| why(error))
}

fn why(error: COREWEBVIEW2_WEB_ERROR_STATUS) -> String {
    match error {
        COREWEBVIEW2_WEB_ERROR_STATUS_HOST_NAME_NOT_RESOLVED => "Could not find web.whatsapp.com. Is this computer online?".into(),
        COREWEBVIEW2_WEB_ERROR_STATUS_DISCONNECTED => "This computer is offline".into(),
        COREWEBVIEW2_WEB_ERROR_STATUS_CANNOT_CONNECT | COREWEBVIEW2_WEB_ERROR_STATUS_SERVER_UNREACHABLE => {
            "Could not reach WhatsApp".into()
        }
        COREWEBVIEW2_WEB_ERROR_STATUS_TIMEOUT => "WhatsApp took too long to answer".into(),
        COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_ABORTED | COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_RESET => {
            "The connection to WhatsApp broke off".into()
        }
        COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_COMMON_NAME_IS_INCORRECT
        | COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_EXPIRED
        | COREWEBVIEW2_WEB_ERROR_STATUS_CLIENT_CERTIFICATE_CONTAINS_ERRORS
        | COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_REVOKED
        | COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_IS_INVALID => {
            "WhatsApp's certificate is not one to trust, so the connection may not be private".into()
        }
        COREWEBVIEW2_WEB_ERROR_STATUS_VALID_PROXY_AUTHENTICATION_REQUIRED => "The proxy asks for a login".into(),
        _ => format!("WhatsApp Web did not load (WebView2 error {})", error.0),
    }
}

/// A CSS colour as the page and the theme write it: `#rrggbb`, `rgb(…)` or `rgba(…)`.
fn colour(css: &str) -> Option<[u8; 3]> {
    let css = css.trim();
    if let Some(hex) = css.strip_prefix('#') {
        let part = |at: usize| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok();
        return if hex.len() == 6 { Some([part(0)?, part(2)?, part(4)?]) } else { None };
    }
    let inner = css.strip_prefix("rgba(").or_else(|| css.strip_prefix("rgb("))?.strip_suffix(')')?;
    let mut parts = inner.split([',', ' ', '/']).filter(|part| !part.is_empty()).map(|part| part.parse::<f64>().ok());
    let mut channel = || parts.next().flatten().map(|value| value.round().clamp(0.0, 255.0) as u8);
    Some([channel()?, channel()?, channel()?])
}

fn dark([red, green, blue]: [u8; 3]) -> bool {
    0.299 * f64::from(red) + 0.587 * f64::from(green) + 0.114 * f64::from(blue) < 128.0
}

/// Opens this chat in WhatsApp Web: through its own notification, as a click on
/// it would, or else by its number, which loads WhatsApp Web again.
pub fn open(phone: &Phone, chat: Chat) {
    let opened = on_main(phone, move |phone| {
        let web = start(phone).inspect_err(|err| tracing::warn!("opening a chat in WhatsApp Web: {err:#}"))?;
        if let Some(notification) = web.take_quiet(&chat) {
            if let Err(err) = unsafe { notification.ReportClicked() } {
                tracing::warn!("opening a chat in WhatsApp Web: {err}");
            }
        } else if let Some(number) = chat.id.as_deref().and_then(phone_number) {
            web.load(&format!("{HOME}send?phone={number}"));
        }
        Ok(())
    });
    if let Err(err) = opened {
        tracing::warn!("opening a chat in WhatsApp Web: {err:#}");
    }
}

pub async fn command(phone: &Phone, method: &str, body: &Value) -> Result<Value> {
    let done = match method {
        "web" => on_main(phone, |_| Ok(json!(current().map_or(Status::OFF, |web| web.status.borrow().clone())))),
        "place" => {
            let place: Option<Place> = serde_json::from_value(body["at"].clone())?;
            on_main(phone, move |phone| place_at(phone, place).map(|()| Value::Null))
        }
        "reload" => on_main(phone, |phone| {
            start(phone)?.load(HOME);
            Ok(Value::Null)
        }),
        "reveal" => return reveal(phone, body),
        _ => bail!("WhatsApp Web has no {method}"),
    };
    done?.await.context("the window went away")?
}

/// Lays the view where the page keeps room, or hides it for none. Room is what
/// starts WhatsApp Web, the first time the page shows.
fn place_at(phone: &Phone, place: Option<Place>) -> Result<()> {
    let web = match &place {
        Some(_) => start(phone)?,
        None => match current() {
            Some(web) => web,
            None => return Ok(()),
        },
    };
    web.place.replace(place);
    web.relay();
    web.show_if_ready();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_css_colours() {
        assert_eq!(colour("#1c1c1c"), Some([0x1c, 0x1c, 0x1c]));
        assert_eq!(colour("rgb(23, 23, 23)"), Some([23, 23, 23]));
        assert_eq!(colour("rgba(251, 251, 251, 0.5)"), Some([251, 251, 251]));
        assert_eq!(colour("rgb(1 2 3 / 50%)"), Some([1, 2, 3]));
        assert_eq!(colour("#fff"), None);
        assert_eq!(colour("transparent"), None);
        assert!(dark([0x17, 0x17, 0x17]));
        assert!(!dark([0xf3, 0xf3, 0xf3]));
    }

    #[test]
    fn only_clicks_to_other_sites_leave() {
        assert!(leads_out("https://faq.whatsapp.com/1", true, false).is_some());
        assert!(leads_out("https://web.whatsapp.com/send?phone=1", true, false).is_none());
        assert!(leads_out("https://www.whatsapp.com/", false, false).is_none());
        assert!(leads_out("https://www.whatsapp.com/", true, true).is_none());
    }

    #[test]
    fn only_whatsapp_gets_permissions() {
        assert!(allowed("https://web.whatsapp.com", COREWEBVIEW2_PERMISSION_KIND_MICROPHONE));
        assert!(!allowed("https://web.whatsapp.com", COREWEBVIEW2_PERMISSION_KIND_GEOLOCATION));
        assert!(!allowed("https://example.com", COREWEBVIEW2_PERMISSION_KIND_CAMERA));
    }

    #[test]
    fn a_cancelled_or_answered_load_did_not_fail() {
        assert_eq!(failure(true, COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN, true), None);
        assert_eq!(failure(false, COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED, false), None);
        assert_eq!(failure(false, COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN, true), None);
        assert!(failure(false, COREWEBVIEW2_WEB_ERROR_STATUS_DISCONNECTED, false).is_some());
    }
}
