//! WhatsApp Web, WhatsApp's own site, in a view laid over the WhatsApp page. It
//! loads web.whatsapp.com as a browser would, with no script added to the page,
//! only a style sheet in Orither's colours (see [`look`](super::look)), and
//! keeps its login in a folder of its own.
//!
//! Tauri cannot place a second view on Linux, so the window's content moves
//! into an overlay at start, and the view sits over it where the page keeps
//! room. GTK clips nothing round, so four small corners in the colour behind
//! round it off. Everything here lives on the main thread, as GTK wants.

use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    f64::consts::{FRAC_PI_2, PI},
    iter,
    path::PathBuf,
    rc::Rc,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use gtk::{gdk, glib, prelude::*};
use serde::Deserialize;
use serde_json::{Value, json};
use tauri::{AppHandle, Listener, Manager, Url};
use tauri_plugin_opener::OpenerExt;
use webkit2gtk::{
    ContextMenuAction, ContextMenuExt, ContextMenuItemExt, CookieAcceptPolicy, CookieManagerExt,
    CookiePersistentStorage, DeviceInfoPermissionRequest, Download, DownloadError, DownloadExt, LoadEvent,
    NavigationPolicyDecision, NavigationPolicyDecisionExt, NavigationType, NetworkError, Notification,
    NotificationExt, NotificationPermissionRequest, PermissionRequest, PermissionRequestExt, PolicyDecision,
    PolicyDecisionExt, PolicyDecisionType, PolicyError, SecurityOrigin, Settings, SettingsExt, URIRequestExt,
    UserContentInjectedFrames, UserContentManager, UserContentManagerExt, UserMediaPermissionRequest,
    UserMediaPermissionRequestExt, UserStyleLevel, UserStyleSheet, WebContext, WebContextExt,
    WebProcessTerminationReason, WebView, WebViewExt, WebsiteDataManager, WebsiteDataManagerExt,
};

use super::{
    Chat,
    common::{
        FOLDER, HOME, HOST, OUTSIDE, QUIET_KEPT, START_AFTER, State, Status, free_name, is_about, on_main, phone_number,
        reveal, unread,
    },
    look,
};
use crate::{
    features::{Phone, notifications},
    theme,
};

/// Where Orither's style sheet applies.
const STYLED: &str = "https://web.whatsapp.com/*";
const FADE_IN: Duration = Duration::from_millis(160);
/// What the context menu leaves out: there are no other windows, and no history.
const LEFT_OUT: [ContextMenuAction; 9] = [
    ContextMenuAction::OpenLink,
    ContextMenuAction::OpenLinkInNewWindow,
    ContextMenuAction::OpenImageInNewWindow,
    ContextMenuAction::OpenFrameInNewWindow,
    ContextMenuAction::OpenVideoInNewWindow,
    ContextMenuAction::OpenAudioInNewWindow,
    ContextMenuAction::GoBack,
    ContextMenuAction::GoForward,
    ContextMenuAction::Stop,
];

/// Where the page keeps room for the view, in the window's pixels.
#[derive(Deserialize)]
struct Place {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    radius: f64,
    /// The colour around it, in CSS.
    behind: String,
}

/// How the corners round the view off.
#[derive(Clone)]
struct Rounding {
    radius: f64,
    behind: gdk::RGBA,
}

/// The window's content, with room over it.
struct Frame {
    overlay: gtk::Overlay,
    /// Orither's own view, the UI.
    main: WebView,
}

struct Web {
    phone: Phone,
    frame: Rc<Frame>,
    view: WebView,
    /// Top left, top right, bottom left, bottom right.
    corners: [gtk::DrawingArea; 4],
    rounding: Rc<RefCell<Rounding>>,
    status: RefCell<Status>,
    /// Where the page keeps room for the view, while it does.
    place: RefCell<Option<Place>>,
    /// WhatsApp Web's notifications that the phone's own stood in for. A click
    /// on the phone's opens the chat through one of these, as WhatsApp would.
    quiet: RefCell<VecDeque<Notification>>,
    fade: RefCell<Option<gtk::TickCallbackId>>,
}

thread_local! {
    static FRAME: RefCell<Option<Rc<Frame>>> = const { RefCell::new(None) };
    static WEB: RefCell<Option<Rc<Web>>> = const { RefCell::new(None) };
}

fn current() -> Option<Rc<Web>> {
    WEB.with_borrow(Clone::clone)
}

