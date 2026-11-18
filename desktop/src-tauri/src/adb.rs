//! Android's adb, which starts Orither's helper on a phone and sets a phone up over USB.
//!
//! A copy of adb the person installed comes first: two different adb versions on one computer
//! stop each other's server, so Orither uses theirs when there is one. Otherwise it uses the
//! copy the installer carries (`resources/adb`), so nobody has to install anything.

use std::{
    env,
    path::{Path, PathBuf},
    process::Output,
    sync::OnceLock,
};

use anyhow::{Context, Result};
use tauri::{AppHandle, Manager};
use tokio::process::Command;

/// The adb that came with Orither, found once at start.
static BUNDLED: OnceLock<Option<PathBuf>> = OnceLock::new();

const NAME: &str = if cfg!(windows) { "adb.exe" } else { "adb" };

pub const MISSING: &str = "Orither could not find Android's adb tool, which it needs to reach your phone over USB. Reinstalling Orither brings it back.";

/// Remembers where the installer put Orither's own adb.
pub fn locate(app: &AppHandle) {
    let bundled = app.path().resource_dir().ok().map(|dir| dir.join("adb").join(NAME)).filter(|path| path.is_file());
    let _ = BUNDLED.set(bundled);
}

/// The adb to run: one the person installed, else Orither's own, else whatever `adb` the PATH finds.
pub fn tool() -> PathBuf {
    installed().or_else(|| BUNDLED.get().cloned().flatten()).unwrap_or_else(|| NAME.into())
}

/// Whether there is any adb at all.
pub fn available() -> bool {
    let tool = tool();
    tool.is_absolute() || on_path(&tool).is_some()
}

/// adb from the Android SDK, or from the system's packages, where the person installed it.
fn installed() -> Option<PathBuf> {
    let in_sdk = |sdk: PathBuf| sdk.join("platform-tools").join(NAME);
    let mut sdks: Vec<PathBuf> = ["ANDROID_HOME", "ANDROID_SDK_ROOT"].iter().filter_map(env::var_os).map(PathBuf::from).collect();
    if cfg!(windows) {
        sdks.extend(env::var_os("LOCALAPPDATA").map(|local| Path::new(&local).join("Android").join("Sdk")));
    } else {
        sdks.extend(env::var_os("HOME").map(|home| Path::new(&home).join("Android/Sdk")));
    }
    sdks.into_iter().map(in_sdk).find(|path| path.is_file()).or_else(|| on_path(Path::new(NAME)))
}

fn on_path(name: &Path) -> Option<PathBuf> {
    env::split_paths(&env::var_os("PATH")?).map(|dir| dir.join(name)).find(|path| path.is_file())
}

/// A command running adb, which on Windows opens no console window of its own.
pub fn command() -> Command {
    let tool = tool();
    let own = BUNDLED.get().and_then(Option::as_ref).is_some_and(|bundled| *bundled == tool);
    let mut command = Command::new(&tool);
    // Orither's own adb finds phones by cable. Its search of the local network listens on
    // every network, so Windows asked to let "adb, publisher unknown" through the firewall,
    // a program the person never installed. The server takes this from the command that
    // starts it. Someone's own adb is theirs to set up.
    if own {
        command.env("ADB_MDNS", "0");
    }
    #[cfg(windows)]
    {
        /// CREATE_NO_WINDOW: adb is a console program, and Orither is not.
        const NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(NO_WINDOW);
    }
    command.kill_on_drop(true);
    command
}

/// Runs adb with `args` and returns what it printed.
pub async fn run(args: &[&str]) -> Result<Output> {
    command().args(args).output().await.context(MISSING)
}
