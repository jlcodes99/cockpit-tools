import assert from 'node:assert/strict';
import test from 'node:test';
import { renderToStaticMarkup } from 'react-dom/server';
import { loadHookModule, deferred, settlePromises } from './helpers/reactHookHarness';
import type { AntigravityCliStatus } from '../src/services/antigravityCliService';
import { resolveAntigravityCliAlert, resolveAntigravityCliState } from '../src/hooks/useAntigravityCliStatus';
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

test('only abnormal CLI states raise an alert, keyed so each problem is shown once', () => {
  assert.equal(resolveAntigravityCliAlert(null, null), null);
  assert.equal(resolveAntigravityCliAlert(status, null), null);
  assert.equal(resolveAntigravityCliAlert({ ...status, current_account_id: null }, null), null);
  assert.equal(resolveAntigravityCliAlert({ ...status, credential_present: false, current_account_id: null }, null), null);
  assert.deepEqual(resolveAntigravityCliAlert({ ...status, api_key_mode: true }, null), { key: 'apiKey', kind: 'apiKey', detail: null });
  assert.deepEqual(
    resolveAntigravityCliAlert({ ...status, credential_error: 'Keyring locked' }, null),
    { key: 'keyringError:Keyring locked', kind: 'keyringError', detail: 'Keyring locked' },
  );
  assert.deepEqual(
    resolveAntigravityCliAlert(null, 'Configuration unreadable'),
    { key: 'error:Configuration unreadable', kind: 'error', detail: 'Configuration unreadable' },
  );
});

const t = (key: string) => key;

test('CLI flow notice uses the standard desc / permission / network structure', () => {
  const view = loadHookModule(new URL('../src/components/antigravity-cli/AntigravityCliFlowNotice.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t }) },
  }, { localStorage: { getItem: () => null, setItem() {} } });
  const html = renderToStaticMarkup(view.render(() => view.exports.AntigravityCliFlowNotice()));
  view.unmount();
  assert.ok(html.includes('ghcp-flow-notice'));
  for (const key of ['title', 'desc', 'permission', 'network']) {
    assert.ok(html.includes(`antigravityCli.flowNotice.${key}`), key);
  }
});

function renderLaunchModal(executable: string | null, detecting = false) {
  const view = loadHookModule(new URL('../src/components/antigravity-cli/AntigravityCliLaunchModal.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t }) },
    '@tauri-apps/plugin-dialog': { open: async () => null },
    '@tauri-apps/plugin-opener': { openUrl: async () => {} },
    '../../services/antigravityCliService': { launchAntigravityCli: async () => {} },
    '../../hooks/useEscClose': { useEscCloseTopmost() {} },
    '../../hooks/useModalFocusTrap': { useModalFocusTrap() {} },
  }, { localStorage: { getItem: () => null, setItem() {}, removeItem() {} } });
  const html = renderToStaticMarkup(view.render(() => view.exports.AntigravityCliLaunchModal({
    executable, detecting, onClose() {},
  })));
  view.unmount();
  return html;
}

test('CLI launch dialog launches when installed and guides installation otherwise', () => {
  const installed = renderLaunchModal(status.executable);
  assert.ok(installed.includes('antigravityCli.directory'));
  assert.ok(installed.includes('antigravityCli.launch<'));
  assert.ok(!installed.includes('antigravityCli.install<'));

  const missing = renderLaunchModal(null);
  assert.ok(missing.includes('antigravityCli.installHint'));
  assert.ok(missing.includes('antigravityCli.install<'));
  assert.ok(!missing.includes('antigravityCli.directory'));

  const detecting = renderLaunchModal(null, true);
  assert.ok(!detecting.includes('antigravityCli.install<'));
  assert.match(detecting, /<button[^>]*disabled=""[^>]*>.*antigravityCli\.launch</);
});
