# How Orither works

```
 Desktop                                         Phone
┌───────────────────────────┐                   ┌───────────────────────────┐
│ UI (React)                │                   │ UI (Compose)              │
│   features/<id>/          │                   │                           │
│        │ call, useNotice  │                   │   feature/<id>/           │
│ Tauri  ▼                  │                   │   (Kotlin Feature)        │
│   features/<id>.rs        │   orither-link    │        │                  │
│   (optional Rust Feature) │◄─────────────────►│        ▼                  │
│        │                  │  QUIC, encrypted  │   Engine (shell user)     │
│        ▼                  │                   │   scrcpy server           │
│ Viewer window (own        │                   │                           │
│ process, wgpu + OpenH264) │                   │                           │
└───────────────────────────┘                   └───────────────────────────┘
```

Three rules hold it together:

- The link knows nothing about features. It carries calls, notices and streams
  addressed by a feature name and a method.
- A feature lives in one folder per side, and is registered on one line.
- Anything the UI calls goes to the phone, unless a Rust feature on the computer
  answers it first.

## The link

`crates/orither-link` is built on [iroh](https://www.iroh.computer): QUIC between
Ed25519 identities, with hole punching, relays and address lookup. Each device
keeps one secret key, and a device's id is its public key. A connection is only
accepted from a paired id.

It offers three operations, all addressed by `feature` and `method`:

| Operation | Use |
| --- | --- |
| `Session::call` | Ask something and await the answer. |
| `Session::notify` | Say something, with no answer. |
| `Session::open` | Open a raw byte stream, for files, video or audio. |

Calls and notices are JSON frames: a 32-bit big-endian length, then JSON, at most
16 MiB. A stream starts with a `StreamHeader`, and then belongs to the feature.

### Staying connected

Exactly one session runs between two devices. The device with the smaller id
dials, and retries on a backoff of 1, 2, 5, 10, 30 and 60 seconds. The other side
can only *ring* it, on a separate ALPN, to say it should dial now. A ring pings
any existing session and only replaces it if the ping fails.

Close code 1 means the pairing has ended, and the receiver forgets the peer.

| ALPN | Use |
| --- | --- |
| `orither/link/1` | The session. |
| `orither/ring/1` | Asking the other side to dial. |
| `orither/path/1` | A short connection that gives an existing session a new path. |
| `orither/pair/1` | Pairing, on throwaway endpoints. |

### Routes

Every path is classified as `Route::{Local, Nearby, Internet, Relay}`, and this is
the order they are preferred in (`RoutePriority`). A path only replaces the
current one on the same route if it is at least 5 ms quicker, which stops flapping.

There is one exception. A direct internet path under 30 ms ranks above Bluetooth,
because a path that quick is running through the local router or close to it, and
Bluetooth could not keep up with it.

Only the QUIC client opens paths, and a new connection opens paths to every known
address. That is what `orither/path/1` is for. When Bluetooth arrives under a
running session, the dialing side holds a short connection open so iroh adds the
Bluetooth path, and the session's selected path then moves over by itself.

### Bluetooth

iroh sees Bluetooth as a custom transport (`nearby.rs`). Any byte stream can carry
it: QUIC packets are written as a 16-bit big-endian length and a payload, after
each side has introduced itself with its 32-byte id. `Node::add_nearby` takes a
stream and adds it as a path. QUIC still does all the encryption.

The computer always dials the phone, and no Bluetooth pairing dialog appears:

1. The phone listens on an insecure RFCOMM socket with service UUID
   `6f726974-6865-7200-8000-00805f9b34fb`. Having the phone dial instead does not
   work, because BlueZ leaves the computer non-connectable by default.
2. Android hides its own Bluetooth address from apps, so the engine reads it from
   `settings get secure bluetooth_address`. The app caches it and sends it to the
   computer as the notice `nearby.address`.
3. The computer finds the RFCOMM channel with a raw SDP request over L2CAP PSM 1,
   and connects. It needs no D-Bus calls to BlueZ for this.
4. It rechecks every 120 seconds, and right after a disconnect. It skips this while
   a local route exists or a link is already up.

Measured between a POCO F7 and a Linux computer: the link is up in about 3 seconds,
with a 55 ms round trip, and carries **64 KiB/s (about 0.5 Mbit/s)**. `screen.rs`
lowers the video to 350 kbit/s at 960 pixels on this route.

`bluer` 0.17 returns from `connect` before the connection is up. `nearby/linux.rs`
works around that in `settled`, by waiting for the socket to turn writable.

### Pairing

The computer shows a token: an 8-character code such as `K7QM-2XPD`, or a 26-character
QR URI, `orither://pair/...`. Argon2id stretches the token into 32 bytes for a
throwaway endpoint secret and 32 bytes for a MAC key, so the joiner derives where to
connect from the token alone. The two sides then exchange their real ids and prove
they hold the token with a keyed BLAKE3 hash.

The host saves the peer before it replies, and the joiner saves before it hangs up,
so neither side can end up paired alone. An invite lasts 5 minutes, and allows 5
attempts.

## The engine

An Android app cannot capture the screen silently, inject input, or watch the
clipboard from the background. The shell user can, and the only way in is adb. So
Orither runs a small program from its own APK as the shell user:

```
CLASSPATH=<apk> app_process / dev.orither.engine.Main <secret>
```

The computer starts it with `adb shell` (`helper::ensure_engine`) whenever a phone
connects. It then keeps running, without the cable, until the phone restarts. The
app reaches it on loopback port 47183, and both sides prove they hold the secret
with HMAC-SHA256.

**An engine survives app reinstalls.** Bump `Protocol.VERSION` whenever engine code
changes, or the old engine keeps running.

The engine runs the scrcpy server for video, control and microphone audio, and
offers `grant`, `activity`, `bluetooth`, `clipboard` and a few more operations.
It reads scrcpy's output to learn the id of each display a capture makes.

### After a phone restart

The link comes back by itself after a restart, but the engine does not, and only
adb can start it. So once the app holds `WRITE_SECURE_SETTINGS`, which the engine
grants for **After restarts**, the computer brings its own adb through the link
(`features/helper.rs` and `feature/helper/Helper.kt`):

1. The computer calls `helper/open`. The phone saves `adb_wifi_enabled` and sets it
   to 1. On a network the user has not trusted, Android turns it straight back off
   and asks them. The phone tells the computer (`asking`) and waits a minute.
2. The phone finds adbd's TLS port through NSD (`_adb-tls-connect._tcp`), keeps the
   announcements from its own addresses that answer on loopback, and replies
   `{state: "ready", port}`.
3. The computer listens on `127.0.0.1:<any>` and carries each connection there over
   a `helper/connect {port}` stream, which the phone joins to `127.0.0.1:<port>`.
4. The computer runs `adb connect` to that address, then the launch command, then
   `adb disconnect`, and sends `helper/close`. The phone puts the setting back.

This uses the computer's own adb key, which the phone already trusts from USB, so
there is no pairing code and no key to copy. adbd sees a connection from the phone
itself, so this works over every route, the relay included.

Wireless debugging stays on only for those seconds. The phone also turns it back
off after 2 minutes, when the computer disconnects, and at the app's next start if
the app stopped first. If the user answers Android after the phone gave up, the
phone turns it off again, for the next 5 minutes.

When a phone connects without its engine, the computer tries once a minute for 15
minutes while the phone has no Wi-Fi or adbd does not answer. Other failures show
as a toast.

**Android forgets a key after 7 days without use** (`adb_allowed_connection_time`).
The computer records when it last reached each phone's adbd (`helper-contact`), and
after 4 days does a quiet run: the same steps without a command. A quiet open never
makes Android ask, so the phone only does it on a network that was trusted before.
Android trusts networks by BSSID, which apps cannot read, so the phone tells them
apart by a hash of the gateways, interface, domains and DHCP server.

**Rehearsal.** For Android to ask while the user is holding the phone, rather than
after a restart, the phone sends `helper/ready` when the user allows something on a
new network. The computer then runs the steps without a command.

Tried on a POCO F7 with Android 16, far from its computer: a stopped helper came
back in about 6 seconds, with wireless debugging off before and after. Android's
question on a new network, and a real restart, are not tested yet.

## Screen and Integrated Mode

The phone encodes H.264 and the link carries it. A window in a process of its own
(`orither --viewer`) decodes it with OpenH264 and draws it with wgpu. Video goes to
the viewer's stdin, its input comes back on stdout, and closing stdin closes it.
A separate process keeps a decoder crash or GPU trouble out of the main app.

Integrated Mode starts the app on a virtual display of its own (`new_display`,
`flex_display`), so a resized window resizes the phone's display and the app reflows.
On Linux it also writes `orither-<package>.desktop`, whose `StartupWMClass` matches
the window, which gives it the app's icon and name in the taskbar and launcher.

A window that follows its size opens before the capture starts. Its first message
is its size, which becomes the display's size from the start, so that an app adapts
to the computer once rather than again a moment later. That size, and anything else
for the control channel, waits for the codec id on the video channel. scrcpy starts
taking control messages before its encoder exists, and a resize in that moment kills
the capture.

**Send to computer** moves an app that already runs. The Quick Settings tile
(`HandoffTile`) only sends `screen.handoff` to the computer, which may first have to
start the engine. A tap sends to the computer used last, which is the last one sent
to or the last to start a capture, if it is connected, or else to the first one that
is. A long press opens the tile's preferences activity (`HandoffPicker`) to choose.
The picker has a task of its own and removes it before it sends, so the app under it
is the one in front again. The computer then asks the phone for the app in front
(`foreground`: the engine collapses the shade and reads the tasks through its
`ShellContext`), opens the app's window, and asks for that task to move onto the
window's display (`move`, which is `am display move-stack`). The app keeps its
process and where it was. Android does recreate its screen for the new size and
density, as it does on rotation, unless the app handles that itself. This display
has `vd_destroy_content=false`, so when the window closes or the link drops, Android
puts the app back on the phone's screen instead of closing it.

In the window, left click is touch, right click is back, and middle click is Home.

## Clipboard

The engine watches the phone's clipboard through a `ShellContext`, and the app writes
it. The phone only reports changes while the computer asks (`follow {on}`). Files go
from the computer into `Download/Orither`, and reach the clipboard as a `FileProvider`
URI. A guard on each side stops a clip from echoing back.

## Hardware

On Linux, `pactl` creates a `module-pipe-sink` and a `module-pipe-source` with FIFOs
in `$XDG_RUNTIME_DIR/orither/`, as s16le 48 kHz stereo. Both set `node.virtual=false`,
because KDE's sound panel hides virtual devices, and a low `priority.session` so they
never become the default on their own. Orither removes them as it quits, on SIGTERM
too, and removes any a crash left behind when it starts.

- **Speaker:** whatever reaches the sink goes to the phone over the stream
  `speaker.play`.
- **Microphone:** the engine's scrcpy audio capture goes into the source, but only
  while a program records from it, which `pactl subscribe` reports. Level meters
  (`stream.monitor`) do not count. The phone keeps listening for 5 seconds after the
  last program stops, and its sound meanwhile is thrown away. PipeWire's pipe source
  still holds the last fraction of a second it had not handed on (it buffers about
  8192 frames) and plays that when the next recording starts, so once the phone stops
  listening the source is unloaded and loaded fresh. A program that starts recording
  within the 5 seconds can still get that fraction. Writes carry whole frames of at
  most 4096 bytes, which a pipe takes whole, so a stop can never leave half a frame in
  the FIFO.
- **Camera:** H.264 from the phone's camera is decoded into a `v4l2loopback` device,
  as YU12. Loading that module takes root, so **Set up** runs `pkexec` once
  (`hardware/camera_setup.rs`): it writes `/etc/modules-load.d/orither-camera.conf`
  and, unless another file already sets the module's options,
  `/etc/modprobe.d/orither-camera.conf` with `card_label="Orither Camera"
  exclusive_caps=1`, then loads the module if nothing has. From then on it loads at
  every start. With `exclusive_caps=1`, programs only list the device while a writer has
  it open with a picture on it, so Orither keeps it open with black on it. The phone
  films only while a program streams from the device, which v4l2loopback 0.13 and
  later report with its `V4L2_EVENT_PRI_CLIENT_USAGE` event. Older versions cannot,
  and then the phone films all the while. As with the microphone, filming goes on for
  5 seconds after the last program stops. Until the phone's first picture arrives,
  black goes out ten times a second, so a program gets pictures at once. A program
  that starts reading gets the last picture on the device first, so black replaces
  the phone's last picture when filming stops.
- **Camera size:** the device's size can only change while no program holds it. It
  opens at the size the phone's pictures had last time, which is kept in
  `hardware-camera-size.json`, and at 1280×960 before that. A picture of another
  size is centred, cut down or edged with black to fit. Once filming stops, the
  device opens again at the phone's size.

## WhatsApp

The WhatsApp page is WhatsApp Web, WhatsApp's own site, loaded from
`web.whatsapp.com`. Orither adds no scripts or user agent of its own, and does only
what a browser does. It never uses an unofficial client, which keeps the account
safe. Only Linux has it so far (`whatsapp/web.rs`).

- **The view** is a second WebKitGTK view. Tauri cannot place one on Linux, so at
  start the window's own view moves into a `gtk::Overlay`, and WhatsApp Web's lies
  over it. The page keeps an empty slot and sends its place whenever it changes,
  with its corner radius and the colour behind it. GTK clips nothing round, so four
  small corners in that colour round the view off. While anything around the slot
  moves or fades, as when the page opens, the view hides. It fades in once the slot
  is still and WhatsApp Web has loaded.
- **Its look** is Orither's, through a user style sheet like a browser's
  (`whatsapp/look.rs`). WhatsApp Web keeps its colours in CSS variables, mostly its
  design system's `--WDS-…`, each with an `-RGB` twin for see-through shades. The
  sheet sets them from the theme on every element, since WhatsApp sets them at
  several depths, along with the system font, the caret, selection and scrollbars.
  Chat lists and bars take the view colour, the chat itself the window colour, and
  bubbles the button colour, the user's own tinted with the accent. It is made
  again whenever the theme changes. Should WhatsApp rename its variables, its own
  colours show through, and nothing breaks.
- **Video** plays without WebKit's GL video sink (`WEBKIT_GST_DISABLE_GL_SINK`, set
  in `gstreamer.rs` unless the environment sets it). With NVIDIA's decoder, and with
  software decoding, that sink left the picture black or green while the sound
  played. In the AppImage, `gstreamer.rs` also points GStreamer at the bundle's own
  plugins, since the AppImage's launcher sends it to `QT_PLUGIN_PATH`, where a Qt
  plugin can take WebKit down, and gives the bundle a registry of its own.
