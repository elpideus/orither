//! The features that need code on the computer, not just in the UI. Adding one
//! means writing a [`Feature`] and listing it in [`Features::install`].

mod calls;
mod clipboard;
mod hardware;
mod helper;
mod input;
mod nearby;
mod notifications;
pub mod screen;
mod setup;
mod transfer;
mod updates;
pub use updates::restarted_by_click;
mod whatsapp;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use orither_link::{IncomingStream, Node, PeerId, Session};
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter};

/// A message for the UI, in the same shape as one from the phone.
#[derive(Clone, Serialize)]
pub struct Notice {
    pub feature: String,
    pub method: String,
    pub body: Value,
    /// The phone it is about, when it is about one.
    pub peer: Option<String>,
}

/// What a feature needs to reach the phones and the UI.
#[derive(Clone)]
pub struct Phone {
    pub app: AppHandle,
    pub node: Node,
    /// The phone the UI shows, which features without a phone of their own follow.
    selected: Arc<Mutex<Option<PeerId>>>,
}

impl Phone {
    pub fn new(app: AppHandle, node: Node, selected: Option<PeerId>) -> Self {
        Self { app, node, selected: Arc::new(Mutex::new(selected)) }
    }

    /// The phone in use: the one the UI shows while it is connected, or else any connected one.
    pub fn session(&self) -> Result<Session> {
        let selected = *self.selected.lock().unwrap();
        selected
            .and_then(|peer| self.node.session(&peer))
            .or_else(|| self.node.any_session())
            .context("your phone is not connected")
    }

    /// A particular phone, connected.
    pub fn session_of(&self, peer: &PeerId) -> Result<Session> {
        self.node.session(peer).context("that phone is not connected")
    }

    pub fn selected(&self) -> Option<PeerId> {
        *self.selected.lock().unwrap()
    }

    /// Returns whether the choice changed.
    pub fn select(&self, peer: Option<PeerId>) -> bool {
        let mut selected = self.selected.lock().unwrap();
        let changed = *selected != peer;
        *selected = peer;
        changed
    }

    pub fn tell_ui(&self, feature: &str, method: &str, body: Value) {
        let _ = self.app.emit("notice", Notice { feature: feature.into(), method: method.into(), body, peer: None });
    }

    /// Tells the UI something about one phone in particular.
    pub fn tell_ui_about(&self, peer: &PeerId, feature: &str, method: &str, body: Value) {
        let notice = Notice { feature: feature.into(), method: method.into(), body, peer: Some(peer.to_string()) };
        let _ = self.app.emit("notice", notice);
    }
}

/// Everything here is optional, so a feature implements only what it needs.
#[async_trait]
pub trait Feature: Send + Sync {
    fn id(&self) -> &'static str;

    /// Answers a call from the UI, or returns `None` to pass it on to the phone.
    async fn command(&self, _method: &str, _body: &Value) -> Option<Result<Value>> {
        None
    }

    fn connected(&self, _session: &Session) {}

    fn disconnected(&self, _peer: &PeerId) {}

    /// The UI now shows another phone. `session` is that phone, if it is connected.
    fn switched(&self, _session: Option<&Session>) {}

    /// Answers a call from a phone, or returns `None` when this feature takes none.
    async fn answer(&self, _peer: &PeerId, _method: &str, _body: &Value) -> Option<Result<Value>> {
        None
    }

    /// Orither is quitting: undo anything that would outlast it.
    fn stopping(&self) {}

    /// Takes a message from the phone. Returns false to pass it on to the UI.
    fn notice(&self, _peer: &PeerId, _method: &str, _body: &Value) -> bool {
        false
    }

    async fn stream(&self, stream: IncomingStream) -> Result<()> {
        bail!("{} takes no stream {}", self.id(), stream.method)
    }
}

#[derive(Clone)]
pub struct Features {
    installed: Arc<HashMap<&'static str, Arc<dyn Feature>>>,
}

impl Features {
    pub fn install(phone: Phone) -> Self {
        let screen = screen::Screen::new(phone.clone());
        let features: Vec<Arc<dyn Feature>> = vec![
            nearby::Nearby::start(phone.clone()),
            transfer::Transfer::new(phone.clone()),
            clipboard::Clipboard::start(phone.clone()),
            whatsapp::WhatsApp::new(phone.clone()),
            Arc::new(screen.clone()),
            notifications::Notifications::start(phone.clone(), screen),
            helper::Helper::new(phone.clone()),
            calls::Calls::start(phone.clone()),
            input::RemoteInput::start(phone.clone()),
            Arc::new(updates::Handle(updates::Updates::start(phone.clone()))),
            Arc::new(setup::Handle(setup::Setup::new(phone.clone()))),
            hardware::Hardware::new(phone),
        ];
        let installed = features.into_iter().map(|feature| (feature.id(), feature)).collect();
        Self { installed: Arc::new(installed) }
    }

    pub async fn command(&self, feature: &str, method: &str, body: &Value) -> Option<Result<Value>> {
        self.installed.get(feature)?.command(method, body).await
    }

    pub fn connected(&self, session: &Session) {
        self.installed.values().for_each(|feature| feature.connected(session));
    }

    pub fn disconnected(&self, peer: &PeerId) {
        self.installed.values().for_each(|feature| feature.disconnected(peer));
    }

    pub fn switched(&self, session: Option<&Session>) {
        self.installed.values().for_each(|feature| feature.switched(session));
    }

    pub async fn answer(&self, peer: &PeerId, feature: &str, method: &str, body: &Value) -> Option<Result<Value>> {
        self.installed.get(feature)?.answer(peer, method, body).await
    }

    pub fn stopping(&self) {
        self.installed.values().for_each(|feature| feature.stopping());
    }

    pub fn notice(&self, peer: &PeerId, feature: &str, method: &str, body: &Value) -> bool {
        self.installed.get(feature).is_some_and(|feature| feature.notice(peer, method, body))
    }

    pub async fn stream(&self, feature: &str, stream: IncomingStream) -> Result<()> {
        match self.installed.get(feature) {
            Some(feature) => feature.stream(stream).await,
            None => bail!("no feature {feature} on this side"),
        }
    }
}
