import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';

function nodes(value: any): any[] {
  if (Array.isArray(value)) return value.flatMap(nodes);
  return value && typeof value === 'object' && value.props ? [value, ...nodes(value.props.children)] : [];
}

for (const variant of ['settings', 'quick'] as const) {
  test(`${variant} enhancements omit retired dictation controls and patch only retained switches`, async () => {
    const patches: Record<string, unknown>[] = [];
    const listeners = new Map<string, () => void>();
    const hook = loadHookModule(new URL('./CodexAppEnhancementSettings.tsx', import.meta.url), {
      'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
      '@tauri-apps/api/core': {
        invoke: async (command: string, args?: { updates: Record<string, unknown> }) => {
          if (command === 'get_general_config') return {
            codex_app_native_fast_enabled: false,
            codex_app_api_quota_compat_enabled: true,
            // A stale client response must not restore retired inputs or write retired settings.
            codex_app_api_dictation_enabled: true,
            codex_app_api_dictation_model: 'legacy-model',
          };
          assert.equal(command, 'patch_general_config');
          patches.push(args!.updates);
        },
      },
    }, {
      Event: class { constructor(public type: string) {} },
      window: {
        addEventListener: (name: string, listener: () => void) => listeners.set(name, listener),
        removeEventListener: (name: string) => listeners.delete(name),
        dispatchEvent: (event: { type: string }) => listeners.get(event.type)?.(),
      },
    });
    let tree = nodes(hook.render(() => hook.exports.CodexAppEnhancementSettings({ variant })));
    assert.equal(tree.filter(entry => entry.type === 'input').every(entry => entry.props.disabled), true);
    await settlePromises();
    tree = nodes(hook.flush());
    const controls = tree.filter(entry => entry.type === 'input');
    assert.deepEqual(controls.map(entry => entry.props['aria-label']), [
      'settings.general.codexEnhancements.nativeFast',
      'settings.general.codexEnhancements.quotaCompat',
    ]);
    assert.equal(controls.every(entry => entry.props.type === 'checkbox' && !entry.props.disabled), true);
    assert.deepEqual(controls.map(entry => entry.props.checked), [false, true]);
    assert.equal(tree.some(entry => String(entry.props.children).includes('dictation')), false);
    assert.equal(tree.some(entry => entry.props.children === 'settings.general.codexEnhancements.restartHint'), true);
    assert.deepEqual(patches, []);
    controls[0].props.onChange({ target: { checked: true } });
    await settlePromises();
    hook.flush();
    assert.equal(JSON.stringify(patches), JSON.stringify([{ codex_app_native_fast_enabled: true }]));
    hook.unmount();
  });
}
