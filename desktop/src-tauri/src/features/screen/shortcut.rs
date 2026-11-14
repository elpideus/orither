//! What the `.desktop` file is on Linux, a Start-menu shortcut is on Windows: it puts
//! a phone app among the computer's own, opens it with `--open-app`, and carries the
//! app's icon. It also carries the id that the app's window gives itself (see
//! [`orither_viewer`]), which makes the taskbar show that window with the shortcut's
//! name and icon, as a button of its own that can be pinned.
//!
//! Making the icon and the file name is plain work, tested everywhere. The shortcut
//! itself is written through the shell's COM objects.

/// The Start menu folder that holds the phone's apps, apart from everything else.
#[cfg(windows)]
const FOLDER: &str = "Orither apps";
#[cfg(windows)]
const DESCRIPTION: &str = "On your phone, through Orither";
/// Keeps the shortcut's whole path within the 260 characters Windows has long allowed.
const LONGEST_NAME: usize = 80;
/// Names that Windows keeps for devices, whatever comes after a dot.
const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3",
    "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];
/// The icon directory's header and its one entry, after which the picture starts.
const ICO_HEADER: u32 = 6 + 16;

/// The shortcut's file name for an app called `label`, which is what the Start menu
/// shows. Characters Windows does not allow in a name become spaces, and a name left
/// empty, or one Windows keeps for itself, falls back on the package.
pub fn file_name(label: &str, package: &str) -> String {
    let allowed: String = label.chars().map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { ' ' } else { c }).collect();
    let words = allowed.split_whitespace().collect::<Vec<_>>().join(" ");
    let short: String = words.chars().take(LONGEST_NAME).collect();
    // Windows drops dots and spaces at the end of a name.
    let name = short.trim_end_matches(['.', ' ']);
    let stem = name.split('.').next().unwrap_or_default().trim_end();
    if name.is_empty() {
        package.to_owned()
    } else if RESERVED.iter().any(|reserved| reserved.eq_ignore_ascii_case(stem)) {
        format!("{name} ({package})")
    } else {
        name.to_owned()
    }
}

/// An icon file holding `png` as it is, which Windows reads since Vista. `None` when
/// `png` is not a PNG.
pub fn ico(png: &[u8]) -> Option<Vec<u8>> {
    // The header comes first: the signature, then the IHDR chunk's length, its
    // type, and the width and height.
    if png.get(..8)? != PNG_SIGNATURE || png.get(12..16)? != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(png.get(16..20)?.try_into().ok()?);
    let height = u32::from_be_bytes(png.get(20..24)?.try_into().ok()?);
    let length = u32::try_from(png.len()).ok()?;
    // Each side fits in a byte, in which 0 stands for 256 or more.
    let side = |pixels: u32| u8::try_from(pixels).unwrap_or(0);
    let mut ico = Vec::with_capacity(png.len() + ICO_HEADER as usize);
    // Reserved, an icon rather than a cursor, one picture.
    ico.extend([0u16, 1, 1].iter().flat_map(|field| field.to_le_bytes()));
    // Its size, no palette, reserved, one plane, 32 bits a pixel.
    ico.extend([side(width), side(height), 0, 0]);
    ico.extend([1u16, 32].iter().flat_map(|field| field.to_le_bytes()));
    ico.extend(length.to_le_bytes());
    ico.extend(ICO_HEADER.to_le_bytes());
    ico.extend_from_slice(png);
    Some(ico)
}

/// Writes the Start-menu shortcut that opens `package`, called `label`, with the
/// icon made from its PNG `icon`, beside it.
#[cfg(windows)]
pub fn write(package: &str, label: &str, icon: &std::path::Path) -> anyhow::Result<()> {
    use std::{env, fs, path::Path};

    use anyhow::{Context, anyhow};

    let roaming = env::var_os("APPDATA").context("no APPDATA folder")?;
    let folder = Path::new(&roaming).join(r"Microsoft\Windows\Start Menu\Programs").join(FOLDER);
    fs::create_dir_all(&folder)?;
    // The icon comes from the phone once, and so does what is made of it.
    let ico_path = icon.with_extension("ico");
    if !ico_path.exists() {
        let ico = ico(&fs::read(icon)?).context("the app's icon is not a PNG")?;
        // Written whole or not at all, as Explorer may read it any moment.
        let part = icon.with_extension("ico.part");
        fs::write(&part, ico)?;
        fs::rename(part, &ico_path)?;
    }
    let link = folder.join(format!("{}.lnk", file_name(label, package)));
    let program = env::current_exe()?;
    let arguments = format!("{} {package}", super::OPEN_APP);
    let id = super::app_id(package);
    // COM is set up and torn down on a thread of its own, which leaves the async
    // runtime's threads as they were.
    let saved = std::thread::scope(|scope| scope.spawn(|| save(&link, &program, &arguments, &ico_path, &id)).join());
    saved.map_err(|_| anyhow!("writing the shortcut stopped"))?.with_context(|| format!("writing {}", link.display()))
}

