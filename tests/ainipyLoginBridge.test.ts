import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';

const script = readFileSync(new URL('../src-tauri/src/commands/ainipy_login.js', import.meta.url), 'utf8')
  .replace('__AINIPY_CALLBACK_PATH__', '/__cockpit_ainipy_test');

function browser(origin: string, childFrame = false) {
  let token: string | null = null;
  let tick: (() => void) | undefined;
  let stopped = false;
  const window: { top?: unknown } = {};
  window.top = childFrame ? {} : window;
  const location = { origin, href: '' };
  runInNewContext(script, { window, location,
    sessionStorage: { getItem: (key: string) => { assert.equal(key, 'ainipy.admin_credential'); return token; } },
    setInterval: (callback: () => void) => { tick = callback; return 1; },
    clearInterval: () => { stopped = true; },
  });
  return { location, hasTimer: () => Boolean(tick), stopped: () => stopped,
    setToken: (value: string) => { token = value; }, tick: () => tick?.() };
}

test('login bridge waits for a session and encodes it in the intercepted fragment', () => {
  const page = browser('https://www.ainipy.com');
  page.tick();
  assert.equal(page.location.href, '');
  page.setToken('session+with&reserved=characters');
  page.tick();
  const callback = new URL(page.location.href);
  assert.equal(callback.pathname, '/__cockpit_ainipy_test');
  assert.equal(callback.search, '');
  assert.equal(new URLSearchParams(callback.hash.slice(1)).get('token'), 'session+with&reserved=characters');
  assert.equal(page.stopped(), true);
});

test('login bridge does not read sessions on other origins or child frames', () => {
  for (const origin of ['https://www.ainipy.com.evil.test', 'http://www.ainipy.com', 'https://example.com']) {
    assert.equal(browser(origin).hasTimer(), false);
  }
  assert.equal(browser('https://www.ainipy.com', true).hasTimer(), false);
});