- **Its data**, the login, cookies and cache, lives in `whatsapp-web` under the
  app's data and cache folders. Once that folder exists, WhatsApp Web starts 8
  seconds after Orither, to hear of new messages. Before that, the page starts it.
- **Links** to other sites, and anything meant for a new window, open in the
  computer's browser. Downloads go to Downloads under a name not taken yet, and the
  UI offers to show them. Only WhatsApp may notify and use the microphone and
  camera.
- **Notifications:** while the phone's show on the computer, WhatsApp Web's own are
  held back, since the phone's say the same. A click on one of the phone's WhatsApp
  notifications shows the page and clicks WhatsApp Web's notification for that
  chat, found by the chat's id (`conversation`, the notification's shortcut id) or
  by its title. Without one, a chat whose id is a phone number opens through
  `send?phone=`, which loads WhatsApp Web again. Without the phone's notifications,
  WhatsApp Web's own show, and a click on one shows the page.
- **The home tile** shows the phone's latest WhatsApp chats as its notifications
  showed them, kept in `whatsapp.json` on the phone: each chat's name, its last
  message, and how many messages from others came since the user last wrote. That
  count clears when the notification goes, as it does once the chat is read
  anywhere. The tile counts the larger of the waiting chats and WhatsApp Web's own
  count, from its title.
