#!/usr/bin/env node
/**
 * Rebuilds the macOS DMG produced by `tauri build` so that it also ships a
 * plain-text note (install steps plus the "app is damaged" / Gatekeeper fix)
 * and gives that note a stable position in the DMG window layout: the app icon
 * and the Applications link stay paired on the top row so the drag-and-drop
 * affordance is unchanged, and the note sits on a second row below them.
 *
 * Tauri does not expose a bundler hook for extra DMG files, so the image is
 * rebuilt from the app bundle inside the original DMG: same volume icon, same
 * Applications drop link, same window geometry, plus the note.
 */

const { spawnSync } = require('node:child_process');
const crypto = require('node:crypto');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const REPO_ROOT = path.resolve(__dirname, '..', '..');
const README_SOURCE = path.join(__dirname, 'assets', 'dmg-readme.txt');

const WINDOW_SIZE = { width: 660, height: 490 };
const WINDOW_POSITION = { x: 10, y: 60 };
const ICON_SIZE = 112;
const TEXT_SIZE = 14;
const APP_ICON_POSITION = { x: 180, y: 150 };
const APPLICATIONS_ICON_POSITION = { x: 480, y: 150 };
const README_ICON_POSITION = { x: 330, y: 330 };
// Extra room for Finder metadata plus HFS+ block rounding of the app bundle.
const IMAGE_SIZE_MARGIN_MB = 40;

function fail(message) {
  console.error(`[inject-dmg-readme] ${message}`);
  process.exit(1);
}

function log(message) {
  console.log(`[inject-dmg-readme] ${message}`);
}

function parseArgs(argv) {
  const args = {};
  for (let i = 0; i < argv.length; i += 1) {
    const token = argv[i];
    if (!token.startsWith('--')) continue;
    const key = token.slice(2);
    const value = argv[i + 1];
    if (!value || value.startsWith('--')) {
      args[key] = 'true';
      continue;
    }
    args[key] = value;
    i += 1;
  }
  return args;
}

function readTauriProductName() {
  const configPath = path.join(REPO_ROOT, 'src-tauri', 'tauri.conf.json');
  try {
    const config = JSON.parse(fs.readFileSync(configPath, 'utf8'));
    return typeof config.productName === 'string' ? config.productName : '';
  } catch {
    return '';
  }
}

function readAppVersion() {
  try {
    return require(path.join(REPO_ROOT, 'package.json')).version || '';
  } catch {
    return '';
  }
}

function run(command, args, options = {}) {
  const { capture, ...spawnOptions } = options;
  const result = spawnSync(command, args, {
    ...spawnOptions,
    stdio: capture ? 'pipe' : 'inherit',
    encoding: 'utf8',
  });
  if (result.error) {
    throw new Error(`${command} failed: ${result.error.message}`);
  }
  if (result.status !== 0) {
    const stderr = (result.stderr || '').trim();
    const detail = stderr ? `: ${stderr.split('\n').slice(-3).join(' ')}` : '';
    throw new Error(`${command} exited with code ${result.status}${detail}`);
  }
  return result.stdout || '';
}

function sleep(ms) {
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);
}

function findTauriDmg(bundleDir) {
  const dmgDir = path.join(bundleDir, 'dmg');
  if (!fs.existsSync(dmgDir)) {
    return null;
  }
  const entries = fs.readdirSync(dmgDir);

  // Clean up leftovers from an interrupted previous run.
  for (const name of entries.filter((entry) => entry.endsWith('.inject.dmg'))) {
    log(`removing stale temporary image: ${name}`);
    fs.rmSync(path.join(dmgDir, name), { force: true });
  }

  const candidates = entries.filter(
    (name) => name.toLowerCase().endsWith('.dmg') && !name.endsWith('.inject.dmg')
  );
  if (candidates.length === 0) {
    return null;
  }
  if (candidates.length > 1) {
    throw new Error(
      `expected one DMG in ${dmgDir}, found: ${candidates.join(', ')}`
    );
  }
  return path.join(dmgDir, candidates[0]);
}

function attachDiskImage(mountPoint, imagePath, { readOnly = false } = {}) {
  const args = ['attach', '-nobrowse', '-mountpoint', mountPoint];
  if (readOnly) {
    args.push('-readonly');
  } else {
    args.push('-readwrite', '-noverify', '-noautoopen');
  }
  args.push(imagePath);

  const output = run('hdiutil', args, { capture: true });

  const device = output
    .split('\n')
    .map((line) => line.trim().split(/\s+/)[0])
    .find((token) => /^\/dev\/disk\d+s\d+$/.test(token));
  if (!device) {
    throw new Error(`unable to resolve the mounted device for ${imagePath}`);
  }
  return { device, mountPoint };
}

