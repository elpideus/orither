#!/bin/bash
# A Linux desktop session for testing Orither, kept apart from the real one: its own X server (:96),
# session bus and profile, PipeWire, BlueZ and pkexec cut off, and adb on the test phone's server
# only (android.sh). A second Orither would otherwise take the first one's phone devices away.
#
#   desktop.sh start <orither binary or AppImage> [args]
#   desktop.sh click <x> <y>     clicks in the session (XTEST)
#   desktop.sh shot <file.png>
#   desktop.sh stop
HERE=$(cd "$(dirname "$0")" && pwd)
STATE=${ORITHER_TESTBED:-$HOME/.cache/orither-testbed}/desktop
P=$STATE/profile; BUS=unix:path=/tmp/orither-testbed-bus
mkdir -p "$STATE/shim"
for tool in pactl pkexec; do printf '#!/bin/sh\nexit 1\n' > "$STATE/shim/$tool"; chmod +x "$STATE/shim/$tool"; done
mine() { for p in $(pgrep -u "$USER"); do { tr '\0' '\n' < /proc/$p/environ; } 2>/dev/null | grep -qx "XDG_CONFIG_HOME=$P/config\|DBUS_SESSION_BUS_ADDRESS=$BUS" && echo $p; done; }
case "$1" in
start)
  mkdir -p $P/config/dev.orither.desktop $P/data $P/cache $P/state $P/runtime; chmod 700 $P/runtime
  echo true > $P/config/dev.orither.desktop/autostart-set.json
  Xvfb :96 -screen 0 1600x1000x24 -nolisten tcp > "$STATE/xvfb.log" 2>&1 & echo $! > "$STATE/xvfb.pid"
  dbus-daemon --session --address=$BUS --nofork --nopidfile > "$STATE/dbus.log" 2>&1 & echo $! > "$STATE/dbus.pid"
  sleep 1
  setsid env -u WAYLAND_DISPLAY DISPLAY=:96 DBUS_SESSION_BUS_ADDRESS=$BUS GDK_BACKEND=x11 \
    XDG_CONFIG_HOME=$P/config XDG_DATA_HOME=$P/data XDG_CACHE_HOME=$P/cache XDG_STATE_HOME=$P/state \
    XDG_RUNTIME_DIR=$P/runtime PULSE_SERVER=unix:/nonexistent DBUS_SYSTEM_BUS_ADDRESS=unix:path=/nonexistent \
    ANDROID_ADB_SERVER_PORT=5040 ORITHER_PHONE_APK=${ORITHER_PHONE_APK:-} PATH="$STATE/shim:/usr/local/bin:/usr/bin:/bin" \
    RUST_LOG=${RUST_LOG:-info} "$2" "${@:3}" > "$STATE/app.log" 2>&1 < /dev/null &
  echo "log: $STATE/app.log"
  ;;
click)
  python3 -I - "$2" "$3" <<'PY'
import sys, time
from Xlib import X, display
from Xlib.ext import xtest
d = display.Display(":96"); x, y = int(sys.argv[1]), int(sys.argv[2])
d.screen().root.warp_pointer(x, y); d.sync(); time.sleep(0.1)
xtest.fake_input(d, X.ButtonPress, 1); d.sync(); time.sleep(0.05); xtest.fake_input(d, X.ButtonRelease, 1); d.sync()
PY
  ;;
shot) import -display :96 -window root "$2" ;;
stop)
  for p in $(mine); do [ "$(cat /proc/$p/comm 2>/dev/null)" = orither ] && kill -TERM $p; done; sleep 5
  kill -TERM $(mine) 2>/dev/null; sleep 2; left=$(mine); [ -n "$left" ] && kill -KILL $left
  kill "$(cat "$STATE/dbus.pid")" "$(cat "$STATE/xvfb.pid")" 2>/dev/null; rm -f /tmp/orither-testbed-bus
  ;;
*) sed -n '2,10p' "$0"; exit 1 ;;
esac
