import assert from 'node:assert/strict';
import test from 'node:test';
import type { CodexAccount } from '../../types/codex';
import type { GrokAccount } from '../../types/grok';
import { selectCodexLocalAccessMemberRows } from './codexGrokMemberRows';

const codex = { id: 'codex', auth_mode: 'apikey' } as CodexAccount;
const source = { id: 'source' } as GrokAccount;
const proxy = { id: 'proxy', auth_mode: 'apikey', upstream_grok_account_id: source.id } as CodexAccount;
const row = { ...proxy, id: 'grok:source' };
const input = {
  codexAccounts: [codex, proxy], grokRows: [row], grokAccounts: [source],
  manageGrok: true, showCodexPlatform: true, showGrokPlatform: true,
  restrictFreeAccounts: false, selected: new Set(['proxy']),
};
const ids = (rows: CodexAccount[]) => rows.map((account) => account.id);

test('Grok rows loaded after the Codex list are immediately selectable without duplicating proxies', () => {
  assert.deepEqual(ids(selectCodexLocalAccessMemberRows({ ...input, grokRows: [], grokAccounts: [] })), ['codex', 'proxy']);
  assert.deepEqual(ids(selectCodexLocalAccessMemberRows(input)), ['codex', 'grok:source']);
});

test('platform changes independently filter the latest Codex and Grok rows', () => {
  assert.deepEqual(ids(selectCodexLocalAccessMemberRows({ ...input, showCodexPlatform: false })), ['grok:source']);
  assert.deepEqual(ids(selectCodexLocalAccessMemberRows({ ...input, showGrokPlatform: false })), ['codex']);
});

test('a proxy remains removable when its Grok source has been deleted', () => {
  assert.deepEqual(ids(selectCodexLocalAccessMemberRows({ ...input, grokAccounts: [], grokRows: [] })), ['codex', 'proxy']);
});

test('filtering a source row does not bring back its hidden proxy', () => {
  assert.deepEqual(ids(selectCodexLocalAccessMemberRows({ ...input, grokRows: [] })), ['codex']);
});