/// Makes room over the window's content, and starts WhatsApp Web if it was
/// used before.
pub fn install(phone: &Phone) {
    let Some(window) = phone.app.get_webview_window("main") else {
        return;
    };
    let phone = phone.clone();
    let wrapped = window.with_webview(move |webview| {
        if let Err(err) = wrap(webview.inner()) {
            return tracing::warn!("no room for WhatsApp Web: {err:#}");
        }
        if folders(&phone.app).is_ok_and(|(data, _)| data.exists()) {
            glib::timeout_add_local_once(START_AFTER, move || {
                if let Err(err) = start(&phone) {
                    tracing::warn!("starting WhatsApp Web: {err:#}");
                }
            });
        }
    });
    if let Err(err) = wrapped {
        tracing::warn!("no room for WhatsApp Web: {err:#}");
    }
}

/// Puts the window's view in an overlay, where it was.
fn wrap(main: WebView) -> Result<()> {
    let parent = main.parent().and_then(|parent| parent.downcast::<gtk::Box>().ok());
    let parent = parent.context("the window's view is not in a box")?;
    let position = parent.children().iter().position(|child| child == main.upcast_ref::<gtk::Widget>());
    let overlay = gtk::Overlay::new();
    parent.remove(&main);
    overlay.add(&main);
    parent.pack_start(&overlay, true, true, 0);
    if let Some(position) = position.and_then(|position| i32::try_from(position).ok()) {
        parent.reorder_child(&overlay, position);
    }
    overlay.show();
    FRAME.set(Some(Rc::new(Frame { overlay, main })));
    Ok(())
}

fn folders(app: &AppHandle) -> Result<(PathBuf, PathBuf)> {
    let path = app.path();
    Ok((path.app_data_dir()?.join(FOLDER), path.app_cache_dir()?.join(FOLDER)))
}

/// The view, made and loaded the first time it is needed.
fn start(phone: &Phone) -> Result<Rc<Web>> {
    if let Some(web) = current() {
        return Ok(web);
    }
    let frame = FRAME.with_borrow(Clone::clone).context("WhatsApp Web has no room in this window")?;
    let settings = Settings::new();
    settings.set_enable_media_stream(true);
    settings.set_enable_write_console_messages_to_stdout(false);
    let content = UserContentManager::new();
    let builder = WebView::builder().web_context(&context(phone)?).settings(&settings);
    let view = builder.user_content_manager(&content).build();
    dress(&view, &look::stylesheet(&theme::current()));
    let corners: [gtk::DrawingArea; 4] = std::array::from_fn(|_| gtk::DrawingArea::new());
    let widgets = iter::once(view.upcast_ref::<gtk::Widget>()).chain(corners.iter().map(|corner| corner.upcast_ref()));
    for widget in widgets {
        // The window shows all it holds when it opens, and these wait their turn.
        widget.set_no_show_all(true);
        frame.overlay.add_overlay(widget);
    }
    let rounding = Rc::new(RefCell::new(Rounding { radius: 0.0, behind: gdk::RGBA::BLACK }));
    for (index, corner) in corners.iter().enumerate() {
        frame.overlay.set_overlay_pass_through(corner, true);
        let rounding = rounding.clone();
        corner.connect_draw(move |corner, cairo| {
            round(corner, cairo, index, &rounding.borrow());
            glib::Propagation::Stop
        });
    }
    let web = Rc::new(Web {
        phone: phone.clone(),
        frame,
        view,
        corners,
        rounding,
        status: RefCell::new(Status::OFF),
        place: RefCell::default(),
        quiet: RefCell::default(),
        fade: RefCell::default(),
    });
    web.listen();
    WEB.set(Some(web.clone()));
    web.view.load_uri(HOME);
    Ok(web)
}

/// WhatsApp Web's own browser data: its login, cookies and cache.
fn context(phone: &Phone) -> Result<WebContext> {
    let (data, cache) = folders(&phone.app)?;
    std::fs::create_dir_all(&data)?;
    let manager = WebsiteDataManager::builder()
        .base_data_directory(data.to_string_lossy().into_owned())
        .base_cache_directory(cache.to_string_lossy().into_owned())
        .build();
    let cookies = manager.cookie_manager().context("WhatsApp Web has no cookies")?;
    cookies.set_persistent_storage(&data.join("cookies.sqlite").to_string_lossy(), CookiePersistentStorage::Sqlite);
    cookies.set_accept_policy(CookieAcceptPolicy::NoThirdParty);
    let context = WebContext::with_website_data_manager(&manager);
    let languages = spelling_languages();
    context.set_spell_checking_enabled(true);
    context.set_spell_checking_languages(&languages.iter().map(String::as_str).collect::<Vec<_>>());
    // As a browser remembers a site the user let notify.
    context.connect_initialize_notification_permissions(|context| {
        context.initialize_notification_permissions(&[&SecurityOrigin::for_uri(HOME)], &[]);
    });
    let phone = phone.clone();
    context.connect_download_started(move |_, download| save(&phone, download));
    Ok(context)
}