function detachMountedVolume(target) {
  for (const attempt of [0, 1, 2]) {
    const result = spawnSync('hdiutil', ['detach', target], {
      stdio: 'pipe',
      encoding: 'utf8',
    });
    if (result.status === 0) {
      return true;
    }
    sleep(1000 * (attempt + 1));
  }

  const force = spawnSync('hdiutil', ['detach', target, '-force'], {
    stdio: 'pipe',
    encoding: 'utf8',
  });
  if (force.status !== 0) {
    log(`warning: unable to unmount ${target}; unmount it manually before retrying`);
  }
  return force.status === 0;
}

function copyAppBundle(sourceAppPath, destinationAppPath) {
  // ditto keeps the bundle structure, symlinks and resource forks intact.
  run('ditto', ['--norsrc', '--noextattr', '--noqtn', sourceAppPath, destinationAppPath]);
}

function measureMegabytes(targetPath) {
  const output = run('du', ['-sk', targetPath], { capture: true });
  const kilobytes = Number.parseInt(output.trim().split(/\s+/)[0], 10);
  if (!Number.isFinite(kilobytes)) {
    throw new Error(`unable to measure ${targetPath}`);
  }
  return Math.ceil(kilobytes / 1024);
}

function buildPayload({ workDir, appName, appBundlePath, readmeName }) {
  const payloadDir = path.join(workDir, 'payload');
  fs.mkdirSync(payloadDir, { recursive: true });

  copyAppBundle(appBundlePath, path.join(payloadDir, appName));
  fs.copyFileSync(README_SOURCE, path.join(payloadDir, readmeName));

  // Same convenience symlink the Tauri bundler creates.
  fs.symlinkSync('/Applications', path.join(payloadDir, 'Applications'));

  return payloadDir;
}

function buildFinderScript({ volumeName, appName, readmeName }) {
  const lines = [
    'on run',
    '\ttell application "Finder"',
    `\t\tset theDisk to disk "${volumeName}"`,
    '\t\ttell theDisk',
    '\t\t\topen',
    '\t\t\tdelay 1',
    `\t\t\tset theXOrigin to ${WINDOW_POSITION.x}`,
    `\t\t\tset theYOrigin to ${WINDOW_POSITION.y}`,
    `\t\t\tset theWidth to ${WINDOW_SIZE.width}`,
    `\t\t\tset theHeight to ${WINDOW_SIZE.height}`,
    '\t\t\tset theBottomRightX to (theXOrigin + theWidth)',
    '\t\t\tset theBottomRightY to (theYOrigin + theHeight)',
    '\t\t\ttell container window',
    '\t\t\t\tset current view to icon view',
    '\t\t\t\tset toolbar visible to false',
    '\t\t\t\tset statusbar visible to false',
    '\t\t\t\tset the bounds to {theXOrigin, theYOrigin, theBottomRightX, theBottomRightY}',
    '\t\t\tend tell',
    '\t\t\tset opts to the icon view options of container window',
    '\t\t\ttell opts',
    `\t\t\t\tset icon size to ${ICON_SIZE}`,
    `\t\t\t\tset text size to ${TEXT_SIZE}`,
    '\t\t\t\tset arrangement to not arranged',
    '\t\t\tend tell',
    `\t\t\tset position of item "${appName}" to {${APP_ICON_POSITION.x}, ${APP_ICON_POSITION.y}}`,
    `\t\t\tset position of item "${readmeName}" to {${README_ICON_POSITION.x}, ${README_ICON_POSITION.y}}`,
    `\t\t\tset position of item "Applications" to {${APPLICATIONS_ICON_POSITION.x}, ${APPLICATIONS_ICON_POSITION.y}}`,
    '\t\t\tdelay 2',
    '\t\t\tclose',
    '\t\t\topen',
    '\t\t\tdelay 2',
    '\t\tend tell',
    '\t\tdelay 2',
    '\t\treturn "ok"',
    '\tend tell',
    'end run',
  ];
  return lines.join('\n');
}

function ensureFinderIsRunning() {
  const running = spawnSync('/usr/bin/pgrep', ['-x', 'Finder'], { stdio: 'pipe', encoding: 'utf8' });
  if (running.status !== 0) {
    log('starting Finder for the DMG layout step');
    spawnSync('/usr/bin/open', ['-a', 'Finder'], { stdio: 'pipe', encoding: 'utf8' });
  }
}

