#!/usr/bin/env node
// Puts Android's adb in desktop/src-tauri/adb, for the installers to carry (see
// desktop/src-tauri/adb.<os>.conf.json and docs/releasing.md). Orither uses it to start its helper
// on a phone and to set a phone up over USB, so nobody has to install Android's tools first.
//
//   node scripts/fetch-adb.mjs windows|linux
//
// adb is built by Google from the Android Open Source Project under the Apache License 2.0; the
// archive's NOTICE.txt goes with it. The version is pinned and its checksum checked.

import { createHash } from 'node:crypto';
import { chmodSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { inflateRawSync } from 'node:zlib';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const BASE = 'https://dl.google.com/android/repository';

/** From https://dl.google.com/android/repository/repository2-3.xml, package platform-tools. */
export const PLATFORM_TOOLS = {
  windows: {
    archive: 'platform-tools_r37.0.3-win.zip',
    sha1: 'b3fde5239ad89229d5a9178375b81c9887a24172',
    files: ['adb.exe', 'AdbWinApi.dll', 'AdbWinUsbApi.dll', 'NOTICE.txt'],
  },
  linux: {
    archive: 'platform-tools_r37.0.3-linux.zip',
    sha1: 'e1579b288a7ae4e9bc28dedec6a154d78b3c04ed',
    files: ['adb', 'NOTICE.txt'],
  },
};

/** The files of a zip archive, by name, as their bytes. Enough of the format for Google's archives. */
export function unzip(zip) {
  const end = zip.lastIndexOf(Buffer.from([0x50, 0x4b, 0x05, 0x06]));
  if (end < 0) throw new Error('not a zip archive');
  const count = zip.readUInt16LE(end + 10);
  let at = zip.readUInt32LE(end + 16);
  const files = new Map();
  for (let i = 0; i < count; i++) {
    if (zip.readUInt32LE(at) !== 0x02014b50) throw new Error('broken zip directory');
    const method = zip.readUInt16LE(at + 10);
    const packed = zip.readUInt32LE(at + 20);
    const size = zip.readUInt32LE(at + 24);
    const nameLength = zip.readUInt16LE(at + 28);
    const extraLength = zip.readUInt16LE(at + 30);
    const commentLength = zip.readUInt16LE(at + 32);
    const local = zip.readUInt32LE(at + 42);
    const name = zip.toString('utf8', at + 46, at + 46 + nameLength);
    const start = local + 30 + zip.readUInt16LE(local + 26) + zip.readUInt16LE(local + 28);
    const raw = zip.subarray(start, start + packed);
    if (method === 0) files.set(name, raw);
    else if (method === 8) files.set(name, inflateRawSync(raw));
    else throw new Error(`${name}: compression method ${method} is not supported`);
    if (files.get(name).length !== size) throw new Error(`${name}: wrong size after unpacking`);
    at += 46 + nameLength + extraLength + commentLength;
  }
  return files;
}

async function main() {
  const os = process.argv[2];
  const wanted = PLATFORM_TOOLS[os];
  if (!wanted) {
    console.error('Usage: node scripts/fetch-adb.mjs windows|linux');
    process.exit(1);
  }
  const reply = await fetch(`${BASE}/${wanted.archive}`);
  if (!reply.ok) throw new Error(`${wanted.archive}: HTTP ${reply.status}`);
  const zip = Buffer.from(await reply.arrayBuffer());
  const sha1 = createHash('sha1').update(zip).digest('hex');
  if (sha1 !== wanted.sha1) throw new Error(`${wanted.archive}: checksum ${sha1}, expected ${wanted.sha1}`);
  const files = unzip(zip);
  const target = path.join(root, 'desktop/src-tauri/adb');
  rmSync(target, { recursive: true, force: true });
  mkdirSync(target, { recursive: true });
  for (const name of wanted.files) {
    const bytes = files.get(`platform-tools/${name}`);
    if (!bytes) throw new Error(`${wanted.archive} has no ${name}`);
    const file = path.join(target, name);
    writeFileSync(file, bytes);
    if (!name.includes('.')) chmodSync(file, 0o755);
  }
  console.log(`adb from ${wanted.archive} is in ${path.relative(root, target)}: ${wanted.files.join(', ')}`);
}

if (process.argv[1]?.endsWith('fetch-adb.mjs')) {
  main().catch((err) => {
    console.error(err.message);
    process.exit(1);
  });
}