#[cfg(windows)]
fn save(link: &std::path::Path, program: &std::path::Path, arguments: &str, icon: &std::path::Path, id: &str) -> windows::core::Result<()> {
    use windows::{
        Win32::{
            System::Com::{CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile},
            UI::Shell::{IShellLinkW, ShellLink},
        },
        core::{HSTRING, Interface},
    };

    // SAFETY: COM is ready on this thread from here to `CoUninitialize`, and every
    // object made in between is gone before that.
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE).ok()?;
        let saved = (|| {
            let shortcut: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            shortcut.SetPath(&HSTRING::from(program))?;
            shortcut.SetArguments(&HSTRING::from(arguments))?;
            if let Some(folder) = program.parent() {
                shortcut.SetWorkingDirectory(&HSTRING::from(folder))?;
            }
            shortcut.SetDescription(&HSTRING::from(DESCRIPTION))?;
            shortcut.SetIconLocation(&HSTRING::from(icon), 0)?;
            // Without its id, the shortcut still opens the app, whose window then
            // has a taskbar button of its own but not the shortcut's.
            if let Err(err) = set_id(&shortcut, id) {
                tracing::warn!("giving the shortcut for {id} its id: {err}");
            }
            shortcut.cast::<IPersistFile>()?.Save(&HSTRING::from(link), true)
        })();
        CoUninitialize();
        saved
    }
}

/// Sets `System.AppUserModel.ID`, which the shell wants as a plain wide string.
#[cfg(windows)]
fn set_id(shortcut: &windows::Win32::UI::Shell::IShellLinkW, id: &str) -> windows::core::Result<()> {
    use windows::{
        Win32::{
            Storage::EnhancedStorage::PKEY_AppUserModel_ID,
            System::{
                Com::StructuredStorage::{PROPVARIANT, PVCHF_DEFAULT, PropVariantChangeType},
                Variant::VT_LPWSTR,
            },
            UI::Shell::PropertiesSystem::IPropertyStore,
        },
        core::Interface,
    };

    let store: IPropertyStore = shortcut.cast()?;
    let mut value = PROPVARIANT::default();
    // SAFETY: both values are initialized, and dropping them frees what they hold.
    unsafe {
        PropVariantChangeType(&mut value, &PROPVARIANT::from(id), PVCHF_DEFAULT, VT_LPWSTR)?;
        store.SetValue(&PKEY_AppUserModel_ID, &value)?;
        store.Commit()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The start of a PNG, as far as `ico` reads it.
    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut png = PNG_SIGNATURE.to_vec();
        png.extend(13u32.to_be_bytes());
        png.extend(b"IHDR");
        png.extend(width.to_be_bytes());
        png.extend(height.to_be_bytes());
        png.extend([8, 6, 0, 0, 0]);
        png
    }

    #[test]
    fn an_icon_holds_the_png_as_it_is() {
        let png = png(192, 192);
        let ico = ico(&png).unwrap();
        assert_eq!(ico[..6], [0, 0, 1, 0, 1, 0]);
        assert_eq!(ico[6..10], [192, 192, 0, 0]);
        assert_eq!(ico[10..14], [1, 0, 32, 0]);
        assert_eq!(u32::from_le_bytes(ico[14..18].try_into().unwrap()), png.len() as u32);
        assert_eq!(u32::from_le_bytes(ico[18..22].try_into().unwrap()), 22);
        assert_eq!(ico[22..], png[..]);
    }

    #[test]
    fn a_large_icon_says_so_with_a_zero() {
        assert_eq!(ico(&png(256, 512)).unwrap()[6..8], [0, 0]);
        assert_eq!(ico(&png(255, 16)).unwrap()[6..8], [255, 16]);
    }

    #[test]
    fn only_a_png_makes_an_icon() {
        assert_eq!(ico(b"GIF89a"), None);
        assert_eq!(ico(&png(192, 192)[..20]), None);
        let mut not_header = png(192, 192);
        not_header[12..16].copy_from_slice(b"IDAT");
        assert_eq!(ico(&not_header), None);
    }

    #[test]
    fn a_label_becomes_a_file_name() {
        assert_eq!(file_name("Maps", "com.google.android.apps.maps"), "Maps");
        assert_eq!(file_name("Work: To/Do?  list*", "a.b"), "Work To Do list");
        assert_eq!(file_name("Hello...  ", "a.b"), "Hello");
        assert_eq!(file_name("Line\nbreak\ttab", "a.b"), "Line break tab");
        assert_eq!(file_name("Café 日本", "a.b"), "Café 日本");
        assert_eq!(file_name(" ?*: ", "com.example.app"), "com.example.app");
        assert_eq!(file_name("...", "com.example.app"), "com.example.app");
    }

    #[test]
    fn names_windows_keeps_for_itself_fall_back_on_the_package() {
        assert_eq!(file_name("nul", "com.example.nul"), "nul (com.example.nul)");
        assert_eq!(file_name("Con.fig", "a.b"), "Con.fig (a.b)");
        assert_eq!(file_name("COM10", "a.b"), "COM10");
        assert_eq!(file_name("Console", "a.b"), "Console");
    }

    #[test]
    fn a_long_label_is_cut_short() {
        let name = file_name(&"Long ".repeat(40), "a.b");
        assert!(name.chars().count() <= LONGEST_NAME);
        assert!(name.ends_with("Long"));
    }
}
