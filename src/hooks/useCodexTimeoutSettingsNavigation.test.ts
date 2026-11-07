import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';

test('settings navigation reinitializes saved values, focuses once and supports closing/reopening', () => {
  let initialized = 0, opened = 0, focused = 0, scrolled = 0;
  const h = loadHookModule(new URL('./useCodexTimeoutSettingsNavigation.ts', import.meta.url), {});
  h.render(() => h.exports.useCodexTimeoutSettingsNavigation(() => { initialized++; }, () => { opened++; }));
  const field = { focus() { focused++; }, scrollIntoView() { scrolled++; } };
  h.flush().openTimeoutSettings('sidecarStreamOpenTimeoutMs');
  let hook = h.flush(); assert.equal(hook.timeoutFocusField, 'sidecarStreamOpenTimeoutMs');
  hook.timeoutFieldRef(field); hook.timeoutFieldRef(field);
  assert.equal(focused, 1); assert.equal(scrolled, 1);
  hook.timeoutFieldRef(null); hook.openTimeoutSettings('sidecarStreamOpenTimeoutMs');
  h.flush().timeoutFieldRef(field); assert.equal(focused, 2);
  h.flush().openTimeoutSettings('sidecarImageStreamIdleTimeoutMs');
  assert.equal(h.flush().timeoutFocusField, 'sidecarImageStreamIdleTimeoutMs');
  h.flush().openTimeoutSettings(); assert.equal(h.flush().timeoutFocusField, null);
  assert.equal(initialized, 4); assert.equal(opened, 4);
});