function applyFinderLayout({ volumeName, appName, readmeName, workDir }) {
  const scriptPath = path.join(workDir, 'layout.applescript');
  fs.writeFileSync(scriptPath, `${buildFinderScript({ volumeName, appName, readmeName })}\n`);

  ensureFinderIsRunning();

  // create-dmg waits before scripting Finder to dodge occasional -1728 errors;
  // the first attempt right after mounting needs a bit more time.
  sleep(5000);
  run('/usr/bin/osascript', [scriptPath]);
}

function buildDmg({
  workDir,
  appName,
  appBundlePath,
  volumeIconPath,
  readmeName,
  volumeName,
  outputPath,
}) {
  const payloadDir = buildPayload({ workDir, appName, appBundlePath, readmeName });
  const appSizeMb = measureMegabytes(path.join(payloadDir, appName));
  const tempImagePath = path.join(workDir, 'payload-rw.dmg');
  const preferredMountPoint = `/Volumes/${volumeName}`;

  run('hdiutil', [
    'create',
    '-quiet',
    '-srcfolder',
    payloadDir,
    '-volname',
    volumeName,
    '-fs',
    'HFS+',
    '-fsargs',
    '-c c=64,a=16,e=16',
    '-format',
    'UDRW',
    '-size',
    `${appSizeMb + IMAGE_SIZE_MARGIN_MB}m`,
    tempImagePath,
  ]);

  const mounted = attachDiskImage(preferredMountPoint, tempImagePath);
  const { mountPoint } = mounted;
  let layoutSignature = '';

  try {
    fs.copyFileSync(volumeIconPath, path.join(mountPoint, '.VolumeIcon.icns'));
    run('SetFile', ['-c', 'icnC', path.join(mountPoint, '.VolumeIcon.icns')]);
    run('SetFile', ['-a', 'C', mountPoint]);

    const dsStorePath = path.join(mountPoint, '.DS_Store');
    let lastError = null;
    for (let attempt = 0; attempt < 3; attempt += 1) {
      try {
        applyFinderLayout({ volumeName, appName, readmeName, workDir });
        layoutSignature = waitForFinderLayout(dsStorePath, 4);
        lastError = null;
        break;
      } catch (error) {
        lastError = error;
        log(`Finder layout attempt ${attempt + 1} failed, retrying`);
      }
    }
    if (lastError) {
      throw lastError;
    }

    fs.rmSync(path.join(mountPoint, '.fseventsd'), { recursive: true, force: true });
    run('chmod', ['-Rf', 'go-w', mountPoint]);
  } finally {
    detachMountedVolume(mounted.mountPoint) || detachMountedVolume(mounted.device);
  }

  run('hdiutil', ['convert', tempImagePath, '-format', 'UDZO', '-o', outputPath]);
  return layoutSignature;
}

function describeFinderLayout(dsStorePath) {
  const data = fs.readFileSync(dsStorePath);
  const markers = (data.toString('latin1').match(/Iloc/g) || []).length;
  if (markers < 3) {
    throw new Error(
      `Finder layout is missing icon positions (found ${markers} entries) in ${dsStorePath}`
    );
  }
  return `${data.length}:${markers}:${crypto.createHash('sha1').update(data).digest('hex')}`;
}

function waitForFinderLayout(dsStorePath, attempts = 12) {
  let lastError = null;
  for (let attempt = 0; attempt < attempts; attempt += 1) {
    try {
      return describeFinderLayout(dsStorePath);
    } catch (error) {
      lastError = error;
      sleep(1000);
    }
  }
  throw lastError;
}

