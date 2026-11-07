#!/usr/bin/env node
// Package an already-built app. No compilation, signing or version mutation.
const { spawnSync } = require('node:child_process');
const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');
const { createDmgMounts } = require('./dmg-mounts.cjs');
const { parseArgs } = require('./inject-dmg-readme.cjs');

function command(name, args) {
  const result = spawnSync(name, args, { stdio: 'inherit', timeout: 120_000 });
  if (result.error || result.status !== 0) throw new Error(`${name} failed: ${result.error?.message || result.status}`);
}

function digest(file) {
  const hash = crypto.createHash('sha256');
  const fd = fs.openSync(file, 'r');
  const buffer = Buffer.alloc(1024 * 1024);
  try {
    let size;
    while ((size = fs.readSync(fd, buffer)) > 0) hash.update(buffer.subarray(0, size));
    return hash.digest('hex');
  } finally { fs.closeSync(fd); }
}

function verifyTree(source, target) {
  for (const entry of fs.readdirSync(source, { withFileTypes: true })) {
    const from = path.join(source, entry.name);
    const to = path.join(target, entry.name);
    const stat = fs.lstatSync(to);
    if (entry.isSymbolicLink()) {
      if (!stat.isSymbolicLink() || fs.readlinkSync(from) !== fs.readlinkSync(to)) throw new Error(`Link mismatch: ${to}`);
    } else if (entry.isDirectory()) {
      if (!stat.isDirectory()) throw new Error(`Directory mismatch: ${to}`);
      verifyTree(from, to);
    } else if (!stat.isFile() || digest(from) !== digest(to)) {
      throw new Error(`File mismatch: ${to}`);
    }
  }
}

function main() {
  if (process.platform !== 'darwin') throw new Error('Local DMG packaging requires macOS');
  const args = parseArgs(process.argv.slice(2));
  if ((!args.app && !args.stage) || (args.app && args.stage) || !args.output) {
    throw new Error('Usage: node scripts/release/package-local-dmg.cjs (--app <built.app> | --stage <prepared-directory>) --output <file.dmg>');
  }
  const output = path.resolve(args.output);
  if (!output.endsWith('.dmg')) throw new Error('Output must end in .dmg');
  fs.mkdirSync(path.dirname(output), { recursive: true });
  const mounts = createDmgMounts();
  mounts.installExitHandlers();
  mounts.assertImageUnmounted(output);
  // Build beside the destination so the final rename stays on the same volume.
  const work = fs.mkdtempSync(path.join(path.dirname(output), '.dmg-build-'));
  const payload = path.join(work, 'payload');
  const image = path.join(work, 'package.dmg');
  const mountPoint = path.join(work, 'verify');
  try {
    if (args.stage) {
      command('ditto', [path.resolve(args.stage), payload]);
    } else {
      const app = path.resolve(args.app);
      if (!app.endsWith('.app')) throw new Error('Expected a built .app bundle');
      fs.mkdirSync(payload);
      command('ditto', [app, path.join(payload, path.basename(app))]);
      fs.symlinkSync('/Applications', path.join(payload, 'Applications'));
      fs.copyFileSync(path.join(__dirname, 'assets/dmg-readme.txt'), path.join(payload, '安装与常见问题.txt'));
    }
    const apps = fs.readdirSync(payload).filter((name) => name.endsWith('.app'));
    if (apps.length !== 1) throw new Error('Expected exactly one app bundle');
    command('codesign', ['--verify', '--deep', '--strict', path.join(payload, apps[0])]);
    command('hdiutil', ['create', '-volname', 'Cockpit Tools', '-srcfolder', payload, '-fs', 'HFS+', '-format', 'UDZO', image]);
    command('hdiutil', ['verify', image]);
    fs.mkdirSync(mountPoint);
    const mounted = mounts.attach(mountPoint, image, { readOnly: true });
    try {
      command('codesign', ['--verify', '--deep', '--strict', path.join(mountPoint, apps[0])]);
      verifyTree(payload, mountPoint);
    } finally {
      mounts.detach(mounted);
    }
    mounts.assertImageUnmounted(output);
    fs.renameSync(image, output);
    fs.writeFileSync(`${output}.sha256`, `${digest(output)}  ${path.basename(output)}\n`);
    console.log(`Verified package: ${output}`);
  } finally {
    // Cleanup failures must fail packaging and retain the mounted image's files.
    mounts.cleanup();
    fs.rmSync(work, { recursive: true, force: true });
  }
}

if (require.main === module) {
  try { main(); } catch (error) { console.error(error); process.exitCode = 1; }
}

module.exports = { verifyTree };
