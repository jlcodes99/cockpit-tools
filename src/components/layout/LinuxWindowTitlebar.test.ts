import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';

test('Linux title bar sends native window actions and follows window maximize state', async () => {
  let maximized = true;
  let resized: () => void = () => { throw new Error('resize listener not installed'); };
  let cleanupCount = 0;
  const actions: string[] = [];
  const window = {
    label: 'main',
    isMaximized: async () => maximized,
    minimize: async () => { actions.push('minimize'); },
    toggleMaximize: async () => {
      actions.push('toggleMaximize');
      maximized = !maximized;
      resized();
    },
    close: async () => { actions.push('close'); },
  };
  const harness = loadHookModule(new URL('./LinuxWindowTitlebar.tsx', import.meta.url), {
    '@tauri-apps/api/event': { TauriEvent: { WINDOW_RESIZED: 'tauri://resize' } },
    '@tauri-apps/api/window': { getCurrentWindow: () => window },
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../../utils/tauriEventListener': {
      subscribeTauriEvent: (name: string, handler: () => void, options: { target: { label: string } }) => {
        assert.equal(name, 'tauri://resize');
        assert.equal(options.target.label, 'main');
        resized = handler;
        return () => { cleanupCount += 1; };
      },
    },
  });

  harness.render(harness.exports.LinuxWindowTitlebar);
  await settlePromises();
  const [drag, minimize, restore, close] = harness.flush().props.children;
  assert.ok('data-tauri-drag-region' in drag.props);
  assert.equal(restore.props['aria-label'], 'common.windowRestore');
  for (const button of [minimize, restore, close]) {
    assert.equal(button.type, 'button');
    assert.ok(!('data-tauri-drag-region' in button.props));
    button.props.onClick();
  }
  assert.deepEqual(actions, ['minimize', 'toggleMaximize', 'close']);

  await settlePromises();
  const maximize = harness.flush().props.children[2];
  assert.equal(maximize.props['aria-label'], 'common.windowMaximize');
  maximize.props.onClick();
  await settlePromises();
  assert.equal(harness.flush().props.children[2].props['aria-label'], 'common.windowRestore');
  harness.unmount();
  assert.equal(cleanupCount, 1);
});