/// The user's languages, as the spell checker names them.
fn spelling_languages() -> Vec<String> {
    glib::language_names()
        .iter()
        .filter(|name| !["C", "POSIX"].contains(&name.as_str()) && !name.contains(['.', '@']))
        .map(ToString::to_string)
        .collect()
}

/// Lays Orither's style sheet over WhatsApp Web's, in place of the one before.
fn dress(view: &WebView, sheet: &str) {
    let Some(content) = view.user_content_manager() else { return };
    content.remove_all_style_sheets();
    let frames = UserContentInjectedFrames::AllFrames;
    content.add_style_sheet(&UserStyleSheet::new(sheet, frames, UserStyleLevel::User, &[STYLED], &[]));
}

/// Paints one corner in the colour behind, so the view looks round.
fn round(corner: &gtk::DrawingArea, cairo: &gtk::cairo::Context, index: usize, rounding: &Rounding) {
    let (width, height) = (f64::from(corner.allocated_width()), f64::from(corner.allocated_height()));
    // Every corner is the top left one, turned.
    if index % 2 == 1 {
        cairo.translate(width, 0.0);
        cairo.scale(-1.0, 1.0);
    }
    if index >= 2 {
        cairo.translate(0.0, height);
        cairo.scale(1.0, -1.0);
    }
    let (radius, behind) = (rounding.radius, &rounding.behind);
    cairo.set_source_rgba(behind.red(), behind.green(), behind.blue(), behind.alpha());
    cairo.move_to(0.0, 0.0);
    cairo.line_to(radius, 0.0);
    cairo.arc_negative(radius, radius, radius, -FRAC_PI_2, -PI);
    cairo.close_path();
    let _ = cairo.fill();
}

