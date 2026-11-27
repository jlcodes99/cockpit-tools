const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { join } = require('node:path');
const { test } = require('node:test');
const { runInNewContext } = require('node:vm');

const script = readFileSync(join(__dirname, 'antigravity_launch_preferences.js'), 'utf8');
const defaults = { layout: 'software_engineering', permission: 'auto' };
const settle = () => new Promise(resolve => setImmediate(resolve));

function desktop(overrides = {}, write = Object.assign) {
  const window = {};
  window.top = window;
  const stored = { project4kMode: 2, remotePermissionPreset: 3 };
  const writes = [];
  const settings = {
    getConfiguredMagicWorkspaceVcs() {},
    getPrefValue: key => stored[key],
    async setPrefValues(update) {
      writes.push({ ...update });
      await write(stored, update);
    },
  };
  const settingsProvider = { memoizedProps: { value: { getState: () => settings } } };
  const policy = {
    fetchStatus: 'loaded',
    sandboxSupported: true,
    outsideWorkspaceFileAccessPolicy: 3,
    isSettingDisabled: () => false,
    isSettingOptionAllowed: () => true,
    getEffectiveSettingValue: (_key, value) => value,
    ...overrides,
  };
  const policyProvider = { memoizedProps: { value: policy }, return: settingsProvider };
  const composer = { '__reactFiber$fixture': { return: policyProvider } };
  const apply = runInNewContext(script, {
    window, location: { protocol: 'https:', hostname: '127.0.0.1' },
    document: { querySelector: () => composer },
  });
  return { apply, stored, writes, policy };
}

test('applies Software Engineering and native Auto once for this launch', async () => {
  const { apply, stored, writes } = desktop();
  assert.equal(apply(defaults), 'applying');
  assert.equal(apply(defaults), 'applying');
  await settle();
  assert.equal(apply(defaults), 'applied');
  assert.deepEqual(writes, [{
    project4kMode: 1,
    enableTerminalSandbox: true,
    cascadeAutoExecutionPolicy: 2,
    nonWorkspaceFileAccessPolicy: 2,
    permissionPreset: 6,
  }]);
  assert.equal(stored.remotePermissionPreset, 3);
  stored.project4kMode = 2;
  assert.equal(apply(defaults), 'applied');
  assert.equal(writes.length, 1);
});

test('preserves native policy locks and waits for readiness', async () => {
  const { apply, writes, policy } = desktop({
    fetchStatus: 'loading',
    isSettingOptionAllowed: (_setting, value) => value !== 'turbo',
  });
  const preferences = { ...defaults, permission: 'turbo' };
  assert.equal(apply(preferences), 'waiting');
  assert.equal(writes.length, 0);
  policy.fetchStatus = 'loaded';
  apply(preferences);
  await settle();
  assert.equal(apply(preferences), 'permission_restricted');
  assert.deepEqual(writes, [{ project4kMode: 1 }]);
});

test('retains the official file policy and verifies the saved values', async () => {
  const allowed = desktop({ outsideWorkspaceFileAccessPolicy: 2 });
  const preferences = { layout: 'knowledge_work', permission: 'turbo' };
  allowed.apply(preferences);
  await settle();
  assert.equal(allowed.apply(preferences), 'applied');
  assert.equal(allowed.stored.permissionPreset, 3);
  assert.equal(allowed.stored.nonWorkspaceFileAccessPolicy, 2);
  assert.equal(allowed.stored.project4kMode, 2);

  const failed = desktop({}, () => {});
  failed.apply(defaults);
  await settle();
  assert.equal(failed.apply(defaults), 'failed');
});
