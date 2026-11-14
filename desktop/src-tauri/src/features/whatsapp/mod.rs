//! WhatsApp. Its page is WhatsApp Web, see [`web`]. The recent chats on its
//! tile come from the phone's notifications, and the phone keeps them.

#[cfg(any(target_os = "linux", windows))]
mod common;
#[cfg(target_os = "linux")]
mod look;
#[cfg(target_os = "linux")]
mod web;
#[cfg(windows)]
#[path = "web_windows.rs"]
mod web;

use std::sync::Arc;

use anyhow::Result;
use serde_json::Value;

use super::{Feature, Phone};

/// The phone's WhatsApp apps, whose notifications open in the WhatsApp page.
const APPS: [&str; 2] = ["com.whatsapp", "com.whatsapp.w4b"];
/// What the page asks of WhatsApp Web. The rest goes to the phone.
const METHODS: [&str; 4] = ["web", "place", "reload", "reveal"];

pub fn owns(app: &str) -> bool {
    APPS.contains(&app)
}

/// The chat a notification from the phone is about.
pub struct Chat {
    pub title: String,
    /// WhatsApp's own id for it, when the phone said.
    pub id: Option<String>,
}

/// Shows the WhatsApp page, on this chat if WhatsApp Web can find it.
pub fn open(phone: &Phone, chat: Chat) {
    show_page(phone);
    web::open(phone, chat);
}

fn show_page(phone: &Phone) {
    crate::show_window(&phone.app);
    phone.tell_ui("whatsapp", "show", Value::Null);
}

pub struct WhatsApp {
    phone: Phone,
}

impl WhatsApp {
    pub fn new(phone: Phone) -> Arc<dyn Feature> {
        web::install(&phone);
        Arc::new(Self { phone })
    }
}

#[async_trait::async_trait]
impl Feature for WhatsApp {
    fn id(&self) -> &'static str {
        "whatsapp"
    }

    async fn command(&self, method: &str, body: &Value) -> Option<Result<Value>> {
        // A chat picked from the home page opens like a click on its notification.
        if method == "chat" {
            let title = body["title"].as_str().unwrap_or_default().to_owned();
            open(&self.phone, Chat { title, id: body["id"].as_str().map(str::to_owned) });
            return Some(Ok(Value::Null));
        }
        if !METHODS.contains(&method) {
            return None;
        }
        Some(web::command(&self.phone, method, body).await)
    }
}

/// Elsewhere, Orither has no WhatsApp Web of its own yet.
#[cfg(not(any(target_os = "linux", windows)))]
mod web {
    use anyhow::{Result, bail};
    use serde_json::{Value, json};

    use super::{Chat, Phone};

    pub fn install(_phone: &Phone) {}

    pub fn open(_phone: &Phone, _chat: Chat) {}

    pub async fn command(_phone: &Phone, method: &str, _body: &Value) -> Result<Value> {
        match method {
            "web" => Ok(json!({ "state": "unsupported", "unread": 0 })),
            _ => bail!("WhatsApp Web is only built in on Linux and Windows for now"),
        }
    }
}
