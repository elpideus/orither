//! What WhatsApp Web is the same in on every system that has it: where it lives,
//! what the page hears of it, and how a chat from the phone is found in it. Each
//! system's own view is in [`web`](super::web).

use std::{
    iter,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::Value;
use tauri::Manager;
use tauri_plugin_opener::OpenerExt;
use tokio::sync::oneshot;

use super::Chat;
use crate::features::Phone;

pub const HOME: &str = "https://web.whatsapp.com/";
pub const HOST: &str = "web.whatsapp.com";
/// Under the app's data folders.
pub const FOLDER: &str = "whatsapp-web";
/// Once linked, WhatsApp Web starts with Orither, to hear of new messages. It
/// waits a little, as the window comes first.
pub const START_AFTER: Duration = Duration::from_secs(8);
/// How many of WhatsApp Web's notifications are kept from the screen at most.
pub const QUIET_KEPT: usize = 32;
/// What a link leaving WhatsApp may open on this computer.
pub const OUTSIDE: [&str; 4] = ["https", "http", "mailto", "tel"];

#[derive(Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    /// Not started since Orither did.
    Off,
    Opening,
    Open,
    Failed,
}

#[derive(Clone, PartialEq, Serialize)]
pub struct Status {
    pub state: State,
    /// What WhatsApp Web's title counts.
    pub unread: u32,
    pub error: Option<String>,
}

impl Status {
    pub const OFF: Self = Self { state: State::Off, unread: 0, error: None };
}

/// A name in the folder that nothing has yet, as in "photo (2).jpg".
pub fn free_name(folder: &Path, suggested: &str) -> PathBuf {
    let name = Path::new(suggested).file_name().map_or_else(|| "Download".into(), ToOwned::to_owned);
    let path = folder.join(&name);
    if !path.exists() {
        return path;
    }
    let name = Path::new(&name);
    let stem = name.file_stem().unwrap_or_default().to_string_lossy();
    let extension = name.extension().map(|extension| format!(".{}", extension.to_string_lossy())).unwrap_or_default();
    (2..).map(|n| folder.join(format!("{stem} ({n}){extension}"))).find(|path| !path.exists()).expect("a free name")
}

/// What WhatsApp Web's title counts, as in "(3) WhatsApp".
pub fn unread(title: &str) -> u32 {
    let count = title.strip_prefix('(').and_then(|rest| rest.split_once(')'));
    count.and_then(|(count, _)| count.parse().ok()).unwrap_or(0)
}

/// The user part of a WhatsApp id, as in "391234567@s.whatsapp.net".
fn user(id: &str) -> &str {
    id.split(['@', ':']).next().unwrap_or_default()
}

/// The phone number a chat's id holds, if it is one.
pub fn phone_number(id: &str) -> Option<&str> {
    let (number, server) = id.split_once('@')?;
    let digits = !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit());
    (server == "s.whatsapp.net" && digits).then_some(number)
}

/// A chat's title, without the count the phone may add, as in "Ana (2 messages)".
fn uncounted(title: &str) -> String {
    let title = title.trim();
    let counted = title.strip_suffix(')').and_then(|rest| rest.rsplit_once(" ("));
    let counted = counted.filter(|(_, count)| count.starts_with(|c: char| c.is_ascii_digit()));
    counted.map_or(title, |(title, _)| title).trim().to_lowercase()
}

/// Whether a title on the phone and one on WhatsApp Web name the same chat. A
/// group's on the phone may add its sender, as in "Family: Ana".
fn same_chat(phone: &str, web: &str) -> bool {
    let web = uncounted(web);
    let group = phone.split_once(": ").map(|(group, _)| group);
    !web.is_empty() && iter::once(phone).chain(group).any(|title| uncounted(title) == web)
}

/// Whether one of WhatsApp Web's notifications, by its tag and title, is about
/// this chat from the phone. WhatsApp tags a chat's notifications with its id.
pub fn is_about(chat: &Chat, tag: Option<&str>, title: Option<&str>) -> bool {
    let user = chat.id.as_deref().map(self::user).filter(|user| !user.is_empty());
    let by_tag = tag.is_some_and(|tag| Some(self::user(tag)) == user);
    by_tag || title.is_some_and(|title| same_chat(&chat.title, title))
}

/// Shows a download in the file manager. Only those in Downloads, which is
/// where WhatsApp Web saves.
pub fn reveal(phone: &Phone, body: &Value) -> Result<Value> {
    let path = Path::new(body["path"].as_str().context("which download?")?).canonicalize()?;
    if !path.starts_with(phone.app.path().download_dir()?.canonicalize()?) {
        bail!("{} is not a download", path.display());
    }
    phone.app.opener().reveal_item_in_dir(&path)?;
    Ok(Value::Null)
}

/// Runs `work` on the main thread, where the views live, and hands back its answer.
pub fn on_main<T: Send + 'static>(
    phone: &Phone,
    work: impl FnOnce(&Phone) -> Result<T> + Send + 'static,
) -> Result<oneshot::Receiver<Result<T>>> {
    let (done, answer) = oneshot::channel();
    let there = phone.clone();
    phone.app.run_on_main_thread(move || {
        let _ = done.send(work(&there));
    })?;
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat(title: &str, id: Option<&str>) -> Chat {
        Chat { title: title.into(), id: id.map(str::to_owned) }
    }

    #[test]
    fn counts_what_the_title_counts() {
        assert_eq!(unread("(12) WhatsApp"), 12);
        assert_eq!(unread("WhatsApp"), 0);
        assert_eq!(unread("(WhatsApp)"), 0);
    }

    #[test]
    fn finds_a_number_in_a_chat_id() {
        assert_eq!(phone_number("391234567@s.whatsapp.net"), Some("391234567"));
        assert_eq!(phone_number("1203630@g.us"), None);
        assert_eq!(phone_number("@s.whatsapp.net"), None);
        assert_eq!(user("391234567:12@s.whatsapp.net"), "391234567");
    }

    #[test]
    fn matches_titles_across_devices() {
        assert!(same_chat("Ana (2 messages)", "Ana"));
        assert!(same_chat("Family (3 messages): Ana", "family"));
        assert!(same_chat("Team (Work)", "Team (Work)"));
        assert!(!same_chat("Ana", "Anabel"));
        assert!(!same_chat("Ana", ""));
    }

    #[test]
    fn finds_a_chats_notification() {
        let ana = chat("Ana", Some("391234567@s.whatsapp.net"));
        assert!(is_about(&ana, Some("391234567:3@s.whatsapp.net"), Some("Someone else")));
        assert!(is_about(&ana, None, Some("Ana")));
        assert!(!is_about(&ana, Some("4455@s.whatsapp.net"), Some("Bea")));
        assert!(!is_about(&chat("Bea", None), Some(""), None));
    }

    #[test]
    fn picks_a_name_nothing_has() {
        let folder = std::env::temp_dir().join(format!("orither-names-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        assert_eq!(free_name(&folder, "photo.jpg"), folder.join("photo.jpg"));
        std::fs::write(folder.join("photo.jpg"), b"").unwrap();
        std::fs::write(folder.join("photo (2).jpg"), b"").unwrap();
        assert_eq!(free_name(&folder, "photo.jpg"), folder.join("photo (3).jpg"));
        assert_eq!(free_name(&folder, "../notes"), folder.join("notes"));
        std::fs::remove_dir_all(folder).unwrap();
    }
}
