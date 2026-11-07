import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';

function content(node: any): string {
  if (node == null || typeof node === 'boolean') return '';
  if (typeof node === 'string' || typeof node === 'number') return String(node);
  if (Array.isArray(node)) return node.map(content).join(' ');
  return content(node.props?.children);
}

const imports = {
  'react-i18next': { useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }) },
  '../hooks/useEscClose': { useEscClose() {} },
  '../utils/updaterReleaseNotes': { prependUpdaterReleaseHighlights: (_version: string, notes: string) => notes },
  '@tauri-apps/plugin-opener': { openUrl() {} },
};

test('update dialog shows the updater-provided notes without a release API request', () => {
  const h = loadHookModule(new URL('./UpdateNotification.tsx', import.meta.url), imports);
  const view = h.render(() => h.exports.UpdateNotification({
    updateInfo: { latest_version: '1.3.76', current_version: '1.3.75', download_url: '', release_notes: '### Added\n- Local updater notes', release_notes_zh: '' },
    onClose() {},
  }));
  assert.match(content(view), /Local updater notes/);
  h.unmount();
});

test('post-update dialog opens immediately and reads only local notes, ignoring late replies for previous versions', async () => {
  const calls: { command: string; version: string; request: ReturnType<typeof deferred<any>> }[] = [];
  const h = loadHookModule(new URL('./VersionJumpNotification.tsx', import.meta.url), {
    ...imports,
    '@tauri-apps/api/core': { invoke(command: string, args: { version: string }) {
      const request = deferred<any>(); calls.push({ command, version: args.version, request }); return request.promise;
    } },
  }, { window: {}, performance: { now: () => 0 }, console: { log() {}, error() {} } });
  let version = '1.3.75';
  let view = h.render(() => h.exports.VersionJumpNotification({ info: { previous_version: '1.3.74', current_version: version }, onClose() {} }));
  assert.match(content(view), /1.3.75/);
  assert.deepEqual(calls.map((call) => call.command), ['get_bundled_release_notes', 'get_pending_release_notes']);
  version = '1.3.76'; h.flush();
  calls[2].request.resolve({ release_notes: '### Added\n- Current local notes', release_notes_zh: '' });
  await settlePromises(); h.flush();
  calls[0].request.resolve({ release_notes: '### Added\n- Old notes', release_notes_zh: '' });
  calls[1].request.resolve({ release_notes: '### Added\n- Old pending notes', release_notes_zh: '' });
  await settlePromises(); view = h.flush();
  assert.match(content(view), /Current local notes/);
  assert.doesNotMatch(content(view), /Old notes|Old pending notes/);
  calls[3].request.resolve({ release_notes: '### Added\n- Pending local notes', release_notes_zh: '' });
  await settlePromises(); view = h.flush();
  assert.match(content(view), /Pending local notes/);
  assert.equal(calls.length, 4);
  h.unmount();
});

test('saving updater notes does not delay the caller and uses the local save command', () => {
  const calls: { command: string; args: any }[] = [];
  const h = loadHookModule(new URL('../services/localUpdateNotes.ts', import.meta.url), {
    '@tauri-apps/api/core': { invoke(command: string, args: any) {
      calls.push({ command, args }); return new Promise(() => {});
    } },
    '../utils/updaterReleaseNotes': { parseUpdaterReleaseNotes: () => ({ releaseNotes: 'English', releaseNotesZh: '中文' }) },
  });
  h.exports.savePendingUpdateNotes('1.3.76', 'body');
  assert.equal(calls[0].command, 'save_pending_update_notes');
  assert.equal(calls[0].args.version, '1.3.76');
  assert.equal(calls[0].args.releaseNotesZh, '中文');
});


test('a late bundled read cannot replace installation-time notes', async () => {
  const calls: { command: string; request: ReturnType<typeof deferred<any>> }[] = [];
  const h = loadHookModule(new URL('./VersionJumpNotification.tsx', import.meta.url), {
    ...imports,
    '@tauri-apps/api/core': { invoke(command: string) {
      const request = deferred<any>(); calls.push({ command, request }); return request.promise;
    } },
  }, { window: {}, performance: { now: () => 0 }, console: { log() {}, error() {} } });
  h.render(() => h.exports.VersionJumpNotification({ info: { previous_version: '1.3.75', current_version: '1.3.76' }, onClose() {} }));
  calls[1].request.resolve({ release_notes: '### Added\n- Installed notes', release_notes_zh: '' });
  await settlePromises(); h.flush();
  calls[0].request.resolve({ release_notes: '### Added\n- Bundled notes', release_notes_zh: '' });
  await settlePromises();
  assert.match(content(h.flush()), /Installed notes/);
  assert.doesNotMatch(content(h.flush()), /Bundled notes/);
  h.unmount();
});
