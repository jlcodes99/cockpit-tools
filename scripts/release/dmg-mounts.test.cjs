const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const { once } = require('node:events');
const test = require('node:test');
const { createDmgMounts } = require('./dmg-mounts.cjs');

function fakeTools() {
  const state = { images: [], calls: [], busy: false, partialFailure: false };
  state.spawn = (name, args, options) => {
    state.calls.push([name, ...args]);
    if (name === 'plutil') return { status: 0, stdout: options.input };
    if (args[0] === 'info') return { status: 0, stdout: JSON.stringify({ images: state.images }) };
    if (args[0] === 'attach') {
      state.images.push({ 'image-path': args.at(-1), 'system-entities': [
        { 'dev-entry': '/dev/disk900' },
        { 'dev-entry': '/dev/disk900s1', 'mount-point': state.noFilesystem ? undefined : args[args.indexOf('-mountpoint') + 1] },
      ] });
      return state.partialFailure ? { status: 1, stderr: 'attach failed after mount' } : { status: 0, stdout: '' };
    }
    if (args[0] === 'detach') {
      if (state.busy) return { status: 1, stderr: 'Resource busy' };
      state.images = state.images.filter((image) => image['system-entities'][0]['dev-entry'] !== args[1]);
      return { status: 0, stdout: '' };
    }
    throw new Error(`Unexpected command: ${name} ${args}`);
  };
  return state;
}

function fixture() {
  const state = fakeTools();
  return { state, mounts: createDmgMounts({ spawn: state.spawn, pause() {} }) };
}

test('cleanup ejects owned image once and preserves an existing user mount', () => {
  const { state, mounts } = fixture();
  const existing = { 'image-path': '/user.dmg', 'system-entities': [
    { 'dev-entry': '/dev/disk1' }, { 'mount-point': '/user-mount' },
  ] };
  state.images.push(existing);
  const mounted = mounts.attach('/test-mount', '/test.dmg', { readOnly: true });
  mounts.detach(mounted);
  mounts.cleanup();
  assert.deepEqual(state.images, [existing]);
  assert.equal(state.calls.filter((call) => call[1] === 'detach').length, 1);
  assert.ok(state.calls.find((call) => call[1] === 'attach').includes('-noautoopen'));
  assert.ok(state.calls.find((call) => call[1] === 'attach').includes('-readonly'));
});

test('validation errors still run cleanup', () => {
  const { state, mounts } = fixture();
  assert.throws(() => {
    const mounted = mounts.attach('/test-mount', '/test.dmg');
    try { throw new Error('signature invalid'); } finally { mounts.detach(mounted); }
  }, /signature invalid/);
  assert.deepEqual(state.images, []);
});

test('a partially failed attach is unmounted before propagating the error', () => {
  const { state, mounts } = fixture();
  state.partialFailure = true;
  assert.throws(() => mounts.attach('/test-mount', '/test.dmg'), /attach failed after mount/);
  assert.deepEqual(state.images, []);
});

test('busy mounts fail cleanup without force and remain available for retry', () => {
  const { state, mounts } = fixture();
  const mounted = mounts.attach('/test-mount', '/test.dmg');
  state.busy = true;
  assert.throws(() => mounts.detach(mounted), /temporary files retained/);
  assert.throws(() => mounts.assertImageUnmounted('/test.dmg'), /still attached/);
  assert.equal(state.calls.filter((call) => call[1] === 'detach').length, 3);
  assert.ok(state.calls.every((call) => !call.includes('-force')));
  state.busy = false;
  mounts.cleanup();
  mounts.assertImageUnmounted('/test.dmg');
  assert.deepEqual(state.images, []);
});

test('an attached device without a filesystem mount is also cleaned up', () => {
  const { state, mounts } = fixture();
  state.noFilesystem = true;
  assert.throws(() => mounts.attach('/test-mount', '/test.dmg'), /Unable to resolve mounted device/);
  assert.deepEqual(state.images, []);
});

test('an image already opened by the user cannot be reused or ejected', () => {
  const { state, mounts } = fixture();
  mounts.attach('/user-mount', '/test.dmg');
  const other = createDmgMounts({ spawn: state.spawn, pause() {} });
  assert.throws(() => other.attach('/test-mount', '/test.dmg'), /already attached/);
  other.cleanup();
  assert.equal(state.images.length, 1);
  mounts.cleanup();
});

test('mountpoint collisions fail before attaching or taking ownership', () => {
  const { state, mounts } = fixture();
  mounts.attach('/test-mount', '/first.dmg');
  assert.throws(() => mounts.attach('/test-mount', '/second.dmg'), /already in use/);
  assert.equal(state.calls.filter((call) => call[1] === 'attach').length, 1);
  mounts.cleanup();
});

test('build cleanup collects only new writable scratch images in its target directory', () => {
  const { state, mounts } = fixture();
  const image = (name, device) => ({ 'image-path': name, 'system-entities': [
    { 'dev-entry': device }, { 'mount-point': `/mount-${device.split('/').at(-1)}` },
  ] });
  const old = image('/build/target/release/bundle/macos/rw.old.dmg', '/dev/disk1');
  state.images.push(old);
  mounts.watchBuildImages('/build/target');
  const final = image('/build/target/release/bundle/dmg/Cockpit Tools.dmg', '/dev/disk2');
  const unrelated = image('/other/target/release/bundle/macos/rw.other.dmg', '/dev/disk3');
  state.images.push(final, unrelated,
    image('/build/target/aarch64-apple-darwin/release/bundle/macos/rw.new.dmg', '/dev/disk4'));
  mounts.cleanup();
  assert.deepEqual(state.images, [old, final, unrelated]);
  mounts.cleanup();
  assert.equal(state.calls.filter((call) => call[1] === 'detach').length, 1);
});

test('SIGTERM triggers owned mount cleanup', { skip: process.platform === 'win32', timeout: 10_000 }, async () => {
  const script = `
    const { createDmgMounts } = require(${JSON.stringify(require.resolve('./dmg-mounts.cjs'))});
    const state = (${fakeTools.toString()})();
    const mounts = createDmgMounts({ spawn: state.spawn, pause() {} });
    mounts.installExitHandlers();
    mounts.attach('/test-mount', '/test.dmg');
    process.on('exit', () => console.log('remaining=' + state.images.length));
    setInterval(() => {}, 1000);
    console.log('ready');
  `;
  const child = spawn(process.execPath, ['-e', script], { stdio: ['ignore', 'pipe', 'pipe'] });
  let output = '';
  let signalled = false;
  const exited = once(child, 'close');
  child.stdout.on('data', (data) => {
    output += data;
    if (!signalled && output.includes('ready')) {
      signalled = true;
      child.kill('SIGTERM');
    }
  });
  try {
    const [code] = await exited;
    assert.equal(code, 143);
    assert.match(output, /remaining=0/);
  } finally {
    if (child.exitCode === null) child.kill('SIGKILL');
  }
});
