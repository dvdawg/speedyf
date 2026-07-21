#!/usr/bin/env node
/**
 * Downloads the prebuilt PDFium dynamic library for the current platform from
 * https://github.com/bblanchon/pdfium-binaries into src-tauri/pdfium/.
 *
 * Pinned to a specific upstream tag for reproducibility; override with
 *   PDFIUM_TAG="chromium/XXXX" node scripts/fetch-pdfium.mjs
 */
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import process from 'node:process';

const TAG = process.env.PDFIUM_TAG ?? 'chromium/7961';

const ASSETS = {
  'darwin-arm64': ['pdfium-mac-arm64.tgz', 'libpdfium.dylib'],
  'darwin-x64': ['pdfium-mac-x64.tgz', 'libpdfium.dylib'],
  'linux-x64': ['pdfium-linux-x64.tgz', 'libpdfium.so'],
  'linux-arm64': ['pdfium-linux-arm64.tgz', 'libpdfium.so'],
  'win32-x64': ['pdfium-win-x64.tgz', 'pdfium.dll'],
  'win32-arm64': ['pdfium-win-arm64.tgz', 'pdfium.dll'],
};

const key = `${process.platform}-${process.arch}`;
const entry = ASSETS[key];
if (!entry) {
  console.error(`No prebuilt PDFium mapping for ${key}. See https://github.com/bblanchon/pdfium-binaries`);
  process.exit(1);
}
const [asset, libName] = entry;

const destDir = join(import.meta.dirname, '..', 'src-tauri', 'pdfium');
const libPath = join(destDir, libName);
const versionPath = join(destDir, 'VERSION');

if (existsSync(libPath) && existsSync(versionPath) && readFileSync(versionPath, 'utf8').trim() === TAG) {
  console.info(`PDFium ${TAG} already present at ${libPath}`);
  process.exit(0);
}

mkdirSync(destDir, { recursive: true });
const url = `https://github.com/bblanchon/pdfium-binaries/releases/download/${encodeURIComponent(TAG)}/${asset}`;
const archive = join(tmpdir(), asset);

console.info(`Downloading ${url}`);
execFileSync('curl', ['-fL', '--retry', '3', '-o', archive, url], { stdio: 'inherit' });

const extractDir = join(tmpdir(), `pdfium-extract-${Date.now()}`);
mkdirSync(extractDir, { recursive: true });
// bsdtar ships with macOS, Linux distros, and Windows 10+.
execFileSync('tar', ['-xzf', archive, '-C', extractDir], { stdio: 'inherit' });

// Archives contain lib/<libname> (and bin/pdfium.dll on Windows).
const candidates = [
  join(extractDir, 'lib', libName),
  join(extractDir, 'bin', libName),
  join(extractDir, libName),
];
const found = candidates.find((p) => existsSync(p));
if (!found) {
  console.error(`Could not find ${libName} in extracted archive (${extractDir})`);
  process.exit(1);
}

rmSync(libPath, { force: true });
execFileSync(process.platform === 'win32' ? 'cmd' : 'cp',
  process.platform === 'win32' ? ['/c', 'copy', '/y', found, libPath] : [found, libPath],
  { stdio: 'inherit' });
writeFileSync(versionPath, `${TAG}\n`);
rmSync(archive, { force: true });
rmSync(extractDir, { recursive: true, force: true });
console.info(`PDFium ${TAG} installed at ${libPath}`);
