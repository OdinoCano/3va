#!/usr/bin/env node
// postinstall script for the `3va` npm package.
//
// Why does this postinstall exist?  3va is a native binary that must match the
// host OS and architecture.  npm has no native mechanism for shipping
// platform-specific binaries in a single package without optionalDependencies
// tricks that require separate packages per platform.  Downloading the binary
// at install time from a trusted, versioned GitHub Release is the only
// approach that keeps this package self-contained.
//
// 3va's own philosophy blocks postinstall scripts in the projects it runs —
// this script is the justified exception: it installs the runtime itself.

'use strict';

const https  = require('https');
const http   = require('http');
const fs     = require('fs');
const path   = require('path');
const crypto = require('crypto');
const os     = require('os');
const { execFileSync } = require('child_process');

const VERSION   = '2.0.0';
const REPO      = 'OdinoCano/3va';
const BASE_URL  = `https://github.com/${REPO}/releases/download/v${VERSION}`;
const BIN_DIR   = path.join(__dirname, 'bin');

// Maps [process.platform][process.arch] → { archive, sha256 }
// SHA256 values must be updated on each release via the release workflow.
const ASSETS = {
  linux: {
    x64: {
      archive: `3va-v${VERSION}-x86_64-unknown-linux-gnu.tar.gz`,
      sha256:  'ddfd46aee3b0b86d448c7fa5e94ae902b28acfb707089db17a53720e2521f27f',
    },
    arm64: {
      archive: `3va-v${VERSION}-aarch64-unknown-linux-gnu.tar.gz`,
      sha256:  '1d825a34203ed2d9d16bbdea7daa74644a5b29bb0df63602b00bb1801b968f6d',
    },
  },
  darwin: {
    x64: {
      archive: `3va-v${VERSION}-x86_64-apple-darwin.tar.gz`,
      sha256:  '8241b8615cb7802e6740c035cc081400e33d17dc812846623d092ce9c25ff3ed',
    },
    arm64: {
      archive: `3va-v${VERSION}-aarch64-apple-darwin.tar.gz`,
      sha256:  'af0f3deb5187e551fab84062d53d86d8618ab126dc2f7a47215c5addc6b82241',
    },
  },
  win32: {
    x64: {
      archive: `3va-v${VERSION}-x86_64-pc-windows-msvc.zip`,
      sha256:  '68a61a89459547f090c39a761677fe4580bb73a55ee15c8699874bda8c2bc9b9',
    },
  },
};

function fail(msg) {
  process.stderr.write(`\n[3va install] ERROR: ${msg}\n\n`);
  process.exit(1);
}

function info(msg) {
  process.stdout.write(`[3va install] ${msg}\n`);
}

function download(url, dest) {
  return new Promise((resolve, reject) => {
    const file = fs.createWriteStream(dest);
    const client = url.startsWith('https') ? https : http;

    function get(url) {
      client.get(url, (res) => {
        if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
          // Follow redirects (GitHub Releases issues a 302).
          return get(res.headers.location);
        }
        if (res.statusCode !== 200) {
          return reject(new Error(`HTTP ${res.statusCode} for ${url}`));
        }
        res.pipe(file);
        file.on('finish', () => file.close(resolve));
        file.on('error', reject);
      }).on('error', reject);
    }

    get(url);
  });
}

function sha256File(filePath) {
  const buf = fs.readFileSync(filePath);
  return crypto.createHash('sha256').update(buf).digest('hex');
}

function extractTarGz(archive, destDir) {
  // Use the system tar; available on Linux, macOS, and Windows 10+.
  execFileSync('tar', ['xzf', archive, '-C', destDir], { stdio: 'inherit' });
}

function extractZip(archive, destDir) {
  // PowerShell Expand-Archive is present on all supported Windows versions.
  execFileSync('powershell', [
    '-NoProfile', '-Command',
    `Expand-Archive -Force -Path '${archive}' -DestinationPath '${destDir}'`,
  ], { stdio: 'inherit' });
}

async function main() {
  const platform = process.platform;
  const arch     = process.arch;

  const platformAssets = ASSETS[platform];
  if (!platformAssets) {
    fail(`Unsupported platform: ${platform}. Build from source: https://github.com/${REPO}`);
  }

  const asset = platformAssets[arch];
  if (!asset) {
    fail(`Unsupported arch "${arch}" on ${platform}. Build from source: https://github.com/${REPO}`);
  }

  const { archive, sha256: expectedSha256 } = asset;
  const archiveUrl  = `${BASE_URL}/${archive}`;
  const tmpDir      = fs.mkdtempSync(path.join(os.tmpdir(), '3va-install-'));
  const archivePath = path.join(tmpDir, archive);

  info(`Downloading ${archive} …`);
  await download(archiveUrl, archivePath);

  info('Verifying SHA256 …');
  const actualSha256 = sha256File(archivePath);
  if (actualSha256 !== expectedSha256.toLowerCase()) {
    fail(
      `SHA256 mismatch for ${archive}\n` +
      `  expected: ${expectedSha256.toLowerCase()}\n` +
      `  got:      ${actualSha256}\n` +
      'The download may be corrupted or tampered with. Aborting.'
    );
  }
  info('SHA256 OK.');

  fs.mkdirSync(BIN_DIR, { recursive: true });

  info('Extracting …');
  if (archive.endsWith('.tar.gz')) {
    extractTarGz(archivePath, tmpDir);
    const binSrc = path.join(tmpDir, '3va');
    const binDst = path.join(BIN_DIR, '3va');
    fs.copyFileSync(binSrc, binDst);
    fs.chmodSync(binDst, 0o755);
  } else {
    // .zip (Windows)
    extractZip(archivePath, tmpDir);
    const binSrc = path.join(tmpDir, '3va.exe');
    const binDst = path.join(BIN_DIR, '3va.exe');
    fs.copyFileSync(binSrc, binDst);
    // Create a thin .cmd shim so `3va` works from cmd.exe and PowerShell.
    const shimPath = path.join(BIN_DIR, '3va.cmd');
    fs.writeFileSync(shimPath, `@echo off\n"%~dp03va.exe" %*\n`);
  }

  // Clean up temp directory.
  fs.rmSync(tmpDir, { recursive: true, force: true });

  info(`3va v${VERSION} installed successfully.`);
}

main().catch((err) => fail(err.message));
