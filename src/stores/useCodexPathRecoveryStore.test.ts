import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, settlePromises } from '../../tests/helpers/reactHookHarness';
import {
  isCodexPathMissingError, isCodexPathRecoveryManaged, requestCodexPathRecovery,
  useCodexPathRecoveryStore, withCodexPathRecovery,
} from './useCodexPathRecoveryStore';

const missing = new Error('APP_PATH_NOT_FOUND:codex');

test('path classification accepts only the exact Codex error, including Error wrappers', () => {
  for (const error of [missing, ' APP_PATH_NOT_FOUND:codex ', 'Error: APP_PATH_NOT_FOUND:codex',
    'Codex API Service 已激活，但客户端启动失败: APP_PATH_NOT_FOUND:codex',
    '定位 Codex 运行实例失败: APP_PATH_NOT_FOUND:codex', { message: 'APP_PATH_NOT_FOUND:codex' }]) {
    assert.equal(isCodexPathMissingError(error), true);
  }
  for (const error of [null, {}, 'APP_PATH_NOT_FOUND:claude', 'APP_PATH_NOT_FOUND:codex-other',
    'Codex launch failed', 'PermissionDenied', 'detail: APP_PATH_NOT_FOUND:codex',
    'Codex API Service 已激活，但客户端启动失败: APP_PATH_NOT_FOUND:codex extra',
    { originalReason: 'APP_PATH_NOT_FOUND:codex' }]) {
    assert.equal(isCodexPathMissingError(error), false);
  }
});

test('successful calls and unrelated failures preserve original results without a dialog', async () => {
  const value = { instanceId: 'one' };
  assert.equal(await withCodexPathRecovery(async () => value, { key: 'success' }), value);
  const error = new Error('connection failed');
  await assert.rejects(withCodexPathRecovery(async () => { throw error; }, { key: 'failed' }), (caught) => caught === error);
  assert.equal(useCodexPathRecoveryStore.getState().request, null);
  assert.equal(isCodexPathRecoveryManaged(), false);
});

test('a missing path waits for repair and preserves the original operation context and promise result', async () => {
  let calls = 0;
  const target = { accountId: 'account', instanceId: 'managed', skipFailedStep: 'network' };
  const result = withCodexPathRecovery(async () => { if (++calls === 1) throw missing; return target; }, { key: 'context' });
  let completed = false;
  void result.then(() => { completed = true; });
  assert.equal(isCodexPathRecoveryManaged(), true, 'legacy backend events are suppressed before invoke finishes');
  await settlePromises();
  const request = useCodexPathRecoveryStore.getState().request!;
  assert.equal(completed, false);
  await request.retry();
  assert.equal(await result, target);
  assert.equal(calls, 2);
  assert.equal(useCodexPathRecoveryStore.getState().request, null);
});

test('a failed retry keeps the same recovery request and supports a later explicit retry', async () => {
  let calls = 0;
  const retryError = new Error('path no longer exists');
  const result = withCodexPathRecovery(async () => {
    calls++;
    if (calls === 1) throw missing;
    if (calls === 2) throw retryError;
    return 'started';
  }, { key: 'retry-failed' });
  await settlePromises();
  const request = useCodexPathRecoveryStore.getState().request!;
  await assert.rejects(request.retry(), (error) => error === retryError);
  assert.equal(useCodexPathRecoveryStore.getState().request?.id, request.id);
  await request.retry();
  assert.equal(await result, 'started');
});

test('duplicate operations and repeated retry clicks cannot execute the native launch twice', async () => {
  const launched = deferred<string>();
  let calls = 0;
  const operation = async () => { if (++calls === 1) throw missing; return launched.promise; };
  const first = withCodexPathRecovery(operation, { key: 'duplicate' });
  const second = withCodexPathRecovery(operation, { key: 'duplicate' });
  assert.equal(first, second);
  await settlePromises();
  const request = useCodexPathRecoveryStore.getState().request!;
  const retryOne = request.retry();
  const retryTwo = request.retry();
  assert.equal(retryOne, retryTwo);
  await settlePromises();
  assert.equal(calls, 2);
  launched.resolve('ok');
  await retryOne;
  assert.equal(await first, 'ok');
  assert.equal(await second, 'ok');
});

test('a different request cannot replace the active account or silently queue a later launch', async () => {
  const first = requestCodexPathRecovery(async () => 'first', { key: 'first' });
  const rejected = assert.rejects(first, /CODEX_START_CANCELLED/);
  const request = useCodexPathRecoveryStore.getState().request!;
  let invoked = false;
  await assert.rejects(withCodexPathRecovery(async () => { invoked = true; }, { key: 'second' }), /CODEX_START_CANCELLED/);
  assert.equal(useCodexPathRecoveryStore.getState().request?.id, request.id);
  request.cancel();
  await rejected;
  await settlePromises();
  assert.equal(invoked, false);
  assert.equal(useCodexPathRecoveryStore.getState().request, null);
  assert.equal(isCodexPathRecoveryManaged(), false);
});

test('cancelled retries stay single-flight until native completion and cannot reopen a stale dialog', async () => {
  const native = deferred<string>();
  const result = requestCodexPathRecovery(() => native.promise, { key: 'cancel-during-native' });
  const rejected = assert.rejects(result, /CODEX_START_CANCELLED/);
  const request = useCodexPathRecoveryStore.getState().request!;
  const retry = request.retry();
  await settlePromises();
  request.cancel();
  await rejected;
  let duplicateStarted = false;
  await assert.rejects(withCodexPathRecovery(async () => { duplicateStarted = true; }, { key: request.key }), /CODEX_START_CANCELLED/);
  native.resolve('already dispatched');
  await retry;
  assert.equal(duplicateStarted, false);
  assert.equal(useCodexPathRecoveryStore.getState().request, null);
  assert.equal(isCodexPathRecoveryManaged(), false);
  await assert.rejects(request.retry(), /CODEX_START_CANCELLED/);
});

test('switch-only continuation resolves its real result without invoking desktop launch', async () => {
  let launches = 0;
  let switches = 0;
  const result = requestCodexPathRecovery(async () => { launches++; return 'launched'; }, {
    key: 'switch-only', withoutDesktop: async () => { switches++; return 'switched'; },
  });
  await useCodexPathRecoveryStore.getState().request!.continueWithoutDesktop!();
  assert.equal(await result, 'switched');
  assert.equal(launches, 0);
  assert.equal(switches, 1);
});

test('cancelling before the retry microtask starts prevents any native launch', async () => {
  let called = false;
  const result = requestCodexPathRecovery(async () => { called = true; }, { key: 'cancel-before-dispatch' });
  const rejected = assert.rejects(result, /CODEX_START_CANCELLED/);
  const request = useCodexPathRecoveryStore.getState().request!;
  const retry = request.retry();
  request.cancel();
  await rejected;
  await assert.rejects(retry, /CODEX_START_CANCELLED/);
  assert.equal(called, false);
  assert.equal(isCodexPathRecoveryManaged(), false);
});
