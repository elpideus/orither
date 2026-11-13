#!/usr/bin/env node
// The body of a GitHub release, in Markdown: what changed since the previous release, then how to
// install it. The release workflow publishes it with the release, and the "What's changed" part
// also goes into latest.json, where the app's Updates settings show it.
//
//   node scripts/release-notes.mjs v0.2.0            the whole body
//   node scripts/release-notes.mjs v0.2.0 --changes  only the list of changes
//
// The changes are the subjects of the commits since the previous release (every commit up to the
// tag, for the first release), leaving out the "Release v..." commits that only set the version.
// A pre-release counts from the previous v* tag of any kind. A stable release counts from the
// previous stable one, so it also lists what its pre-releases already shipped: installed apps only
// follow stable releases and never saw those.

import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { SEMVER } from './version.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const REPO = 'https://github.com/elpideus/orither';

function git(...args) {
  return execFileSync('git', args, { cwd: root, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trimEnd();
}

function fail(message) {
  console.error(message);
  process.exit(1);
}

/** The release before `tag`: the previous v* tag for a pre-release, the previous stable one otherwise. */
function previousTag(tag) {
  const exclude = tag.includes('-') ? [] : ['--exclude', 'v*-*'];
  try {
    return git('describe', '--tags', '--abbrev=0', '--match', 'v*', ...exclude, `${tag}^`);
  } catch {
    // No earlier such tag, or the tag is on the very first commit, which has no parent.
    return null;
  }
}

const tag = process.argv[2];
if (!tag || !tag.startsWith('v') || !SEMVER.test(tag.slice(1))) {
  fail('Usage: node scripts/release-notes.mjs v<version> [--changes]');
}
const onlyChanges = process.argv.includes('--changes');

try {
  git('rev-parse', '--verify', '--quiet', `refs/tags/${tag}`);
} catch {
  fail(`There is no tag ${tag}`);
}

const previous = previousTag(tag);
const subjects = git('log', '--format=%s', previous ? `${previous}..${tag}` : tag)
  .split('\n')
  .map((subject) => subject.trim())
  .filter((subject) => subject && !subject.startsWith('Release v'));
const changes = subjects.length ? subjects.map((subject) => `- ${subject}`) : ['- No changes besides the version.'];

if (onlyChanges) {
  process.stdout.write(`${changes.join('\n')}\n`);
  process.exit(0);
}

const compare = previous ? [`**Full changelog:** ${REPO}/compare/${previous}...${tag}`, ''] : [];
// One line per paragraph: GitHub shows every line break in a release body.
const install = [
  '| | Download |',
  '| --- | --- |',
  '| **Windows** 10 or 11 | `Orither-Setup-x64.exe`. Run it; no administrator rights needed. |',
  '| **Linux**, any distribution | `Orither-x86_64.AppImage`. Make it executable and run it. |',
  '| **Debian, Ubuntu** and derivatives | `orither_amd64.deb` |',
  '| **Fedora, openSUSE** and derivatives | `orither.x86_64.rpm` |',
  '| **Android** 10 or newer, on the phone | `Orither-android-arm64.apk` |',
  '',
  'Orither on the computer updates itself: it downloads new versions in the background and installs them when it restarts.',
  'The Android app does not update itself yet: install the new APK over the old one.',
  '`SHA256SUMS` lists every file\'s checksum.',
];

process.stdout.write(["## What's changed", '', ...changes, '', ...compare, '## Install', '', ...install, ''].join('\n'));