impl Web {
    fn listen(self: &Rc<Self>) {
        let web = Rc::downgrade(self);
        // The margins that hold the view in place depend on the window's size.
        self.frame.overlay.connect_size_allocate(move |_, _| {
            let web = web.clone();
            glib::idle_add_local_once(move || {
                if let Some(web) = web.upgrade()
                    && let Some(place) = web.place.borrow().as_ref()
                {
                    web.lay(place);
                }
            });
        });

        // Orither's colours follow the desktop's, and WhatsApp Web's follow them.
        let app = self.phone.app.clone();
        self.phone.app.listen_any(theme::CHANGED, move |_| {
            let sheet = look::stylesheet(&theme::current());
            let _ = app.run_on_main_thread(move || {
                if let Some(web) = current() {
                    dress(&web.view, &sheet);
                }
            });
        });

        let web = Rc::downgrade(self);
        self.view.connect_load_changed(move |_, event| {
            let Some(web) = web.upgrade() else { return };
            match event {
                LoadEvent::Started => web.update(|status| {
                    status.state = State::Opening;
                    status.error = None;
                }),
                // Not on a load that failed, which finishes too.
                LoadEvent::Finished => web.update(|status| {
                    if status.state == State::Opening {
                        status.state = State::Open;
                    }
                }),
                _ => {}
            }
        });

        let web = Rc::downgrade(self);
        self.view.connect_load_failed(move |_, _, _, error| {
            // A load that gives way to another did not fail.
            if error.matches(NetworkError::Cancelled) || error.matches(PolicyError::FrameLoadInterruptedByPolicyChange) {
                return false;
            }
            if let Some(web) = web.upgrade() {
                web.failed(error.to_string());
            }
            true
        });

        let web = Rc::downgrade(self);
        self.view.connect_web_process_terminated(move |_, reason| {
            let Some(web) = web.upgrade() else { return };
            web.failed(match reason {
                WebProcessTerminationReason::ExceededMemoryLimit => "WhatsApp Web ran out of memory".into(),
                _ => "WhatsApp Web stopped".into(),
            });
        });

        let web = Rc::downgrade(self);
        self.view.connect_title_notify(move |view| {
            let unread = view.title().as_deref().map_or(0, unread);
            if let Some(web) = web.upgrade() {
                web.update(|status| status.unread = unread);
            }
        });

        let phone = self.phone.clone();
        self.view.connect_decide_policy(move |_, decision, kind| {
            let Some(url) = leads_out(decision, kind) else {
                return false;
            };
            decision.ignore();
            if OUTSIDE.contains(&url.scheme())
                && let Err(err) = phone.app.opener().open_url(url.as_str(), None::<&str>)
            {
                tracing::warn!("opening a link from WhatsApp Web: {err}");
            }
            true
        });

        self.view.connect_permission_request(|view, request| {
            if allowed(view, request) {
                request.allow();
            } else {
                request.deny();
            }
            true
        });

        let web = Rc::downgrade(self);
        self.view.connect_show_notification(move |_, notification| {
            let Some(web) = web.upgrade() else { return false };
            // The phone's own notification already tells of it.
            if notifications::mirrored(&web.phone) {
                web.keep_quiet(notification);
                return true;
            }
            let phone = web.phone.clone();
            notification.connect_clicked(move |_| super::show_page(&phone));
            false
        });

        self.view.connect_context_menu(|_, menu, _, _| {
            for item in menu.items() {
                if LEFT_OUT.contains(&item.stock_action()) {
                    menu.remove(&item);
                }
            }
            false
        });
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

    fn lay(&self, place: &Place) {
        let behind = place.behind.parse().unwrap_or(gdk::RGBA::BLACK);
        self.view.set_background_color(&behind);
        let room = self.frame.overlay.allocation();
        bound(&self.view, &room, &gdk::Rectangle::new(place.x, place.y, place.width, place.height));
        let radius = place.radius.clamp(0.0, f64::from(place.width.min(place.height)) / 2.0);
        // Whole pixels, which the radius fills.
        let size = radius.ceil() as i32;
        let (right, bottom) = (place.x + place.width - size, place.y + place.height - size);
        let corners = [(place.x, place.y), (right, place.y), (place.x, bottom), (right, bottom)];
        for (corner, (x, y)) in self.corners.iter().zip(corners) {
            bound(corner, &room, &gdk::Rectangle::new(x, y, size, size));
            corner.queue_draw();
        }
        *self.rounding.borrow_mut() = Rounding { radius, behind };
    }

    /// Shows the view where the page keeps room, once WhatsApp Web is there.
    fn show_if_ready(self: &Rc<Self>) {
        let ready = self.place.borrow().is_some() && self.status.borrow().state == State::Open;
        if ready == self.view.is_visible() {
            return;
        }
        if ready {
            self.view.show();
            self.corners.iter().for_each(WidgetExt::show);
            self.fade_in();
            self.view.grab_focus();
        } else {
            // Typing goes back to Orither, not to a view that is gone.
            if self.view.is_focus() {
                self.frame.main.grab_focus();
            }
            self.view.hide();
            self.corners.iter().for_each(WidgetExt::hide);
        }
    }

    fn fade_in(self: &Rc<Self>) {
        if let Some(fade) = self.fade.take() {
            fade.remove();
        }
        self.view.set_opacity(0.0);
        let start = Cell::new(None);
        let web = Rc::downgrade(self);
        let fade = self.view.add_tick_callback(move |view, clock| {
            let now = clock.frame_time();
            let began = start.get().unwrap_or(now);
            start.set(Some(began));
            let done = ((now - began) as f64 / FADE_IN.as_micros() as f64).min(1.0);
            view.set_opacity(1.0 - (1.0 - done).powi(3));
            if done < 1.0 {
                return glib::ControlFlow::Continue;
            }
            if let Some(web) = web.upgrade() {
                web.fade.take();
            }
            glib::ControlFlow::Break
        });
        self.fade.replace(Some(fade));
    }

    fn keep_quiet(self: &Rc<Self>, notification: &Notification) {
        {
            let mut quiet = self.quiet.borrow_mut();
            if quiet.len() == QUIET_KEPT {
                quiet.pop_front();
            }
            quiet.push_back(notification.clone());
        }
        let web = Rc::downgrade(self);
        notification.connect_closed(move |closed| {
            if let Some(web) = web.upgrade() {
                web.quiet.borrow_mut().retain(|kept| kept != closed);
            }
        });
    }

    /// The latest of WhatsApp Web's notifications from this chat.
    fn take_quiet(&self, chat: &Chat) -> Option<Notification> {
        let mut quiet = self.quiet.borrow_mut();
        let found = quiet
            .iter()
            .rposition(|notification| is_about(chat, notification.tag().as_deref(), notification.title().as_deref()))?;
        quiet.remove(found)
    }
}

/// Lays a widget over `area` of the window's content. An overlay sizes what it
/// holds by what that would like, and a web view would like its page's size, so
/// the widget fills the overlay, and margins keep it to the area.
fn bound(widget: &impl IsA<gtk::Widget>, room: &gtk::Allocation, area: &gdk::Rectangle) {
    widget.set_margin_start(area.x());
    widget.set_margin_top(area.y());
    widget.set_margin_end((room.width() - area.x() - area.width()).max(0));
    widget.set_margin_bottom((room.height() - area.y() - area.height()).max(0));
}

/// Where a link that leaves WhatsApp Web goes: anything for another window,
/// and a click on a link to another site.
fn leads_out(decision: &PolicyDecision, kind: PolicyDecisionType) -> Option<Url> {
    let action = decision.downcast_ref::<NavigationPolicyDecision>()?.navigation_action()?;
    let url = Url::parse(&action.request()?.uri()?).ok()?;
    match kind {
        PolicyDecisionType::NewWindowAction => Some(url),
        PolicyDecisionType::NavigationAction
            if action.navigation_type() == NavigationType::LinkClicked && url.host_str() != Some(HOST) =>
        {
            Some(url)
        }
        _ => None,
    }
}

/// Only WhatsApp gets what it asks a browser for: to notify, and the
/// microphone and camera for voice messages and photos.
fn allowed(view: &WebView, request: &PermissionRequest) -> bool {
    let whatsapp = view.uri().and_then(|uri| Url::parse(&uri).ok()).is_some_and(|url| url.host_str() == Some(HOST));
    let device = request
        .downcast_ref::<UserMediaPermissionRequest>()
        .is_some_and(|media| media.is_for_audio_device() || media.is_for_video_device());
    whatsapp
        && (device || request.is::<NotificationPermissionRequest>() || request.is::<DeviceInfoPermissionRequest>())
}

/// Saves what WhatsApp Web downloads in Downloads, under a name not taken yet.
fn save(phone: &Phone, download: &Download) {
    let downloads = phone.app.path().download_dir().ok();
    download.connect_decide_destination(move |download, suggested| {
        let Some(downloads) = &downloads else { return false };
        match glib::filename_to_uri(free_name(downloads, suggested), None) {
            Ok(uri) => download.set_destination(&uri),
            Err(_) => return false,
        }
        true
    });
    let failed = Rc::new(Cell::new(false));
    let (phone, gone) = (phone.clone(), failed.clone());
    let told = phone.clone();
    download.connect_failed(move |_, error| {
        gone.set(true);
        if !error.matches(DownloadError::CancelledByUser) {
            told.tell_ui("whatsapp", "failed", json!({ "error": format!("Saving a download: {error}") }));
        }
    });
    download.connect_finished(move |download| {
        let saved = download.destination().and_then(|uri| glib::filename_from_uri(&uri).ok());
        let Some((path, _)) = saved.filter(|_| !failed.get()) else { return };
        let name = path.file_name().map(|name| name.to_string_lossy().into_owned());
        phone.tell_ui("whatsapp", "saved", json!({ "name": name, "path": path }));
    });
}

/// Opens this chat in WhatsApp Web: through its own notification, as a click
/// on it would, or else by its number, which loads WhatsApp Web again.
pub fn open(phone: &Phone, chat: Chat) {
    let opened = on_main(phone, move |phone| {
        let web = start(phone).inspect_err(|err| tracing::warn!("opening a chat in WhatsApp Web: {err:#}"))?;
        if let Some(notification) = web.take_quiet(&chat) {
            notification.clicked();
        } else if let Some(number) = chat.id.as_deref().and_then(phone_number) {
            web.view.load_uri(&format!("{HOME}send?phone={number}"));
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
            start(phone)?.view.load_uri(HOME);
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
        Some(place) => {
            let web = start(phone)?;
            web.lay(place);
            web
        }
        None => match current() {
            Some(web) => web,
            None => return Ok(()),
        },
    };
    web.place.replace(place);
    web.show_if_ready();
    Ok(())
}