- **Chat names** come from the conversation title, or else from the other person's
  name on the messages. Android titles an unnamed chat after whoever wrote last,
  which is the user once they reply.

## Notifications

The listener behind WhatsApp's feature also hands every notification to `Shade`.
It is still called `WhatsAppListener`, because Android allows a listener by its
name, and a new name would take back what users allowed. A computer gets the
notifications the user could swipe away, so not ongoing ones, group summaries or
Orither's own, with a chat's latest lines in place of its summary. `alert` is false
for an update the app asked to alert only once, and `quiet` is true below default
importance or when Do Not Disturb holds it back.

On Linux the computer shows them through `org.freedesktop.Notifications` on the
session bus, with the app's name and icon and the notification's own picture, one
per notification key. Elsewhere it only pops up those that alert, through Tauri's
notification plugin, and a click does nothing.

- **A click** opens the app in Integrated Mode, or in the window it has already. The
  engine starts `OpenActivity` on that window's display (`activity` with the
  capture's `session`). It sends the notification's `contentIntent` from there, as a
  tap on the phone does, and lends it its right to start activities, which Android
  allows because it has a window. The intent decides the page, and how an app that
  is open already takes it, exactly as on the phone. Without an activity to open,
  or once the notification is gone, the app opens from its launcher. An auto-cancel
  notification is dismissed afterwards. WhatsApp's open in the WhatsApp page
  instead, see above.
