# Testing on an emulator and a Windows machine

Real phones and real Windows computers are where Orither has to work, and
[tools/testbed](../tools/testbed) recreates both on one Linux computer:

- an **Android 16 emulator with Google Play**, so Play Protect checks apps the way
  it does on people's phones;
- an **isolated Linux desktop session**, so a test copy of Orither never touches
  the real desktop, its sound devices, Bluetooth or a real phone;
- a **Windows 11 virtual machine**, installed unattended from Microsoft's
  evaluation image, with an agent that runs commands in it.

Nothing here reaches a real phone: the emulator has an adb server of its own on
port 5040, and both test computers use only that one.

## The phone

```bash
tools/testbed/android.sh start
```

Creates the emulator the first time (the Android SDK's `emulator` and the
Android 16 Google Play image, about 2 GB), then boots it without a window or
sound. `android.sh adb …`, `texts`, `tap <text>` and `shot <file>` drive it.
The image runs the release's 64-bit ARM app through Android's translation.

To install a build the way people do, from the Files app, and see what Play
Protect says:

```bash
tools/testbed/android.sh scan android/app/build/outputs/apk/release/app-release.apk
```

Play Protect gives a new APK a verdict in seconds: "This app looks safe", "App
scan recommended" (then the script lets it scan), or "Harmful app blocked".
**Never add `READ_SMS` or `RECEIVE_SMS` to the app**: an app that asks for either,
and for nothing else, is blocked as harmful.

## A Linux computer

```bash
tools/testbed/desktop.sh start target/release/orither
```

Runs that build on its own X server (`:96`), session bus and profile, with
PipeWire, BlueZ and `pkexec` cut off. `desktop.sh shot` and `click` look and
click. A build made with `cargo build` loads its UI from the dev server
(`pnpm --dir desktop dev`); `--features tauri/custom-protocol` builds it in.

To try USB setup with a build that has no release yet, start it with
`ORITHER_PHONE_APK=<path to the APK>`.

## A Windows computer

Once, about 8 GB and twenty minutes:

```bash
tools/testbed/windows/vm.sh iso
```

```bash
tools/testbed/windows/vm.sh install
```

Then, with `vm.sh serve` running in the background:

```bash
tools/testbed/windows/vm.sh run 'hostname'
```

Files put in `~/.cache/orither-testbed/windows/files` are fetched in the machine
from `http://10.0.2.2:8800/files/<name>`. The machine reaches the test phone's
adb server with `ADB_SERVER_SOCKET=tcp:10.0.2.2:5040` set for Orither, so USB
setup sees the emulator as a plugged-in phone. `vm.sh shot`, `click` and `ui`
(UI Automation by name) look and act; elevated dialogs such as the firewall's
need `click`.

To try a Windows build of this checkout, build it with the UI inside and put it
next to adb:

```bash
cargo xwin build --release --target x86_64-pc-windows-msvc -p orither-desktop --features tauri/custom-protocol
```

Toast notifications need Orither installed with its installer, whose Start-menu
shortcut carries the app id Windows shows them under; take the installer from a
manual run of the release workflow.

The machine uses Windows' 90-day evaluation licence and has no GPU, which makes
it a good test of Orither's software rendering paths too.

## Video that lags

`ORITHER_VIDEO_STATS=1` makes Orither write, every two seconds, how the video
arrives (pictures and key frames a second, bit rate, decoding time, and how far
behind the phone's clock the pictures come), how the window draws it, and the
link's paths with their round trips. `ORITHER_TEST_VIDEO_KBPS=1500` makes it take
video no faster than that, which the phone cannot tell from a slow link:

```bash
ORITHER_VIDEO_STATS=1 ORITHER_TEST_VIDEO_KBPS=1500 tools/testbed/desktop.sh start target/release/orither
```

The emulator's own network throttle (`adb emu network speed`) does not reach the
link, so it is no use for this.

## What these found

- Play Protect blocked Orither 0.1.1 as harmful because of its SMS permissions.
- Opening an app failed silently whenever the phone's helper could not start,
  which on Windows was always, since Windows has no adb.
- **Turn everything on** left notification access off on apps installed from a
  file (Android's restricted settings) and said nothing.
- A pairing link that started the app waited for nothing until it gave up.
- Screen sharing, apps and the camera fell ever further behind whenever the link
  carried less than the phone sent: 26 seconds on a 1.5 Mbit/s link. Video is
  paced now (architecture.md, "Keeping video live").
