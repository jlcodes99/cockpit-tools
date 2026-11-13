import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';

test('official reset history retains grant/redemption events and opaque cursor', async () => {
  let args: any;
  const h = loadHookModule(new URL('./codexResetHistoryService.ts', import.meta.url), {
    '@tauri-apps/api/core': { invoke: async (command: string, params: any) => {
      assert.equal(command, 'get_codex_reset_credit_history'); args = params;
      return { events: [
        { id: 'g', kind: 'granted', occurred_at: '2026-11-02T00:00:00Z' },
        { id: 'r', kind: 'redeemed', occurred_at: 'invalid' },
      ], next_cursor: 'next+/=' };
    } },
  });
  const page = await h.exports.loadCodexResetHistory('account', 'official', 'cursor+/=');
  assert.equal(args.accountId, 'account'); assert.equal(args.cursor, 'cursor+/=');
  assert.equal(page.entries[0].kind, 'granted'); assert.equal(page.entries[0].at, Date.parse('2026-11-02T00:00:00Z'));
  assert.equal(page.entries[1].kind, 'redeemed'); assert.equal(page.entries[1].at, null);
  assert.equal(page.nextCursor, 'next+/=');
});

test('local reset history keeps request identity and actual no-op outcome', async () => {
  const h = loadHookModule(new URL('./codexResetHistoryService.ts', import.meta.url), {
    '@tauri-apps/api/core': { invoke: async (command: string, params: any) => {
      assert.equal(command, 'get_codex_reset_usage_history'); assert.equal(params.cursor, 42);
      return { records: [
        { request_id: 'stable-request', at: 100, reason: 'manual', outcome: 'nothing_to_reset' },
        { request_id: 'automatic', at: 200, reason: 'quota', outcome: 'reset',
          context: { rule: { enabled: true, expiry_enabled: false, quota_enabled: true, remaining_percent: 5, before_seconds: 60 },
            before: { available_count: 1, available_card_ids: ['card-1'], hourly: null, weekly: { remaining: 4 } } },
          after_observation: { available_count: 0, available_card_ids: [], hourly: null, weekly: { remaining: 100 } },
          receipt: { credit_id: 'card-1', windows_reset: 2 } },
      ], next_cursor: null };
    } },
  });
  const page = await h.exports.loadCodexResetHistory('account', 'local', 42);
  assert.equal(page.entries[0].id, 'stable-request');
  assert.equal(page.entries[0].at, 100000);
  assert.equal(page.entries[0].kind, 'nothing_to_reset');
  assert.equal(page.entries[0].reason, 'manual');
  assert.equal(page.entries[0].context, undefined);
  assert.deepEqual(page.entries[1].context.before.available_card_ids, ['card-1']);
  assert.equal(page.entries[1].context.rule.remaining_percent, 5);
  assert.equal(page.entries[1].after.available_count, 0);
  assert.equal(page.entries[1].after.weekly.remaining, 100);
  assert.deepEqual(page.entries[1].receipt, { credit_id: 'card-1', windows_reset: 2 });
});
