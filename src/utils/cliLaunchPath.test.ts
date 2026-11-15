import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';
import { INSTANCE_START_COMMANDS, LAUNCH_ONLY_COMMANDS, readMissingLaunchPathApp } from './appLaunchPath';

interface RecoveryEvent {
  app: string;
  resume: () => Promise<void>;
  cancel: () => void;
}

function captureRecovery() {
  class TestCustomEvent<T> extends Event {
    readonly detail: T;
    constructor(type: string, init: EventInit & { detail: T }) {
      super(type, init);
      this.detail = init.detail;
    }
  }
  const target = new EventTarget();
  const requests: RecoveryEvent[] = [];
  target.addEventListener('app-path-missing', (event) => {
    requests.push((event as CustomEvent<RecoveryEvent>).detail);
    event.preventDefault();
  });
  const { exports } = loadHookModule(new URL('./cliLaunchPath.ts', import.meta.url), {
    '@tauri-apps/api/core': { invoke: async () => undefined },
    '../i18n': { default: { t: (key: string) => key } },
    '../services/codexWakeupService': {},
  }, { window: target, CustomEvent: TestCustomEvent });
  return {
    requests,
    withCliLaunchPath: exports.withCliLaunchPath as
      <T>(app: string, action: () => Promise<T>) => Promise<T>,
  };
}

test('saving a CLI path resumes the original operation and keeps its request until it succeeds', async () => {
  const harness = captureRecovery();
  let installed = false;
  let attempts = 0;
  const result = harness.withCliLaunchPath('claude_cli', async () => {
    attempts++;
    if (!installed) throw 'CLI_PATH_NOT_FOUND:claude_cli';
    return { instanceId: 'named-profile', workingDir: '/my/project' };
  });
  await settlePromises();
  const request = harness.requests[0];
  assert.equal(request.app, 'claude_cli');
  assert.equal(attempts, 1);
  await assert.rejects(request.resume(), (error) => error === 'CLI_PATH_NOT_FOUND:claude_cli');
  installed = true;
  await request.resume();
  assert.deepEqual(await result, { instanceId: 'named-profile', workingDir: '/my/project' });
  assert.equal(attempts, 3);
});

test('canceling path recovery leaves the launch untouched and unrelated errors do not enter recovery', async () => {
  const harness = captureRecovery();
  const launch = harness.withCliLaunchPath('codex_cli', async () => {
    throw 'CLI_PATH_NOT_FOUND:codex_cli';
  });
  await settlePromises();
  const rejected = assert.rejects(launch, /operationCancelled/);
  harness.requests[0].cancel();
  await rejected;
  await assert.rejects(
    harness.withCliLaunchPath('codex_cli', async () => { throw new Error('auth expired'); }),
    /auth expired/,
  );
  assert.equal(harness.requests.length, 1);
});

test('desktop recovery recognizes real missing paths and has distinct launch-only targets', () => {
  assert.equal(readMissingLaunchPathApp(new Error('APP_PATH_MISSING_AFTER_SWITCH:workbuddy_ai')), 'workbuddy_ai');
  assert.equal(readMissingLaunchPathApp('APP_PATH_NOT_FOUND:minimaxcode'), 'minimaxcode');
  assert.equal(readMissingLaunchPathApp('APP_PATH_NOT_FOUND:zed unrelated failure'), undefined);
  assert.equal(INSTANCE_START_COMMANDS.zcode, 'zcode_start_instance');
  assert.equal(LAUNCH_ONLY_COMMANDS.zcode, 'zcode_launch_current');
});
