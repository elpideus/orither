//! The one-time setup the camera needs on Linux. Programs see the phone's camera
//! through a v4l2loopback device, and loading that driver takes root. Rather than
//! hand people a `modprobe` command to type after every restart, Orither asks for
//! the password once, through polkit, and writes two small files that load the
//! driver with Orither's options at every start from then on.

use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
};

use anyhow::{Result, bail};

use super::CAMERA_LABEL;

/// Loads the driver at every start.
const LOAD_AT_START: &str = "/etc/modules-load.d/orither-camera.conf";
/// Gives the driver Orither's options whenever it loads.
const OPTIONS: &str = "/etc/modprobe.d/orither-camera.conf";
/// Where modprobe reads options from, so options set by someone else can be found.
const OPTION_FOLDERS: [&str; 3] = ["/etc/modprobe.d", "/run/modprobe.d", "/usr/lib/modprobe.d"];

/// Why the camera device is missing, as far as this computer can tell.
#[derive(Debug, PartialEq, Eq)]
pub enum Missing {
    /// The driver is not installed at all, which is a package to install.
    Driver,
    /// The driver is there, and one setup with the password makes the device.
    Setup,
    /// The driver runs already with someone else's options, so Orither's apply after a restart.
    Restart,
}

/// What stands between this computer and a camera device, given that there is none now.
pub fn missing() -> Missing {
    if !driver_installed() {
        Missing::Driver
    } else if Path::new("/sys/module/v4l2loopback").exists() && Path::new(OPTIONS).exists() {
        // Another program, such as OBS, loaded the driver before Orither's options could apply.
        Missing::Restart
    } else {
        Missing::Setup
    }
}

fn driver_installed() -> bool {
    Command::new("modinfo")
        .args(["--field", "filename", "v4l2loopback"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Whether options for v4l2loopback come from a file Orither did not write, which
/// Orither's own would fight with.
fn foreign_options() -> bool {
    OPTION_FOLDERS
        .iter()
        .filter_map(|folder| fs::read_dir(folder).ok())
        .flatten()
        .flatten()
        .filter(|entry| entry.path() != Path::new(OPTIONS))
        .filter_map(|entry| fs::read_to_string(entry.path()).ok())
        .any(|text| sets_options(&text))
}

fn sets_options(config: &str) -> bool {
    config.lines().any(|line| {
        let mut words = line.split_whitespace();
        words.next() == Some("options") && words.next() == Some("v4l2loopback")
    })
}

/// The commands that run as root: write both files, then load the driver now if
/// nothing has loaded it yet. Kept free of anything the user could influence.
fn script(write_options: bool) -> String {
    let mut script = String::from("set -e\n");
    script.push_str(&format!("printf '%s\\n' '# Written by Orither, so the phone camera can be a webcam.' 'v4l2loopback' > {LOAD_AT_START}\n"));
    if write_options {
        script.push_str(&format!(
            "printf '%s\\n' '# Written by Orither.' 'options v4l2loopback card_label=\"{CAMERA_LABEL}\" exclusive_caps=1' > {OPTIONS}\n"
        ));
    }
    script.push_str("[ -d /sys/module/v4l2loopback ] || modprobe v4l2loopback\n");
    script
}

/// Asks for the password once and sets the driver up for good.
pub async fn set_up() -> Result<()> {
    if !driver_installed() {
        bail!("{}", Missing::Driver.explain());
    }
    let script = script(!foreign_options());
    let status = tokio::process::Command::new("pkexec")
        .args(["/bin/sh", "-c", &script])
        .stdin(Stdio::null())
        .status()
        .await
        .map_err(|_| anyhow::anyhow!("This computer has no pkexec to ask for the password with."))?;
    match status.code() {
        Some(0) => Ok(()),
        // pkexec's own codes: the person closed the dialog, or the password was not accepted.
        Some(126) => bail!("The password dialog was closed, so nothing changed."),
        Some(127) => bail!("The password was not accepted, so nothing changed."),
        _ => bail!("Setting up the camera driver did not work."),
    }
}

impl Missing {
    /// What the person reads beside the camera.
    pub fn explain(&self) -> String {
        match self {
            Missing::Driver => "Needs the v4l2loopback driver. Install it from your distribution (often v4l2loopback-dkms), then set it up here.".into(),
            Missing::Setup => "Needs a one-time setup that asks for your password. After that it works after every restart.".into(),
            Missing::Restart => "Set up. Another program loaded the driver first, so it applies after the next restart.".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_options_for_the_driver_only() {
        assert!(sets_options("# obs\noptions v4l2loopback exclusive_caps=1\n"));
        assert!(!sets_options("options snd_hda_intel power_save=0\n#options v4l2loopback x=1"));
    }

    #[test]
    fn script_names_the_device_and_loads_only_when_needed() {
        let written = script(true);
        assert!(written.contains(&format!("card_label=\"{CAMERA_LABEL}\"")));
        assert!(written.ends_with("[ -d /sys/module/v4l2loopback ] || modprobe v4l2loopback\n"));
        assert!(!script(false).contains("options v4l2loopback"));
    }
}
