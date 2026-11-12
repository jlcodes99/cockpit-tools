import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';
import { PLATFORM_LAYOUT_ICON_STORAGE_KEY } from './platformLayoutIconStorage.ts';

function storageHarness(fail = false) {
  const writes: Array<[string, string]> = [];
  const { exports } = loadHookModule(new URL('./platformLayoutIconStorage.ts', import.meta.url), {
    './durableUiSettings': {
      async saveDurableUiSetting(key: string, value: string) {
        if (fail) throw new Error('write failed');
        writes.push([key, value]);
      },
    },
  }, { console: { warn() {} } });
  return { exports, writes };
}

test('custom platform icons are serialized to the durable preference key', async () => {
  const { exports, writes } = storageHarness();
  const icons = [{ id: 'one', dataUrl: 'data:image/png;base64,abc' }];
  assert.equal(await exports.persistPlatformLayoutCustomIcons(icons), true);
  assert.deepEqual(writes, [[PLATFORM_LAYOUT_ICON_STORAGE_KEY, JSON.stringify(icons)]]);
});

test('native icon persistence reports a rejected write without changing the saved value', async () => {
  const { exports, writes } = storageHarness(true);
  assert.equal(await exports.persistPlatformLayoutCustomIcons([]), false);
  assert.deepEqual(writes, []);
});
