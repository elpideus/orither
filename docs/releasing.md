# Releasing

A release is a `v*` tag on `main`. Pushing one starts
[the release workflow](../.github/workflows/release.yml), which builds, signs
and publishes everything. Nobody builds release files by hand.

## Cutting a release

```bash
node scripts/release.mjs 0.2.0
```

The script refuses unless the working tree is clean, the branch is `main` and
`main` is not behind `origin/main`. Then it sets the version in every file that
holds it ([scripts/version.mjs](../scripts/version.mjs)), commits
"Release v0.2.0", tags `v0.2.0` and pushes both. `--no-push` stops before the
push and prints the command for later.

A version with a `-`, such as `0.3.0-beta.1`, is a **pre-release**. It is
published like any release, but never marked as the latest, so installed copies
do not update to it and the README's download links skip it.

The version lives in `Cargo.toml` (`[workspace.package]`), `Cargo.lock`,
`desktop/package.json`, `desktop/src-tauri/tauri.conf.json` and
`android/app/build.gradle.kts`. CI fails when they disagree:

```bash
node scripts/version.mjs --check
```

The Android version code follows from the version: 1.2.3 is 1002003.

## What the workflow does

1. **Checks**: the whole of [CI](../.github/workflows/ci.yml) runs on the tag.
2. **Builds**, each on its own runner, none of which sees a signing key. The
   desktop builds carry Android's adb (`scripts/fetch-adb.mjs`, pinned to a
   platform-tools version and its checksum, passed to `tauri build` with
   `--config src-tauri/adb.<os>.conf.json`):
   - Windows: the NSIS installer, `Orither-Setup-x64.exe`.
   - Linux, on Ubuntu 22.04 so the binaries need no newer glibc than it has:
     `Orither-x86_64.AppImage`, `orither_amd64.deb` and `orither.x86_64.rpm`.
   - Android: `Orither-android-arm64.apk`, signed with a throwaway debug key.
3. **Sign**, on a fresh runner with no checkout and no cache, in the `release`
   environment: the four desktop files get an updater signature (a `.sig` file
   beside each, made by the Tauri CLI and bound to the version), and the APK is
   signed again with Orither's Android key.
4. **Publish**: the release notes (the commit subjects since the previous
   release, then how to install), `latest.json`
   ([scripts/updater-manifest.mjs](../scripts/updater-manifest.mjs)) and
   `SHA256SUMS`, in a GitHub release named "Orither 0.2.0".

The files have no version in their names, so
`releases/latest/download/<file>` always reaches the newest one, and an
AppImage that has updated itself does not carry an old version in its name.

Running the workflow by hand (Actions › Release › Run workflow) builds
everything from the chosen branch and keeps it as workflow artifacts, unsigned,
publishing nothing. Use it to try a build before tagging.

A failed run can be re-run from the Actions page. Publishing again replaces the
release's files and notes instead of failing.

## How installed copies update

`plugins.updater` in `desktop/src-tauri/tauri.conf.json` points the app at
`https://github.com/elpideus/orither/releases/latest/download/latest.json` and
holds the public half of the updater key. `latest.json` names, for each kind of
installation, the file to download and its signature:

| Target | File |
| --- | --- |
| `windows-x86_64-nsis`, `windows-x86_64` | `Orither-Setup-x64.exe` |
| `linux-x86_64-appimage` | `Orither-x86_64.AppImage` |
| `linux-x86_64-deb` | `orither_amd64.deb` |
| `linux-x86_64-rpm` | `orither.x86_64.rpm` |

The app knows which kind it is because Tauri's bundler marks the binary in each
package. It downloads the matching file, refuses it unless its signature checks
out against the public key and names the announced version, and keeps it in
the cache folder until it installs it
([features/updates.rs](../desktop/src-tauri/src/features/updates.rs)). The
repository must stay public: installed apps read releases without signing in.

## The signing keys

Two keys sign releases. Their only copies outside GitHub are in
`~/.orither-signing` on the computer that made them, and that folder must be
backed up somewhere safe and private:

| File | What it signs | If it is lost |
| --- | --- | --- |
| `updater.key`, `updater.password` | Desktop updates | Installed copies can never update themselves again. Everyone has to download a release built with a new key by hand. |
| `android.jks`, `android.password` (alias `orither`) | The Android app | The app cannot be updated in place. Everyone has to uninstall it, losing its pairing, and install the new one. |

They reach the workflow as secrets of the repository's `release` environment,
which only `v*` tags may use:

| Secret | Value |
| --- | --- |
| `TAURI_SIGNING_PRIVATE_KEY` | The contents of `updater.key` |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | The contents of `updater.password` |
| `ANDROID_KEYSTORE_BASE64` | `base64 -w0 android.jks` |
| `ANDROID_KEYSTORE_PASSWORD` | The contents of `android.password` |

To set them again, for instance on a new repository:

```bash
cd ~/.orither-signing
gh secret set TAURI_SIGNING_PRIVATE_KEY --env release < updater.key
gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD --env release < updater.password
base64 -w0 android.jks | gh secret set ANDROID_KEYSTORE_BASE64 --env release
gh secret set ANDROID_KEYSTORE_PASSWORD --env release < android.password
```

To sign a local Android build with the release key, pass it to Gradle:

```bash
cd android
ORITHER_KEYSTORE=~/.orither-signing/android.jks \
ORITHER_KEYSTORE_PASSWORD="$(cat ~/.orither-signing/android.password)" \
ORITHER_KEY_ALIAS=orither \
ORITHER_KEY_PASSWORD="$(cat ~/.orither-signing/android.password)" \
./gradlew :app:assembleRelease
```

Without those variables, release builds are signed with the computer's debug
key, which installs only where no release-signed Orither is installed.

## Play Protect

Play Protect judges an APK from outside Google Play by what it asks for. It
blocks one that asks for `READ_SMS` or `RECEIVE_SMS` as harmful, whatever else it
does, which is why the app reads texts through its helper. Before releasing a
change to `AndroidManifest.xml`, let Play Protect judge the new APK on the test
emulator ([testing.md](testing.md)):

```bash
tools/testbed/android.sh scan android/app/build/outputs/apk/release/app-release.apk
```

It should say "This app looks safe". If a release is blocked anyway, the developer
can ask Google to review it as a false positive at
<https://support.google.com/googleplay/android-developer/contact/protectappeals>.

## Not done yet

- The Windows installer is not code-signed, so SmartScreen warns about it until
  it builds a reputation. A code-signing certificate would go in the sign job.
- The Android app does not update itself. Plugging the phone in and choosing
  **Set up** installs the version that matches the computer; the desktop could
  offer that by itself when the versions differ.
- Release builds of the Android app are not shrunk with R8 yet
  (`isMinifyEnabled` in `android/app/build.gradle.kts`): nobody has tried a
  shrunk build on a phone.
