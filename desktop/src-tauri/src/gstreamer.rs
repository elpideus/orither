//! Readies GStreamer, which WebKit plays sound and video with, before anything
//! starts it.

use std::{env, ffi::OsStr, path::Path};

use gtk::glib;

/// Where GStreamer looks for its plugins, and where an AppImage keeps them.
const PLUGINS: [(&str, &str); 2] =
    [("GST_PLUGIN_SYSTEM_PATH", "usr/lib/gstreamer"), ("GST_PLUGIN_SYSTEM_PATH_1_0", "usr/lib/gstreamer-1.0")];
/// The app's own folders go by this, as in tauri.conf.json.
const IDENTIFIER: &str = "dev.orither.desktop";

/// Runs first, while no other thread can read the environment.
pub fn prepare() {
    // WebKit's GL video sink can leave a video black or green, as it does
    // with NVIDIA's graphics, while the sound plays on. Without it, WebKit
    // draws the frames itself, which works with any graphics.
    if env::var_os("WEBKIT_GST_DISABLE_GL_SINK").is_none() {
        set("WEBKIT_GST_DISABLE_GL_SINK", "1");
    }
    if let Some(bundle) = env::var_os("APPDIR") {
        bundled(Path::new(&bundle));
    }
}

/// Puts right what the AppImage's launcher gets wrong. It sends GStreamer to
/// the user's QT_PLUGIN_PATH for plugins, and GStreamer loads whatever it
/// finds there: Qt plugins too, which can take a page down with them. The
/// bundle's plugins get a registry apart from the system's, or each would
/// have GStreamer look through every plugin again after the other ran.
fn bundled(bundle: &Path) {
    for (name, folder) in PLUGINS {
        let folder = bundle.join(folder);
        if folder.is_dir() {
            set(name, folder);
        } else {
            // SAFETY: as for `set`.
            unsafe { env::remove_var(name) };
        }
    }
    set("GST_REGISTRY_1_0", glib::user_cache_dir().join(IDENTIFIER).join("gstreamer-registry.bin"));
}

fn set(name: &str, value: impl AsRef<OsStr>) {
    // SAFETY: only `prepare` calls this, before there are other threads.
    unsafe { env::set_var(name, value) };
}
