import assert from 'node:assert/strict';
import test from 'node:test';
import { renderToStaticMarkup } from 'react-dom/server';
import { loadHookModule, deferred, settlePromises } from './helpers/reactHookHarness';
import type { AntigravityCliStatus } from '../src/services/antigravityCliService';
import type { AntigravityCliStatusController } from '../src/hooks/useAntigravityCliStatus';
import { resolveAntigravityCliState } from '../src/hooks/useAntigravityCliStatus';
import { PLATFORM_PAGE_MAP } from '../src/types/platform';
import { resolvePlatformIdFromPage } from '../src/utils/accountSyncEvents';

const status: AntigravityCliStatus = {
  executable: '/home/test/.local/bin/agy', version: '1.2.16', version_error: null,
  config_dir: '/home/test/.gemini/antigravity-cli', api_key_mode: false,
  credential_present: true, credential_error: null, current_account_id: 'cli',
};

test('Antigravity CLI is routed as its own sub-platform page', () => {
  assert.equal(PLATFORM_PAGE_MAP.antigravity_cli, 'antigravity-cli');
  assert.equal(resolvePlatformIdFromPage('antigravity-cli'), 'antigravity_cli');
  assert.equal(resolvePlatformIdFromPage('overview'), 'antigravity');
});

test('CLI state follows the keyring credential, never treating read failures as logout', () => {
  assert.equal(resolveAntigravityCliState(null, null), 'loading');
  assert.equal(resolveAntigravityCliState(null, 'Configuration unreadable'), 'error');
  assert.equal(resolveAntigravityCliState(status, null), 'managed');
  assert.equal(resolveAntigravityCliState({ ...status, api_key_mode: true }, null), 'apiKey');
  assert.equal(resolveAntigravityCliState({ ...status, current_account_id: null }, null), 'notImported');
  assert.equal(resolveAntigravityCliState({ ...status, credential_present: false, current_account_id: null }, null), 'signedOut');
  assert.equal(resolveAntigravityCliState({ ...status, credential_error: 'Keyring locked' }, null), 'keyringError');
});

test('failed status lookup surfaces the error instead of a signed-out state', async () => {
  const pending = deferred<AntigravityCliStatus>();
  const view = loadHookModule(new URL('../src/hooks/useAntigravityCliStatus.ts', import.meta.url), {
    '../services/antigravityCliService': { getAntigravityCliStatus: () => pending.promise },
  }, { window: { addEventListener() {}, removeEventListener() {} } });
  const first = view.render(() => view.exports.useAntigravityCliStatus(true, null));
  assert.equal(first.state, 'loading');
  pending.reject(new Error('Configuration unreadable'));
  await settlePromises();
  const next = view.flush();
  assert.equal(next.state, 'error');
  assert.match(next.error, /Configuration unreadable/);
  assert.equal(next.loading, false);
  view.unmount();
});

function renderPanel(cli: AntigravityCliStatusController) {
  const view = loadHookModule(new URL('../src/components/antigravity-cli/AntigravityCliOverviewPanel.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '@tauri-apps/plugin-opener': { openUrl: async () => {} },
    './AntigravityCliLaunchModal': { AntigravityCliLaunchModal: () => null },
  }, { localStorage: { getItem: () => null, setItem() {} } });
  const html = renderToStaticMarkup(view.render(() => view.exports.AntigravityCliOverviewPanel({
    cli, disabled: false, onRefresh() {}, onImport() {},
  })));
  view.unmount();
  return html;
}

const controller = (value: AntigravityCliStatus | null, error: string | null = null): AntigravityCliStatusController => ({
  status: value, error, loading: false, refresh: async () => {},
  state: resolveAntigravityCliState(value, error),
});

test('CLI panel shows each sign-in state with the matching action', () => {
  const managed = renderPanel(controller(status));
  assert.ok(managed.includes('antigravityCli.state.managed'));
  assert.ok(managed.includes('antigravityCli.launch'));
  assert.ok(!managed.includes('antigravityCli.importLocal'));

  const notImported = renderPanel(controller({ ...status, current_account_id: null }));
  assert.ok(notImported.includes('antigravityCli.notImported'));
  assert.ok(notImported.includes('antigravityCli.importLocal'));

  assert.ok(renderPanel(controller({ ...status, api_key_mode: true })).includes('antigravityCli.apiKeyMode'));
  assert.ok(renderPanel(controller({ ...status, credential_error: 'Keyring locked' })).includes('Keyring locked'));

  const missing = renderPanel(controller({ ...status, executable: null, version: null }));
  assert.ok(missing.includes('antigravityCli.install'));
  assert.ok(!missing.includes('antigravityCli.launch<'));

  const failed = renderPanel(controller(null, 'Configuration unreadable'));
  assert.ok(failed.includes('Configuration unreadable'));
  assert.ok(!failed.includes('antigravityCli.notSignedIn'));
});
