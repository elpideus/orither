#!/usr/bin/env bash
# Builds the scrcpy server that the Android app carries (assets/scrcpy-server.jar):
# scrcpy's own release, pinned below, with bit-rate.patch, which lets a computer set
# the video bit rate while a capture runs (control message 100). The version name
# stays scrcpy's, as a helper already running on a phone checks it against its own.
#
# Needs git, a JDK and the Android SDK (platform 36, build-tools 36.0.0), found
# through ANDROID_HOME or ~/Android/Sdk.
#
#   android/scrcpy/build.sh
set -euo pipefail

TAG=v5.0.1
COMMIT=a60891aea193d92e7e5c3942700eca63f9d19a5f

here=$(cd "$(dirname "$0")" && pwd)
out="$here/../app/src/main/assets/scrcpy-server.jar"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

git -c advice.detachedHead=false clone --quiet --depth 1 --branch "$TAG" https://github.com/Genymobile/scrcpy.git "$work/scrcpy"
actual=$(git -C "$work/scrcpy" rev-parse HEAD)
if [ "$actual" != "$COMMIT" ]; then
  echo "scrcpy $TAG is $actual now, not $COMMIT, which the patch was made for." >&2
  exit 1
fi
git -C "$work/scrcpy" apply "$here/bit-rate.patch"

ANDROID_HOME=${ANDROID_HOME:-$HOME/Android/Sdk} BUILD_DIR="$work/build" \
  bash "$work/scrcpy/server/build_without_gradle.sh" > "$work/build.log" 2>&1 || {
  cat "$work/build.log" >&2
  exit 1
}
cp "$work/build/scrcpy-server" "$out"
echo "Wrote $out"
