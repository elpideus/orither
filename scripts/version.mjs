#!/usr/bin/env node
// Orither's version is written in several files that must agree: the Cargo workspace and its
// crates' entries in Cargo.lock, the desktop app's package.json and Tauri config, and the Android
// app's build script. Cargo.toml is the one the others follow.
//
//   node scripts/version.mjs                  print the version
//   node scripts/version.mjs 0.2.0            set it everywhere (0.2.0-beta.1 for a pre-release)
//   node scripts/version.mjs --check [0.2.0]  fail when a file disagrees, or differs from 0.2.0

import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

/**
 * Semantic versions without build metadata, which Windows and the updater would both drop. A
 * numeric pre-release identifier has no leading zero (beta.1, never beta.01): Cargo refuses to
 * read such a version.
 */
export const SEMVER =
  /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*))*)?$/;

// Each place, with a pattern whose second group is the version. Replacing only that group leaves
// the rest of the file as it was.
const PLACES = [
  // `version = "..."` inside the `[workspace.package]` table.
  { file: 'Cargo.toml', pattern: /(\[workspace\.package\][^[]*?\nversion\s*=\s*")([^"]*)(")/ },
  // The first "version" key of a JSON file is its own.
  { file: 'desktop/package.json', pattern: /("version"\s*:\s*")([^"]*)(")/ },
  { file: 'desktop/src-tauri/tauri.conf.json', pattern: /("version"\s*:\s*")([^"]*)(")/ },
  { file: 'android/app/build.gradle.kts', pattern: /(\bversionName\s*=\s*")([^"]*)(")/ },
];
// Every crate of this workspace is named orither-*, and Cargo.lock records its version.
const LOCK_ENTRY = /(\[\[package\]\]\r?\nname = "orither-[a-z-]+"\r?\nversion = ")([^"]*)(")/g;

const read = (file) => readFileSync(path.join(root, file), 'utf8');

/** Every place the version is written, with the version found there. */
function versions() {
  const found = PLACES.map(({ file, pattern }) => ({ file, version: read(file).match(pattern)?.[2] ?? null }));
  for (const m of read('Cargo.lock').matchAll(LOCK_ENTRY)) {
    const name = m[1].match(/name = "([^"]+)"/)[1];
    found.push({ file: `Cargo.lock (${name})`, version: m[2] });
  }
  return found;
}

function replaceIn(file, pattern, version) {
  const text = read(file);
  if (!text.match(pattern)) throw new Error(`${file}: no version found`);
  writeFileSync(path.join(root, file), text.replace(pattern, (_, before, _old, after) => `${before}${version}${after}`));
}

function set(version) {
  for (const { file, pattern } of PLACES) replaceIn(file, pattern, version);
  replaceIn('Cargo.lock', LOCK_ENTRY, version);
}

function check(expected) {
  const found = versions();
  const want = expected ?? found[0].version;
  const wrong = found.filter((f) => f.version !== want);
  for (const f of wrong) console.error(`${f.file}: ${f.version ?? 'missing'} (expected ${want})`);
  if (wrong.length) process.exit(1);
  console.log(`Every file says ${want}.`);
}

if (import.meta.url === `file://${process.argv[1]}` || process.argv[1]?.endsWith('version.mjs')) {
  const args = process.argv.slice(2);
  if (args[0] === '--check') {
    check(args[1]?.replace(/^v/, ''));
  } else if (args[0]) {
    const version = args[0].replace(/^v/, '');
    if (!SEMVER.test(version)) {
      console.error(`${args[0]} is not a version like 1.2.3 or 1.2.3-beta.1`);
      process.exit(1);
    }
    set(version);
    console.log(`The version is now ${version} everywhere.`);
  } else {
    console.log(versions()[0].version);
  }
}
