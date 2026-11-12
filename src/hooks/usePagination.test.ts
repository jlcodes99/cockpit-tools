import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';

test('page-size hydration restores the saved value without writing the initial default', () => {
  const eventName = 'agtools:page-size-preference-changed';
  const storageKey = 'agtools.codex.accounts_page_size';
  let value: string | null = null;
  const writes: string[] = [];
  let listener: ((event: { detail: { key: string } }) => void) | undefined;
  const h = loadHookModule(new URL('./usePagination.ts', import.meta.url), {
    '../utils/durableUiSettings': {
      PAGE_SIZE_PREFERENCE_CHANGED_EVENT: eventName,
      readDurableUiSetting: () => value,
      previewDurableUiSetting: (_key: string, next: string) => { writes.push(next); },
    },
  }, {
    window: {
      addEventListener: (_event: string, callback: typeof listener) => { listener = callback; },
      removeEventListener: () => { listener = undefined; },
    },
  });
  const items = Array.from({ length: 100 }, (_, index) => index);
  h.render(() => h.exports.usePagination({ items, storageKey }));
  assert.equal(h.flush().pageSize, 20);
  assert.deepEqual(writes, []);
  value = '50';
  listener?.({ detail: { key: storageKey } });
  assert.equal(h.flush().pageSize, 50);
  assert.deepEqual(writes, []);
  h.flush().setPageSize(100);
  assert.equal(h.flush().pageSize, 100);
  assert.deepEqual(writes, ['100']);
  h.unmount();
});