- **Dismissing** one on the computer dismisses it on the phone, and one removed on
  the phone closes on the computer. A click closes the computer's copy first, so
  the server's own close after a click is not taken for a dismissal.
- **On reconnecting**, the computer closes those the phone no longer has, and stays
  quiet about those that came in meanwhile.

## Calls

The phone's `calls` feature lists the calls on the phone in one list, from two
sources, and the computer answers, declines, ends and dials them through it. The
computer becomes the phone's Bluetooth headset, so that a call sounds on its
speakers and hears its microphone.

**Mobile calls** come from telephony, which only says whether the phone is idle,
ringing or in a call. The caller's number comes with Android's phone state
broadcast, to an app that may read the call log, and the name from the contacts.
Telecom answers, ends and places them, as for a headset or a watch. A second call
that rings during one is listed beside it, but once the phone is back in a call,
telephony cannot say whether it was taken, so it is dropped. Outgoing calls show as
active from the start, since telephony never says when the other side picks up.

**Apps' calls**, WhatsApp's, Telegram's, Signal's and the like, come from their
call notifications, which the listener hands to `Ringing` before the notifications
feature leaves them out for being ongoing. A `CallStyle` notification carries its
answer, decline and hang-up intents and its caller. Other apps only mark the
notification a call, which a missed call can be too, so such a notification counts
only while it cannot be swiped away or rings through the screen, and its buttons
are told apart by their titles, in the languages most phones speak. The phone app's
own call notification is the mobile call again, so it only lends that call the
caller's name and picture.

