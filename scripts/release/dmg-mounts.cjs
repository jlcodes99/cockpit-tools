// Own only mounts created by this invocation. Never force-eject a user's volume.
const { spawnSync } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');

function canonical(value) {
  const absolute = path.resolve(value);
  try { return fs.realpathSync(absolute); } catch {
    const parent = path.dirname(absolute);
    return parent === absolute ? absolute : path.join(canonical(parent), path.basename(absolute));
  }
}

function createDmgMounts({ spawn = spawnSync, pause = (ms) => {
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);
} } = {}) {
  const owned = new Set();
  const collectors = [];

  function command(executable, args, options = {}) {
    const result = spawn(executable, args, {
      encoding: 'utf8', stdio: 'pipe', timeout: 60_000, ...options,
    });
    if (result.error || result.status !== 0) {
      throw new Error(`${executable} ${args[0]} failed: ${result.error?.message || result.stderr || result.status}`);
    }
    return result.stdout;
  }

  function images() {
    const plist = command('hdiutil', ['info', '-plist']);
    return JSON.parse(command('plutil', ['-convert', 'json', '-o', '-', '-'], { input: plist })).images || [];
  }

  function mountedImage(record) {
    return images().find((image) => canonical(image['image-path']) === record.imagePath);
  }

  function detach(record) {
    if (!owned.has(record)) return;
    let lastError;
    for (let attempt = 0; attempt < 3; attempt += 1) {
      const image = mountedImage(record);
      if (!image) {
        owned.delete(record);
        return;
      }
      // The first device is the entire image, including any APFS container.
      const target = image['system-entities'].find((entity) => entity['dev-entry'])?.['dev-entry'];
      if (!target) throw new Error(`Cannot identify device at ${record.mountPoint}`);
      try {
        command('hdiutil', ['detach', target]);
        if (mountedImage(record)) throw new Error(`Image remains mounted at ${record.mountPoint}`);
        owned.delete(record);
        return;
      } catch (error) {
        lastError = error;
        if (attempt < 2) pause(250 * (attempt + 1));
      }
    }
    throw new Error(`Unable to unmount ${record.mountPoint}; temporary files retained. ${lastError.message}`);
  }

  function attach(mountPoint, imagePath, { readOnly = false } = {}) {
    const record = { mountPoint: canonical(mountPoint), imagePath: canonical(imagePath) };
    const existing = images();
    // Refuse hdiutil's reuse of an already attached image. This also lets us
    // recover a partial attach that created a device but no filesystem mount.
    if (existing.some((image) => canonical(image['image-path']) === record.imagePath)) {
      throw new Error(`Image is already attached: ${record.imagePath}`);
    }
    if (existing.some((image) => image['system-entities']?.some((e) => e['mount-point'] && canonical(e['mount-point']) === record.mountPoint))) {
      throw new Error(`Mount point is already in use: ${record.mountPoint}`);
    }
    // Register before attaching: even a failed command can leave a mounted image.
    owned.add(record);
    try {
      command('hdiutil', ['attach', '-nobrowse', '-noautoopen', '-mountpoint', record.mountPoint,
        readOnly ? '-readonly' : '-readwrite', record.imagePath]);
      const image = mountedImage(record);
      const device = image?.['system-entities']?.find((e) => e['mount-point'] && canonical(e['mount-point']) === record.mountPoint)?.['dev-entry'];
      if (!device) throw new Error(`Unable to resolve mounted device for ${record.imagePath}`);
      return { ...record, device, record };
    } catch (error) {
      try { detach(record); } catch (cleanupError) {
        throw new AggregateError([error, cleanupError], 'Mount failed and cleanup failed; temporary files retained');
      }
      throw error;
    }
  }

  function cleanup() {
    const errors = [];
    for (const collect of collectors) {
      try { collect(); } catch (error) { errors.push(error); }
    }
    for (const record of [...owned].reverse()) {
      try { detach(record); } catch (error) { errors.push(error); }
    }
    if (errors.length) throw new AggregateError(errors, errors.map((e) => e.message).join('\n'));
  }

  function installExitHandlers() {
    const onExit = () => {
      try { cleanup(); } catch (error) {
        console.error(error);
        process.exitCode = 1;
      }
    };
    const onInt = () => process.exit(130);
    const onTerm = () => process.exit(143);
    process.once('exit', onExit);
    process.once('SIGINT', onInt);
    process.once('SIGTERM', onTerm);
    return () => {
      process.off('exit', onExit);
      process.off('SIGINT', onInt);
      process.off('SIGTERM', onTerm);
    };
  }

  function assertImageUnmounted(imagePath) {
    if (images().some((image) => canonical(image['image-path']) === canonical(imagePath))) {
      throw new Error(`Image is still attached; retaining ${imagePath}`);
    }
  }

  function watchBuildImages(targetDir) {
    const previousImages = new Set(images().flatMap((image) => (image['system-entities'] || [])
      .filter((entity) => entity['dev-entry'])
      .map((entity) => `${canonical(image['image-path'])}\0${entity['dev-entry']}`)));
    const target = canonical(targetDir);
    collectors.push(() => {
      for (const image of images()) {
        const imagePath = canonical(image['image-path']);
        const relative = path.relative(target, imagePath).split(path.sep).join('/');
        // Tauri/create-dmg's private writable scratch image. Final DMGs and
        // mounts existing before the build never become ours.
        if (!/^(?:[^/]+\/)?(?:release|debug)\/bundle\/macos\/rw\.[^/]+\.dmg$/.test(relative)) continue;
        const entities = image['system-entities'] || [];
        const device = entities.find((entity) => entity['dev-entry'])?.['dev-entry'];
        if (!device || previousImages.has(`${imagePath}\0${device}`)) continue;
        if (![...owned].some((record) => record.imagePath === imagePath)) {
          owned.add({ imagePath, mountPoint: entities.find((e) => e['mount-point'])?.['mount-point'] || device });
        }
      }
    });
  }

  return { attach, detach: (mounted) => detach(mounted.record), cleanup, installExitHandlers, assertImageUnmounted, watchBuildImages };
}

module.exports = { createDmgMounts };
