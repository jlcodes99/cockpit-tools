import assert from 'node:assert/strict';
import test from 'node:test';
import { renderToStaticMarkup } from 'react-dom/server';
import { deferred, loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';
import type { AntigravityInstalledVersionInfo } from '../services/antigravityRuntimeService';
import type { AntigravityRuntimeTarget } from '../utils/antigravityRuntimeTarget';

test('the version badge names the desktop client while scanning and ignores a stale IDE scan', async () => {
  const previous = deferred<AntigravityInstalledVersionInfo>();
  const desktop = {
    product_name: 'Antigravity',
    version: '2.32.1',
    app_path: '/opt/Antigravity',
    source: 'app.asar/package.json',
  };
  let target: AntigravityRuntimeTarget = 'antigravity_ide';
  let startScan: (() => void) | undefined;
  const calls: string[] = [];
  const hook = loadHookModule(new URL('./AntigravityInstalledVersionBadge.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }) },
    '../hooks/useAntigravityRuntimeTarget': { useAntigravityRuntimeTarget: () => target },
    '../services/antigravityRuntimeService': {
      getAntigravityInstalledVersionInfo: async (requested: string, mode: string) => {
        calls.push(`${requested}/${mode}`);
        return requested === 'antigravity_ide'
          ? previous.promise
          : mode === 'quick'
            ? desktop
            : null;
      },
    },
  }, {
    window: {
      setTimeout: (callback: () => void) => { startScan = callback; return 1; },
      clearTimeout: () => { startScan = undefined; },
    },
  });

  assert.match(renderToStaticMarkup(hook.render(() => hook.exports.AntigravityInstalledVersionBadge())), /Antigravity IDE/);
  assert.ok(startScan);
  startScan();
  target = 'antigravity';
  const loading = renderToStaticMarkup(hook.flush());
  assert.match(loading, /installed-version-name">Desktop</);
  assert.match(loading, /is-loading/);
  assert.ok(startScan);
  startScan();
  await settlePromises();

  previous.resolve({ ...desktop, product_name: 'Antigravity IDE', version: '1.0.0' });
  await settlePromises();
  const detected = renderToStaticMarkup(hook.flush());
  assert.match(detected, /installed-version-name">Desktop</);
  assert.match(detected, /v2\.32\.1/);
  assert.doesNotMatch(detected, /1\.0\.0/);
  assert.deepEqual(calls, ['antigravity_ide/quick', 'antigravity/quick', 'antigravity/full']);
  hook.unmount();
});