To act on an app's call, the phone sends the notification's intent, as a tap would.
Answering usually opens the app's call screen, which Android lets only an app on
the screen do, so while the screen is on, the engine starts `CallActivity`, which
shows over the lock screen with nothing in it and sends the intent from there.
`place` starts an app's call to a contact the same way the contacts app does: the
engine opens the contact's data row for that app, such as WhatsApp's
`vnd.com.whatsapp.voip.call`. Rows whose type names a call, and which an installed
app opens, count, along with a few known ones.

A call that ends stays in the list as `ended` for 3 seconds.

### Sound

Phones send a call's sound, the mobile network's and every app's, to a Bluetooth
headset. So the computer takes the hands-free side of the Hands-Free Profile, which
PipeWire plays on Linux (`calls/linux.rs`):

1. **Pairing** (`calls/pair`) needs the phone's Bluetooth address, which `nearby`
   learns. BlueZ pairs only with a device it knows of, and it learns of one as soon
   as anything connects to it, so the computer opens a connection to the phone's
   service directory first, which works without the phone being discoverable. It
   then asks BlueZ to pair. The desktop's own Bluetooth agent and Android each show
   a code to confirm. The phone is then trusted, and its calls are wanted here.
2. **Connecting** asks BlueZ for the phone's Hands-Free Audio Gateway profile
   (`0000111f-…`). PipeWire takes the connection and shows it on the session bus as
   an `org.pipewire.Telephony.AudioGateway1` with the phone's address, which is how
   the computer knows it is connected. `calls/route {to: "phone"}` disconnects the
   profile, and the phone takes its calls back. The choice is kept per phone in
   `calls-audio.json`, and while it is the computer, the computer connects again
   when the phone's link comes up and when a call starts, at most every 30 seconds.
   Connecting during a call also asks PipeWire to bring that call's sound over.
3. **The sound** flows while the gateway's transport is `active`. WirePlumber makes
   a phone's call sound two streams that follow the default output and the default
   microphone, so normally nothing more is needed. Should something leave either
   side a plain device that nothing links, the computer links it with a
   `module-loopback` for as long as the call's sound flows.

The computer follows BlueZ's and PipeWire's signals, and tells the UI `calls/audio`
whenever the state changes.

Nothing of this has been tried with a real phone yet. The computer also offers
itself as a Bluetooth speaker, so once connected, Android may play the phone's
music on it too, unless **Media audio** is turned off for the computer in the
phone's Bluetooth settings.

## Theme

`theme.rs` reads `kdeglobals` on KDE, and asks the XDG portal for a light or dark
preference and an accent elsewhere. It checks again every 2 seconds, and the UI
falls back to neutrals close to Breeze Dark with a green accent. The UI derives
every other colour from those seven, and tells a light theme from a dark one by
the window colour's luminance. The phone app follows the phone's own light or
dark setting.

## Updates

