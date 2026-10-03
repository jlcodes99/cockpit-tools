import assert from 'node:assert/strict';
import test from 'node:test';
import { renderToStaticMarkup } from 'react-dom/server';
import { loadHookModule, deferred, settlePromises } from './helpers/reactHookHarness';
import type { AntigravityCliStatus } from '../src/services/antigravityCliService';

const status: AntigravityCliStatus = {
  executable: '/home/test/.local/bin/agy', version: '1.2.16', version_error: null,
  config_dir: '/home/test/.gemini/antigravity-cli', api_key_mode: false,
  credential_present: true, credential_error: null, current_account_id: 'cli',
};

function panel(getStatus: () => Promise<AntigravityCliStatus>) {
  return loadHookModule(new URL('../src/components/AntigravityCliPanel.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../services/antigravityCliService': { getAntigravityCliStatus: getStatus },
    '../stores/useAccountStore': { useAccountStore: () => undefined },
  }, { window: { addEventListener() {}, removeEventListener() {} } });
}

test('CLI panel displays credential, API key and keyring failure states', async () => {
  for (const [value, label] of [
    [status, 'antigravityCli.managed'],
    [{ ...status, api_key_mode: true }, 'antigravityCli.apiKeyMode'],
    [{ ...status, current_account_id: null }, 'antigravityCli.notImported'],
    [{ ...status, credential_present: false, current_account_id: null }, 'antigravityCli.notSignedIn'],
    [{ ...status, credential_error: 'Keyring locked' }, 'Keyring locked'],
  ] as const) {
    const view = panel(async () => value);
    view.render(() => view.exports.AntigravityCliPanel({ active: true, desktopLabel: 'IDE', disabled: false, onChange() {} }));
    await settlePromises();
    const html = renderToStaticMarkup(view.flush());
    assert.ok(html.includes(label), label);
    assert.ok(html.includes('agy 1.2.16'));
    view.unmount();
  }
});

test('failed status lookup is shown as an error, not a successful logout', async () => {
  const pending = deferred<AntigravityCliStatus>();
  const view = panel(() => pending.promise);
  const first = view.render(() => view.exports.AntigravityCliPanel({ active: true, desktopLabel: 'IDE', disabled: false, onChange() {} }));
  assert.ok(renderToStaticMarkup(first).includes('antigravityCli.loading'));
  pending.reject(new Error('Configuration unreadable'));
  await settlePromises();
  const html = renderToStaticMarkup(view.flush());
  assert.ok(html.includes('Configuration unreadable'));
  assert.ok(!html.includes('antigravityCli.notSignedIn'));
  view.unmount();
});
