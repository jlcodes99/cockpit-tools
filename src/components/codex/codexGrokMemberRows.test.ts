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

test('Grok member quota chip uses the unified allowance even with a depleted product share', async () => {
  const { CodexGrokBuildQuotaChip } = await import('./codexGrokMemberRows');
  const account = {
    id: 'unified', quota: {
      isUnifiedBillingUser: true, weeklyLimitPercent: 99.99,
      products: [{ product: 'PRODUCT_GROK_BUILD', usagePercent: 100 }],
    },
  } as GrokAccount;
  const element = CodexGrokBuildQuotaChip({ account, t: (key, fallback) => typeof fallback === 'string' ? fallback : key });
  assert.ok(element);
  assert.match(element.props.children.props.title, /共享额度/);
  assert.deepEqual(element.props.children.props.children[1].props.children, ['共享额度', ' ', 1, '%']);
});

test('incomplete legacy product amounts do not invent a fully available quota chip', async () => {
  const { CodexGrokBuildQuotaChip } = await import('./codexGrokMemberRows');
  for (const product of [{ product: 'GrokBuild', used: 12 }, { product: 'GrokBuild', total: 100 }]) {
    const account = { id: 'legacy', quota: { products: [product] } } as GrokAccount;
    assert.equal(CodexGrokBuildQuotaChip({ account, t: (key) => key }), null);
  }
});