`features/updates.rs` keeps the desktop app up to date through Tauri's updater
plugin. It reads `latest.json` from the latest GitHub release 15 seconds after
start and every hour, downloads the file for its kind of installation (the
bundler marks the binary as AppImage, .deb, .rpm or NSIS), and keeps it in the
cache folder once its signature checks out. Restart installs it: the AppImage
is replaced in place, a package goes through `pkexec`, and on Windows the
installer runs in passive mode and opens Orither again. Started at login with a
download from an earlier run waiting, and the window not yet open, it installs
at once, at most twice per version (`update-attempts`). The tray offers the
update too. [releasing.md](releasing.md) covers how releases and their
signatures are made.

## Several phones

The computer can be linked to any number of phones. The UI keeps one in focus,
and the computer remembers it (`selected-phone`). A call from the UI goes to the
phone in focus unless it names another (`peer`), and so do the features that
work with one phone at a time, such as the screen and the microphone, speaker
and camera, which move to the phone in focus when it changes. Notices reach the
UI with the phone they came from, so calls and notifications from every phone
can show together, each tagged with its phone's satellite.

## Trackpad and keyboard

The phone's **Trackpad** screen sends `input` notices: pointer motion already
accelerated on the phone, buttons, smooth scrolling, text and named keys. The
computer turns them into input through the XDG desktop portal's RemoteDesktop
interface, which asks the user once and is then remembered with a restore token
(`input-restore-token`). Text goes as keysyms, named keys as evdev codes, so a
chord such as Ctrl+C is the key Control held, the text "c", and Control released.
The session closes after 10 minutes without input and opens again silently.

## Feature methods

Dates are in milliseconds. These are on the phone unless marked otherwise.

| Feature | Calls | Notices | Streams |
| --- | --- | --- | --- |
| `system` | `battery`, `about` | to computer: `battery` | `wallpaper` |
| `sms` | `threads`, `messages {thread}` | to computer: `received` | |
| `whatsapp` | `chats`, `allow` | to computer: `changed` | |
| `notifications` | `list`, `open {key,app,session?}`, `dismiss {key}` | to computer: `posted`, `removed {key}` | `picture {key}` |
| `gallery` | `albums`, `items {album?,offset,limit}` | | `thumbnail {id,size}`, `original {id}` |
| `files` | `roots`, `list {path}`, `delete {path}` | | `read {path}`, `write {folder,name}` |
| `clipboard` | | `follow {on}`, `set {text}`; to computer: `changed {text?,files}` | `read {uri}`, `put {name,batch,last}` |
| `apps` | `list`, `open {package}` | | `icon {package}` |
| `screen` | `start {options,channels}`, `stop {session}`, `foreground`, `move {session,task}` | to computer: `handoff` | `attach {session,channel}` |
| `speaker` | | | `play` |
| `nearby` | | to computer: `address {address}` | |
| `helper` | `status`, `open {quiet}` | `close`; to computer: `ready`, `asking` | `connect {port}` |
| `calls` | `list`, `answer {id}`, `decline {id}`, `hangup {id}`, `dial {number}`, `contacts {query?,limit?}`, `place {data}`, `recents {limit?}` | to computer: `changed {calls}` | `picture {id}`, `contact-picture {id}` |

The phone calls these on the computer:

| Feature | Calls | Notices to the computer |
| --- | --- | --- |
| `input` | `open {ask?}` | `move {dx,dy}`, `button {button,down}`, `click {button}`, `scroll {dx,dy,end?}`, `text {text}`, `key {key,down?}` |

The computer answers these calls from the UI itself:

| Feature | Calls | Notices to the UI |
| --- | --- | --- |
| `transfer` | `download`, `open`, `upload` | `progress`, `failed` |
| `screen` | `view {screenOff}`, `app {package,label?}` | `failed` |
| `whatsapp` | `web`, `place {at}`, `reload`, `reveal {path}`, `chat {title,id?}` | `web`, `show`, `saved {name,path}`, `failed {error}` |
| `notifications` | `state`, `enable {on}`, `open {key,app,label,title?,conversation?}` | `posted`, `removed {key}`, passed on from the phone |
| `clipboard` | `state`, `enable`, `copy` | |
| `hardware` | `devices`, `set`, `setup {id}` | `failed` |
| `input` | `state` | `state {supported,active}` |
| `helper` | | `waking`, `woke`, `rehearsing`, `rehearsed`, `failed {error}` |
| `calls` | `audio {peer?}`, `pair {peer?}`, `route {to,peer?}` | `audio {state,where,detail}` |
| `updates` | `status`, `check`, `download`, `install`, `open`, `auto {on}` | `status` |

The UI loads media through the `orither://localhost/<feature>/<method>?b=<json>&p=<phone>`
URI scheme, which turns a stream from that phone into something an `<img>` tag can show.
