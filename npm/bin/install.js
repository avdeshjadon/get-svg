#!/usr/bin/env node

const fs = require('fs');
const path = require('path');
const os = require('os');
const { execSync } = require('child_process');

const VERSION = 'v0.2.3';
const REPO = 'avdeshjadon/get-svg';

function getTarget() {
  const platform = os.platform();
  const arch = os.arch();

  if (platform === 'darwin') {
    if (arch === 'arm64') return { target: 'aarch64-apple-darwin', ext: 'tar.gz', binary: 'getsvg' };
    if (arch === 'x64') return { target: 'x86_64-apple-darwin', ext: 'tar.gz', binary: 'getsvg' };
  } else if (platform === 'linux') {
    if (arch === 'x64') return { target: 'x86_64-unknown-linux-gnu', ext: 'tar.gz', binary: 'getsvg' };
  } else if (platform === 'win32') {
    if (arch === 'x64') return { target: 'x86_64-pc-windows-msvc', ext: 'zip', binary: 'getsvg.exe' };
  }
  return null;
}

async function download(url, dest) {
  if (typeof fetch === 'function') {
    const res = await fetch(url);
    if (!res.ok) throw new Error(`HTTP ${res.status}: ${res.statusText}`);
    const buffer = Buffer.from(await res.arrayBuffer());
    fs.writeFileSync(dest, buffer);
    return;
  }

  const https = require('https');
  return new Promise((resolve, reject) => {
    function get(u) {
      https.get(u, (res) => {
        if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
          return get(res.headers.location);
        }
        if (res.statusCode !== 200) {
          return reject(new Error(`HTTP ${res.statusCode}`));
        }
        const file = fs.createWriteStream(dest);
        res.pipe(file);
        file.on('finish', () => file.close(resolve));
      }).on('error', reject);
    }
    get(url);
  });
}

async function install() {
  const info = getTarget();
  if (!info) {
    console.warn(`[getsvg] Warning: Prebuilt binary not available for ${os.platform()} ${os.arch()}.`);
    return;
  }

  const binDir = path.join(__dirname);
  const binaryPath = path.join(binDir, info.binary);

  if (fs.existsSync(binaryPath)) {
    return binaryPath;
  }

  const artifact = `get-svg-${info.target}.${info.ext}`;
  const url = `https://github.com/${REPO}/releases/download/${VERSION}/${artifact}`;
  const tempArchive = path.join(os.tmpdir(), `getsvg-${Date.now()}.${info.ext}`);

  try {
    process.stdout.write(`[getsvg] Downloading ${artifact}…\n`);
    await download(url, tempArchive);

    const member = `get-svg-${info.target}/${info.binary}`;
    if (info.ext === 'zip') {
      try {
        execSync(`tar -xf "${tempArchive}" -C "${binDir}" --strip-components=1 "${member}"`, { stdio: 'ignore' });
      } catch (_) {
        execSync(`tar -xf "${tempArchive}" -C "${binDir}" --strip-components=1`, { stdio: 'ignore' });
      }
    } else {
      try {
        execSync(`tar -xzf "${tempArchive}" -C "${binDir}" --strip-components=1 "${member}"`, { stdio: 'ignore' });
      } catch (_) {
        execSync(`tar -xzf "${tempArchive}" -C "${binDir}" --strip-components=1`, { stdio: 'ignore' });
      }
    }

    if (fs.existsSync(binaryPath)) {
      if (os.platform() !== 'win32') {
        fs.chmodSync(binaryPath, 0o755);
      }
      process.stdout.write(`[getsvg] Successfully installed to ${binaryPath}\n`);
      return binaryPath;
    }
  } catch (err) {
    console.warn(`[getsvg] Notice: Could not download prebuilt binary during postinstall (${err.message}).`);
    console.warn(`[getsvg] It will be downloaded on first run when 'getsvg' or 'npx getsvg' is executed.`);
  } finally {
    if (fs.existsSync(tempArchive)) {
      try { fs.unlinkSync(tempArchive); } catch (_) {}
    }
  }
}

if (require.main === module) {
  install().catch(() => process.exit(0));
}

module.exports = { install, getTarget, VERSION, REPO };
