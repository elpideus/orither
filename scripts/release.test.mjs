// The release scripts' rules: node --test scripts/
import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

import { UPDATES, manifest } from './updater-manifest.mjs';
import { SEMVER } from './version.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

test('versions are semantic versions without build metadata', () => {
  for (const good of ['0.1.0', '1.20.3', '0.3.0-beta.1', '2.0.0-rc.10']) assert.ok(SEMVER.test(good), good);
  for (const bad of ['v0.1.0', '0.1', '01.2.3', '0.3.0-beta.01', '1.0.0+build.5']) assert.ok(!SEMVER.test(bad), bad);
});

test('latest.json names a signed download from the release for every installation', () => {
  const folder = mkdtempSync(path.join(tmpdir(), 'orither-manifest-'));
  for (const file of new Set(Object.values(UPDATES))) writeFileSync(path.join(folder, `${file}.sig`), `signature of ${file}\n`);
  const latest = manifest('0.2.0', folder, '- A change', new Date('2026-11-14T10:00:00Z'));
  assert.equal(latest.version, '0.2.0');
  assert.equal(latest.pub_date, '2026-11-14T10:00:00.000Z');
  assert.deepEqual(Object.keys(latest.platforms).sort(), [
    'linux-x86_64-appimage',
    'linux-x86_64-deb',
    'linux-x86_64-rpm',
    'windows-x86_64',
    'windows-x86_64-nsis',
  ]);
  assert.deepEqual(latest.platforms['linux-x86_64-appimage'], {
    url: 'https://github.com/elpideus/orither/releases/download/v0.2.0/Orither-x86_64.AppImage',
    signature: 'signature of Orither-x86_64.AppImage',
  });
});

test('latest.json is refused when a file is not signed', () => {
  const folder = mkdtempSync(path.join(tmpdir(), 'orither-manifest-'));
  assert.throws(() => manifest('0.2.0', folder, ''), /\.sig is missing/);
});

test('the release workflow publishes the files latest.json and the README name', () => {
  const workflow = readFileSync(path.join(root, '.github/workflows/release.yml'), 'utf8');
  const readme = readFileSync(path.join(root, 'README.md'), 'utf8');
  for (const file of [...new Set(Object.values(UPDATES)), 'Orither-android-arm64.apk']) {
    assert.ok(workflow.includes(`out/${file}`), `release.yml does not build ${file}`);
    assert.ok(readme.includes(`releases/latest/download/${file}`), `README.md does not link ${file}`);
  }
});

test('the updater reads latest.json from the latest release of this repository', () => {
  const config = JSON.parse(readFileSync(path.join(root, 'desktop/src-tauri/tauri.conf.json'), 'utf8'));
  assert.deepEqual(config.plugins.updater.endpoints, ['https://github.com/elpideus/orither/releases/latest/download/latest.json']);
  assert.equal(config.plugins.updater.requireSignedVersion, true);
});

test('the sign job signs with the tauri CLI version the desktop app is built with', () => {
  const workflow = readFileSync(path.join(root, '.github/workflows/release.yml'), 'utf8');
  const lock = readFileSync(path.join(root, 'desktop/pnpm-lock.yaml'), 'utf8');
  const pinned = workflow.match(/TAURI_CLI_VERSION: (\S+)/)[1];
  const locked = lock.match(/'@tauri-apps\/cli@([\d.]+)':/)[1];
  assert.equal(pinned, locked);
});
