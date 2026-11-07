import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';

test('refreshing callback data preserves the session; reopen and target changes initialize with latest data', () => {
  const h = loadHookModule(new URL('./useDialogSession.ts', import.meta.url), {});
  let key: string | null = 'first', data = 'original';
  const initialized: string[] = [];
  h.render(() => h.exports.useDialogSession(key, () => initialized.push(data)));
  data = 'refreshed'; h.flush(); h.flush();
  assert.deepEqual(initialized, ['original']);
  key = null; h.flush(); key = 'first'; h.flush();
  assert.deepEqual(initialized, ['original', 'refreshed']);
  key = 'second'; data = 'other target'; h.flush();
  assert.deepEqual(initialized, ['original', 'refreshed', 'other target']);
  h.unmount();
});
