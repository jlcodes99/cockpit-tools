import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';

function harness() {
  const calls: Array<{ args: any; pending: ReturnType<typeof deferred<any>> }> = [];
  const listeners = new Set<() => void>();
  const timers = new Map<number, () => void>();
  let next = 0;
  const document = { visibilityState: 'visible',
    addEventListener(_event: string, callback: () => void) { listeners.add(callback); },
    removeEventListener(_event: string, callback: () => void) { listeners.delete(callback); },
  };
  const h = loadHookModule(new URL('./codexRemoteKeyData.ts', import.meta.url), {
    '@tauri-apps/api/core': { invoke: (_command: string, args: any) => {
      const pending = deferred<any>(); calls.push({ args, pending }); return pending.promise;
    } },
  }, { document,
    setTimeout(callback: () => void) { const id = ++next; timers.set(id, callback); return id; },
    clearTimeout(id: number) { timers.delete(id); },
  });
  return { h, calls, document, timers, listeners,
    visibility(value: string) { document.visibilityState = value; for (const callback of listeners) callback(); },
  };
}

test('remote detail queries ignore stale key results and stop after unmount', async () => {
  const c = harness();
  let key = 'key-one';
  const range = { startAt: 0, endAt: 3600000 };
  c.h.render(() => c.h.exports.useCodexRemoteKeyData('http://gateway/v1', key, 'stats', range, 0));
  key = 'key-two'; c.h.flush();
  assert.equal(c.calls.length, 2);
  c.calls[0].pending.resolve({ ready: true, updatedAt: 1, secret: 'old-key-data' });
  await settlePromises();
  assert.equal(c.h.flush().data, undefined);
  c.calls[1].pending.resolve({ ready: true, updatedAt: 2 });
  await settlePromises();
  assert.equal(c.h.flush().data.updatedAt, 2);
  assert.equal(c.timers.size, 1);
  c.h.unmount();
  assert.equal(c.timers.size, 0); assert.equal(c.listeners.size, 0);
});

test('hidden views pause polling and concurrent refreshes share one request', async () => {
  const c = harness(); let revision = 0;
  const range = { startAt: 0, endAt: 3600000 };
  c.h.render(() => c.h.exports.useCodexRemoteKeyData('http://gateway/v1', 'key', 'requests', range, revision));
  revision++; c.h.flush();
  assert.equal(c.calls.length, 1);
  c.calls[0].pending.resolve({ ready: true, updatedAt: 1 });
  await settlePromises(); c.h.flush();
  c.visibility('hidden'); assert.equal(c.timers.size, 0);
  c.visibility('visible'); assert.equal(c.calls.length, 2);
  c.visibility('visible'); assert.equal(c.calls.length, 2);
  c.calls[1].pending.reject(new Error('offline'));
  await settlePromises();
  assert.equal(c.h.flush().error, true);
  revision++; c.h.flush();
  assert.equal(c.h.flush().error, false);
  c.h.unmount();
  c.calls[2].pending.resolve({ ready: true, updatedAt: 3 });
  await settlePromises();
  assert.equal(c.timers.size, 0);
});

test('rebuilding statistics preserves the last complete snapshot across multiple polls', async () => {
  const c=harness();let revision=0;
  c.h.render(()=>c.h.exports.useCodexRemoteKeyData('http://gateway/v1','key','stats',{startAt:0,endAt:3600000},revision));
  c.calls[0].pending.resolve({ready:true,updatedAt:1,summary:{requestCount:42}});
  await settlePromises();c.h.flush();
  for (let i=1;i<=2;i++) {
    revision++;c.h.flush();c.calls[i].pending.resolve({ready:false,updatedAt:i+1});
    await settlePromises();const state=c.h.flush();
    assert.equal(state.data.summary.requestCount,42);assert.equal(state.data.ready,false);
    assert.equal(state.data.previousSnapshot,true);
  }
  c.h.unmount();
});