function verifyDmg({ dmgPath, readmeName, workDir, expectedLayout }) {
  const mountPoint = path.join(workDir, 'verify-mount');
  fs.mkdirSync(mountPoint, { recursive: true });

  const attached = attachDiskImage(mountPoint, dmgPath, { readOnly: true });

  try {
    const readmePath = path.join(mountPoint, readmeName);
    if (!fs.existsSync(readmePath)) {
      throw new Error(`DMG does not contain ${readmeName}`);
    }
    const content = fs.readFileSync(readmePath, 'utf8');
    if (!content.includes('com.apple.quarantine')) {
      throw new Error('DMG troubleshooting note has unexpected content');
    }
    const dsStorePath = path.join(mountPoint, '.DS_Store');
    if (!fs.existsSync(dsStorePath)) {
      throw new Error('DMG is missing the Finder layout file (.DS_Store)');
    }
    if (expectedLayout && describeFinderLayout(dsStorePath) !== expectedLayout) {
      throw new Error('Finder layout changed while compressing the DMG');
    }
    if (!fs.existsSync(path.join(mountPoint, '.VolumeIcon.icns'))) {
      log('warning: rebuilt DMG has no volume icon');
    }
    const volumeInfo = run('diskutil', ['info', '-plist', attached.device], { capture: true });
    const nameMatch = volumeInfo.match(/<key>VolumeName<\/key>\s*<string>([^<]*)<\/string>/);
    const volumeName = nameMatch ? nameMatch[1] : '';
    if (!volumeName) {
      throw new Error('rebuilt DMG has no volume name');
    }
  } finally {
    detachMountedVolume(attached.mountPoint) || detachMountedVolume(attached.device);
  }
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  const keepWorkDir = args['keep-work-dir'] === 'true';
  const bundleDir = path.resolve(
    args['bundle-dir'] || path.join(REPO_ROOT, 'target', 'release', 'bundle')
  );

  if (process.platform !== 'darwin') {
    log('skipped: DMG bundling is macOS only');
    return;
  }

  const sourceDmg = findTauriDmg(bundleDir);
  if (!sourceDmg) {
    log('skipped: no DMG found in the bundle output directory');
    return;
  }

  if (!fs.existsSync(README_SOURCE)) {
    fail(`missing troubleshooting note template: ${README_SOURCE}`);
  }

  const productName = readTauriProductName() || 'Cockpit Tools';
  const version = readAppVersion();
  const volumeIconPath = path.join(path.dirname(sourceDmg), 'icon.icns');
  if (!fs.existsSync(volumeIconPath)) {
    fail(`missing volume icon: ${volumeIconPath}`);
  }

  const readmeName = '安装与常见问题.txt';
  const volumeName = version ? `${productName} ${version}` : productName;
  // Keep Tauri's artifact file name: release staging and the Homebrew cask
  // job both derive the published asset name from it.
  const outputPath = sourceDmg;
  const tempOutputPath = path.join(path.dirname(sourceDmg), `${volumeName}.inject.dmg`);
  const workDir = fs.mkdtempSync(path.join(os.tmpdir(), 'cockpit-dmg-readme-'));
  const injectMountPoint = path.join(workDir, 'inject-mount');
  fs.mkdirSync(injectMountPoint, { recursive: true });
  let sourceMount = null;

  log(`source DMG: ${sourceDmg}`);
  log(`target DMG: ${outputPath}`);

  try {
    sourceMount = attachDiskImage(injectMountPoint, sourceDmg, { readOnly: true });

    let appName;
    let appBundlePath;
    let layoutSignature = '';
    try {
      const appBundles = fs
        .readdirSync(injectMountPoint)
        .filter((name) => name.endsWith('.app'));
      if (appBundles.length !== 1) {
        throw new Error(
          `expected one .app in ${injectMountPoint}, found: ${appBundles.join(', ') || 'none'}`
        );
      }
      appName = appBundles[0];
      appBundlePath = path.join(injectMountPoint, appName);

      fs.rmSync(tempOutputPath, { force: true });
      layoutSignature = buildDmg({
        workDir,
        appName,
        appBundlePath,
        volumeIconPath,
        readmeName,
        volumeName,
        outputPath: tempOutputPath,
      });
    } finally {
      if (sourceMount) {
        detachMountedVolume(sourceMount.mountPoint) || detachMountedVolume(sourceMount.device);
      }
    }

    verifyDmg({
      dmgPath: tempOutputPath,
      readmeName,
      workDir,
      expectedLayout: layoutSignature,
    });

    // Only replace the Tauri artifact once the rebuilt image passed verification.
    fs.renameSync(tempOutputPath, outputPath);
    log(`done: ${outputPath}`);
  } finally {
    fs.rmSync(tempOutputPath, { force: true });
    if (keepWorkDir) {
      log(`keeping work directory: ${workDir}`);
    } else {
      fs.rmSync(workDir, { recursive: true, force: true });
    }
  }
}

if (require.main === module) {
  try {
    main();
  } catch (error) {
    fail(error instanceof Error ? error.message : String(error));
  }
}

module.exports = {
  buildFinderScript,
  describeFinderLayout,
  findTauriDmg,
  parseArgs,
};
