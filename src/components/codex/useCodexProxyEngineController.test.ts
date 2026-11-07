import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';
import * as prerequisite from '../../utils/codexProxyEnginePrerequisite';
import { proxyEngineReadiness } from '../../utils/codexProxySetup';
import type { EngineInstallStatus } from '../../services/codexProxyEngineService';

const missing: EngineInstallStatus = { supported: true, version: '1.19.31', installedVersion: null,
  assetName: 'official.zip', archiveBytes: 100, jobId: null, phase: 'idle', receivedBytes: 0, totalBytes: 100, error: null };
const active = (status: EngineInstallStatus | null) => !!status && ['downloading', 'importing', 'verifying', 'extracting', 'checking', 'installing'].includes(status.phase);
function harness(initial = missing) {
  let snapshot = initial;
  let readsFail = false;
  const installs: ReturnType<typeof deferred<EngineInstallStatus>>[] = [];
  const timers = new Map<number, () => void>();
  let timerId = 0;
  const h = loadHookModule(new URL('./useCodexProxyEngineController.ts', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '@tauri-apps/plugin-dialog': { open: async () => null },
    '../../services/codexProxyEngineService': {
      engineInstallActive: active, engineInstallErrorKey: () => 'installFailed',
      getCodexProxyEngineStatus: async () => { if (readsFail) throw new Error('unavailable'); return snapshot; },
      installCodexProxyEngine: () => { const task = deferred<EngineInstallStatus>(); installs.push(task); return task.promise; },
      cancelCodexProxyEngineInstall: async () => {},
    },
    '../../utils/codexProxySetup': { proxyEngineReadiness },
    '../../utils/codexProxyEnginePrerequisite': prerequisite,
  }, {
    setTimeout: (callback: () => void) => { timers.set(++timerId, callback); return timerId; },
    clearTimeout: (id: number) => timers.delete(id),
  });
  h.render(() => h.exports.useCodexProxyEngineController());
  return { ...h, installs, setStatus: (next: EngineInstallStatus) => { snapshot = next; },
    failReads: () => { readsFail = true; },
    tick: () => { const entry = timers.entries().next().value; assert.ok(entry); timers.delete(entry[0]); entry[1](); },
  };
}

test('a concurrent prerequisite failure during installation is cleared only by that installation completing', async () => {
  const h = harness(); await settlePromises();
  const installing = { ...missing, jobId: 'install-1', phase: 'downloading' as const };
  const pending = h.flush().run('install');
  h.setStatus(installing); h.installs[0].resolve(installing); await pending; h.flush(); await settlePromises();
  h.flush().reportPreflightFailure('PROXY_ENGINE_MISSING');
  assert.equal(h.flush().readiness, 'installing');
  h.setStatus({ ...installing, phase: 'completed', installedVersion: missing.version });
  h.tick(); await settlePromises();
  assert.equal(h.flush().readiness, 'ready');
  assert.equal(h.flush().error, '');
  h.unmount();
});

test('an old completed job cannot erase a newly observed corrupt-engine failure', async () => {
  const h = harness({ ...missing, phase: 'completed', jobId: 'old-job', installedVersion: missing.version });
  await settlePromises();
  h.flush().reportPreflightFailure('ENGINE_INSTALL_VERIFY'); h.flush().refresh(); h.flush(); await settlePromises();
  assert.equal(h.flush().readiness, 'failed');
  assert.equal(h.flush().error, 'codex.proxy.engine.repairHint');
  h.flush().clearPreflightFailure();
  assert.equal(h.flush().readiness, 'ready');
  h.unmount();
});

test('shared installation action lock prevents two entry points from starting duplicate jobs', async () => {
  const h = harness(); await settlePromises();
  const first = h.flush().run('install');
  await h.flush().run('install');
  assert.equal(h.installs.length, 1);
  const next = { ...missing, jobId: 'one-job', phase: 'downloading' as const };
  h.setStatus(next); h.installs[0].resolve(next); await first; h.flush(); await settlePromises();
  assert.equal(h.flush().active, true);
  h.unmount();
});

test('cancelling the file picker does not mark a corrupt engine as ready', async () => {
  const h = harness({ ...missing, phase: 'completed', jobId: 'old-job', installedVersion: missing.version });
  await settlePromises();
  h.flush().reportPreflightFailure('ENGINE_INSTALL_VERIFY');
  await h.flush().run('import'); h.flush(); await settlePromises();
  assert.equal(h.installs.length, 0);
  assert.equal(h.flush().readiness, 'failed');
  h.unmount();
});

test('ending an installation clears a busy-only prerequisite without hiding a failed repair', async () => {
  const downloading = { ...missing, phase: 'downloading' as const, jobId: 'active-job', installedVersion: missing.version };
  const h = harness(downloading); await settlePromises();
  h.flush().reportPreflightFailure('ENGINE_INSTALL_BUSY');
  h.setStatus({ ...downloading, phase: 'cancelled' }); h.tick(); await settlePromises();
  assert.equal(h.flush().readiness, 'ready');
  assert.equal(h.flush().error, '');
  h.unmount();
});

test('unreadable status stays unknown without launching or installing any process', async () => {
  const h = harness(); await settlePromises();
  h.failReads(); h.flush().refresh(); h.flush(); await settlePromises();
  assert.equal(h.flush().readiness, 'unknown');
  assert.equal(h.installs.length, 0);
  h.unmount();
});
