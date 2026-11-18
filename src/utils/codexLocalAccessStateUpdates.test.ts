import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';

test('a burst of 90 service events shares one pending read and one follow-up', async () => {
  const window = new EventTarget();
  const event = 'codex-local-access-state-updated';
  let nativeUpdate: (() => void) | undefined;
  const errors: unknown[] = [];
  const { exports } = loadHookModule(new URL('./codexLocalAccessStateUpdates.ts', import.meta.url), {
    './tauriEventListener': {
      subscribeTauriEvent(name: string, handler: () => void) {
        assert.equal(name, event);
        nativeUpdate = handler;
        return () => { nativeUpdate = undefined; };
      },
    },
  }, { window, console: { error: (_message: string, error: unknown) => errors.push(error) } });

  const reads: ReturnType<typeof deferred<void>>[] = [];
  const dispose = exports.subscribeCodexLocalAccessStateUpdates(() => {
    const read = deferred<void>();
    reads.push(read);
    return read.promise;
  });
  nativeUpdate!();
  for (let index = 0; index < 90; index++) {
    if (index % 2 === 0) window.dispatchEvent(new Event(event));
    else nativeUpdate!();
  }
  assert.equal(reads.length, 1);

  const failure = new Error('snapshot unavailable');
  reads[0].reject(failure);
  await settlePromises();
  assert.equal(reads.length, 2);
  assert.deepEqual(errors, [failure]);

  nativeUpdate!();
  dispose();
  reads[1].resolve();
  window.dispatchEvent(new Event(event));
  await settlePromises();
  assert.equal(reads.length, 2);
  assert.equal(nativeUpdate, undefined);
});
