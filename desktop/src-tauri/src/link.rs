//! Connects the app to the phone: starts the node, routes what it reports to the
//! features and the UI, and gives the UI one generic way to call the phone.

use anyhow::{Context, Result};
use orither_link::{Event, Invite, Node, NodeConfig, PeerId, PeerStatus, Session};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager, State, async_runtime};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::{
    features::{Features, Notice, Phone},
    preferences,
};

/// Where the phone the UI shows is remembered.
const SELECTED: &str = "selected-phone";

pub struct Link {
    node: Node,
    features: Features,
    phone: Phone,
}

impl Link {
    pub async fn start(app: AppHandle, config: NodeConfig) -> Result<Self> {
        let (node, events) = Node::start(config).await?;
        let selected = preferences::read::<String>(&app, SELECTED).and_then(|id| id.parse().ok());
        let phone = Phone::new(app.clone(), node.clone(), selected);
        let features = Features::install(phone.clone());
        async_runtime::spawn(route_events(app, events));
        Ok(Self { node, features, phone })
    }

    pub async fn shutdown(&self) {
        self.features.stopping();
        self.node.shutdown().await;
    }

    pub fn phone(&self) -> &Phone {
        &self.phone
    }

    /// The phone `peer` names, or the one in use.
    pub fn session_for(&self, peer: Option<&str>) -> Result<Session> {
        match peer {
            Some(peer) => self.phone.session_of(&peer.parse().context("invalid device id")?),
            None => self.phone.session(),
        }
    }

    /// Lets a feature on this side answer, and asks the phone otherwise.
    pub async fn call(&self, feature: &str, method: &str, body: Value) -> Result<Value> {
        self.call_on(None, feature, method, body).await
    }

    /// The same, for a particular phone when `peer` names one.
    pub async fn call_on(&self, peer: Option<&str>, feature: &str, method: &str, body: Value) -> Result<Value> {
        if let Some(handled) = self.features.command(feature, method, &body).await {
            return handled;
        }
        self.session_for(peer)?.call(feature, method, body).await
    }

    fn select(&self, peer: Option<PeerId>) {
        if !self.phone.select(peer) {
            return;
        }
        preferences::write(&self.phone.app, SELECTED, &peer.map(|peer| peer.to_string()));
        let session = peer.and_then(|peer| self.node.session(&peer));
        self.features.switched(session.as_ref());
    }
}

async fn route_events(app: AppHandle, mut events: UnboundedReceiver<Event>) {
    while let Some(event) = events.recv().await {
        let link = app.state::<Link>();
        match event {
            Event::Connected(peer) => {
                if let Some(session) = link.node.session(&peer.id) {
                    link.features.connected(&session);
                }
            }
            Event::Disconnected(peer) => link.features.disconnected(&peer),
            Event::Notice { peer, feature, method, body } => {
                if !link.features.notice(&peer, &feature, &method, &body) {
                    let _ = app.emit("notice", Notice { feature, method, body, peer: Some(peer.to_string()) });
                }
            }
            Event::Stream(stream) => {
                let (feature, features) = (stream.feature.clone(), link.features.clone());
                async_runtime::spawn(async move {
                    if let Err(err) = features.stream(&feature, stream).await {
                        tracing::warn!("{feature} stream: {err:#}");
                    }
                });
            }
            Event::Call(call) => {
                let features = link.features.clone();
                async_runtime::spawn(async move {
                    match features.answer(&call.peer, &call.feature, &call.method, &call.body).await {
                        Some(Ok(body)) => call.responder.ok(body),
                        Some(Err(err)) => call.responder.err(format!("{err:#}")),
                        None => call.responder.err("nothing on the computer answers this"),
                    }
                });
            }
            Event::Paired(_) => crate::autostart::enable_once(&app),
            Event::Unpaired(_) | Event::Route { .. } => {}
        }
        let _ = app.emit("peers", link.node.peers());
    }
}

fn text_error(error: anyhow::Error) -> String {
    format!("{error:#}")
}

#[tauri::command]
pub fn device(link: State<'_, Link>) -> Value {
    json!({ "id": link.node.id().to_string(), "name": link.node.device().name })
}

#[tauri::command]
pub fn peers(link: State<'_, Link>) -> Vec<PeerStatus> {
    link.node.peers()
}

#[tauri::command(async)]
pub fn invite(link: State<'_, Link>) -> Invite {
    link.node.invite()
}

#[tauri::command(async)]
pub fn cancel_invite(link: State<'_, Link>) {
    link.node.cancel_invite();
}

#[tauri::command(async)]
pub fn unpair(link: State<'_, Link>, id: String) -> Result<(), String> {
    let peer: PeerId = id.parse().map_err(|_| "invalid device id".to_owned())?;
    link.node.unpair(&peer).map_err(text_error)
}

#[tauri::command]
pub async fn call(
    link: State<'_, Link>,
    feature: String,
    method: String,
    body: Value,
    peer: Option<String>,
) -> Result<Value, String> {
    link.call_on(peer.as_deref(), &feature, &method, body).await.map_err(text_error)
}

/// The phone the UI shows, if one was chosen.
#[tauri::command]
pub fn selected(link: State<'_, Link>) -> Option<String> {
    link.phone.selected().map(|peer| peer.to_string())
}

#[tauri::command(async)]
pub fn select(link: State<'_, Link>, id: Option<String>) -> Result<(), String> {
    let peer = id.map(|id| id.parse::<PeerId>()).transpose().map_err(|_| "invalid device id".to_owned())?;
    link.select(peer);
    Ok(())
}
