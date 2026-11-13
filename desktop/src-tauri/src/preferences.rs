//! Small choices that belong to this computer, each kept in a file of its own.

use std::{fs, path::PathBuf};

use serde::{Serialize, de::DeserializeOwned};
use tauri::{AppHandle, Manager};

fn path(app: &AppHandle, name: &str) -> Option<PathBuf> {
    Some(app.path().app_config_dir().ok()?.join(format!("{name}.json")))
}

pub fn read<T: DeserializeOwned>(app: &AppHandle, name: &str) -> Option<T> {
    serde_json::from_slice(&fs::read(path(app, name)?).ok()?).ok()
}

pub fn write(app: &AppHandle, name: &str, value: &impl Serialize) {
    let saved = path(app, name).and_then(|path| {
        fs::create_dir_all(path.parent()?).ok()?;
        fs::write(path, serde_json::to_vec(value).ok()?).ok()
    });
    if saved.is_none() {
        tracing::warn!("could not remember {name}");
    }
}
