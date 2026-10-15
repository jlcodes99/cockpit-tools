import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';

function harness() {
  const events: { type: string; detail: unknown }[] = [];
  const h = loadHookModule(new URL('./codexProxyEnginePrerequisite.ts', import.meta.url), {}, {
    window: { dispatchEvent: (event: { type: string; detail: unknown }) => { events.push(event); return true; } },
    CustomEvent: class { type: string; detail: unknown; constructor(type: string, options: { detail: unknown }) { this.type = type; this.detail = options.detail; } },
  });
  return { service: h.exports, events };
}

test('only exact prerequisite codes can open the installer, never connection errors or private text', () => {
  const h = harness();
  for (const value of ['PROXY_CONNECTION_REFUSED', 'PROXY_TLS_FAILED', 'PROXY_ENGINE_MISSING /private/archive', 'https://user:secret@proxy', 'toString']) {
    assert.equal(h.service.presentProxyEnginePrerequisite(value), false);
  }
  assert.equal(h.events.length, 0);
  assert.equal(h.service.presentProxyEnginePrerequisite(new Error('ENGINE_INSTALL_VERIFY')), true);
  assert.equal(h.events[0].type, 'codex-proxy-engine-required');
  assert.equal(h.events[0].detail, 'ENGINE_INSTALL_VERIFY');
});

test('a blocked action retains its original rejection and cannot proceed as a successful save', async () => {
  const h = harness();
  const error = new Error('PROXY_ENGINE_MISSING');
  let written = false;
  await assert.rejects(async () => {
    await h.service.withProxyEnginePrerequisite(Promise.reject(error));
    written = true;
  }, (caught) => caught === error);
  assert.equal(written, false);
  assert.equal(h.events.length, 1);
  assert.equal(h.events[0].detail, 'PROXY_ENGINE_MISSING');
});

test('successful direct requests pass through untouched and node connection failures do not request installation', async () => {
  const h = harness();
  const value = { ip: '127.0.0.1' };
  assert.equal(await h.service.withProxyEnginePrerequisite(Promise.resolve(value)), value);
  await assert.rejects(h.service.withProxyEnginePrerequisite(Promise.reject('PROXY_CONNECTION_REFUSED')), (error) => error === 'PROXY_CONNECTION_REFUSED');
  assert.equal(h.events.length, 0);
});
