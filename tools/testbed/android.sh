#!/bin/bash
# The Android test phone: an emulator with Google Play (so Play Protect runs), reached through an adb
# server of its own on port 5040, so nothing here ever touches a real phone plugged into this computer.
#
#   android.sh start            create the Android 16 emulator if needed, boot it headless and silent
#   android.sh adb <args>       adb, on the test server
#   android.sh texts            the texts on the phone's screen, one per line
#   android.sh tap <text> [n]   taps the n-th element whose text or description is exactly <text>
#   android.sh shot <file.png>  a screenshot
#   android.sh scan <apk>       installs <apk> from the Files app as a person would, lets Play Protect
#                               scan it, and prints its verdict
#   android.sh stop
set -e
SDK=${ANDROID_HOME:-$HOME/Android/Sdk}
export ANDROID_ADB_SERVER_PORT=5040
AVD=orither-a16
IMAGE="system-images;android-36;google_apis_playstore;x86_64"
SERIAL=emulator-5580
adb() { "$SDK/platform-tools/adb" -P 5040 "$@"; }
texts() { adb -s $SERIAL exec-out uiautomator dump /dev/tty 2>/dev/null | grep -o -E '(text|content-desc)="[^"]+"' | sed -E 's/^(text|content-desc)="//; s/"$//'; }
tap() {
  local xml; xml=$(adb -s $SERIAL exec-out uiautomator dump /dev/tty 2>/dev/null)
  local point; point=$(python3 -I - "$xml" "$1" "${2:-0}" <<'PY'
import re, sys
xml, want, nth = sys.argv[1], sys.argv[2], int(sys.argv[3])
hits = []
for node in re.findall(r"<node [^>]*>", xml):
    t = re.search(r' text="([^"]*)"', node); d = re.search(r'content-desc="([^"]*)"', node)
    if (t and t.group(1) == want) or (d and d.group(1) == want):
        x1, y1, x2, y2 = map(int, re.search(r'bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"', node).groups())
        hits.append(f"{(x1 + x2) // 2} {(y1 + y2) // 2}")
print(hits[nth] if len(hits) > nth else "")
PY
)
  [ -n "$point" ] || { echo "no element: $1" >&2; return 1; }
  adb -s $SERIAL shell input tap $point
}
case "$1" in
start)
  if ! "$SDK/cmdline-tools/latest/bin/avdmanager" list avd -c | grep -qx $AVD; then
    "$SDK/cmdline-tools/latest/bin/sdkmanager" --install emulator "$IMAGE" < /dev/null
    echo no | "$SDK/cmdline-tools/latest/bin/avdmanager" create avd -n $AVD -k "$IMAGE" -d pixel_7 --force
    sed -i 's/^PlayStore.enabled=no/PlayStore.enabled=yes/; s/^hw.ramSize=.*/hw.ramSize=4096/' "$HOME/.android/avd/$AVD.avd/config.ini"
  fi
  adb start-server
  nohup "$SDK/emulator/emulator" -avd $AVD -port 5580 -no-window -no-audio -no-boot-anim -gpu swiftshader_indirect -no-snapshot-save > /tmp/orither-emulator.log 2>&1 &
  until [ "$(adb -s $SERIAL shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" = 1 ]; do sleep 3; done
  echo "$SERIAL is up"
  ;;
adb) shift; adb "$@" ;;
texts) texts ;;
tap) tap "$2" "$3" ;;
shot) adb -s $SERIAL exec-out screencap -p > "$2" ;;
scan)
  name=scan-$RANDOM.apk
  adb -s $SERIAL uninstall dev.orither > /dev/null 2>&1 || true
  adb -s $SERIAL push "$2" /sdcard/Download/$name > /dev/null
  adb -s $SERIAL shell am start -a android.intent.action.VIEW -d content://com.android.externalstorage.documents/document/primary%3ADownload -t vnd.android.document/directory > /dev/null 2>&1
  sleep 3; tap $name; sleep 4; tap Install || true; sleep 10
  verdict=$(texts | tr '\n' '|')
  case "$verdict" in *"Scan app"*) tap "Scan app"; sleep 20; verdict=$(texts | tr '\n' '|');; esac
  echo "Play Protect: $verdict"
  adb -s $SERIAL shell rm -f /sdcard/Download/$name
  ;;
stop) adb -s $SERIAL emu kill ;;
*) sed -n '2,16p' "$0"; exit 1 ;;
esac
