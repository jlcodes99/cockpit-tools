import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';
import type { CodexApiKeyInspectionRequest } from '../utils/codexApiKeyInspection';

function harness(pending: CodexApiKeyInspectionRequest | null = null) {
  let listener: ((request: CodexApiKeyInspectionRequest) => void) | null = null;
  let subscribed = 0;
  let cleaned = 0;
  let apiKeys: readonly { id: string }[] | null = null;
  let activeTab = 'overview';
  let expanded = new Set(['existing-draft']);
  const scrolls: unknown[] = [];
  const focuses: unknown[] = [];
  const lookups: string[] = [];
  const cards = new Map<string, { focus: (options: unknown) => void }>();
  const hook = loadHookModule(new URL('./useCodexApiKeyInspection.ts', import.meta.url), {
    '../utils/codexApiKeyInspection': {
      subscribeCodexApiKeyInspectionRequests(callback: typeof listener) {
        subscribed += 1;
        listener = callback;
        if (pending) { const request = pending; pending = null; callback!(request); }
        return () => { cleaned += 1; listener = null; };
      },
    },
    '../utils/reducedMotion': {
      scrollElementIntoView(card: unknown, options: unknown) { scrolls.push({ card, options }); },
    },
  }, {
    document: { getElementById(id: string) { lookups.push(id); return cards.get(id) ?? null; } },
  });
  const setActiveTab = (next: string) => { activeTab = next; };
  const setExpandedApiKeyPolicyIds = (next: (ids: Set<string>) => Set<string>) => { expanded = next(expanded); };
  hook.render(() => hook.exports.useCodexApiKeyInspection({
    apiKeys, activeTab, setActiveTab, expandedApiKeyPolicyIds: expanded, setExpandedApiKeyPolicyIds,
  }));
  return {
    hook, lookups, scrolls, focuses,
    receive(request: CodexApiKeyInspectionRequest) { listener!(request); return this.flush(); },
    load(keys: readonly { id: string }[]) { apiKeys = keys; return this.flush(); },
    flush() { hook.flush(); return hook.flush(); },
    addCard(id: string) { cards.set(hook.exports.codexApiKeyInspectionCardId(id), { focus(options) { focuses.push({ id, options }); } }); },
    get tab() { return activeTab; }, get expanded() { return expanded; },
    get subscribed() { return subscribed; }, get cleaned() { return cleaned; },
  };
}

test('mounted inspection expands and focuses only the requested key and preserves existing expansions', () => {
  const h = harness();
  const keys = Object.freeze([Object.freeze({ id: 'first' }), Object.freeze({ id: 'target' })]);
  h.load(keys); h.addCard('target');
  const state = h.receive({ apiKeyId: 'target', requestKind: 'text' });
  assert.equal(h.tab, 'keys');
  assert.deepEqual([...h.expanded], ['existing-draft', 'target']);
  assert.equal(state.inspectionNoticeKey, null);
  assert.equal(h.focuses.length, 1); assert.equal(h.scrolls.length, 1);
  h.load([...keys]);
  assert.equal(h.focuses.length, 1, 'fulfilled requests cannot replay on data refresh');
  h.hook.unmount(); assert.equal(h.subscribed, 1); assert.equal(h.cleaned, 1);
});

test('newly mounted inspection consumes pending context and waits for asynchronous collection data', () => {
  const h = harness({ apiKeyId: 'later', requestKind: 'text' });
  assert.equal(h.tab, 'overview'); assert.equal(h.lookups.length, 0);
  h.addCard('later'); h.load([{ id: 'later' }]);
  assert.equal(h.tab, 'keys'); assert.equal(h.focuses.length, 1);
});

test('missing card can be located after rendering without a timer or losing the request', () => {
  const h = harness(); h.load([{ id: 'target' }]);
  h.receive({ apiKeyId: 'target', requestKind: 'text' });
  assert.equal(h.focuses.length, 0);
  h.addCard('target'); h.load([{ id: 'target' }]);
  assert.equal(h.focuses.length, 1);
});

test('deleted and unknown keys explain the missing key without expanding an unrelated key', () => {
  const h = harness(); h.load([{ id: 'first' }]); h.addCard('first');
  const state = h.receive({ apiKeyId: 'deleted', requestKind: 'text' });
  assert.equal(h.tab, 'keys');
  assert.equal(state.inspectionNoticeKey, 'codex.apiService.inspection.keyUnavailable');
  assert.deepEqual([...h.expanded], ['existing-draft']); assert.equal(h.focuses.length, 0);
});

test('internal, legacy and instance gateway keys use account scope when absent from public keys', () => {
  for (const apiKeyId of ['__cockpit_internal__', 'legacy', 'provider_gateway_account', 'mixed_model_routing']) {
    const h = harness(); h.load([{ id: 'first' }, { id: '__cockpit_internal__' }]);
    const state = h.receive({ apiKeyId, requestKind: 'text' });
    assert.equal(h.tab, 'accounts');
    assert.equal(state.inspectionNoticeKey, 'codex.apiService.inspection.internalScope');
    assert.deepEqual([...h.expanded], ['existing-draft']); assert.equal(h.focuses.length, 0);
  }
});

test('image requests point to image account scope even if a public key matches', () => {
  for (const requestKind of ['image_generation', 'image_edit']) {
    const h = harness(); h.load([{ id: 'public' }]); h.addCard('public');
    const state = h.receive({ apiKeyId: 'public', requestKind });
    assert.equal(h.tab, 'accounts');
    assert.equal(state.inspectionNoticeKey, 'codex.apiService.inspection.imageScope');
    assert.equal(h.focuses.length, 0); assert.equal(h.expanded.has('public'), false);
  }
});

test('latest pending request wins and a matching special-character ID is safely looked up', () => {
  const h = harness();
  h.receive({ apiKeyId: 'stale', requestKind: 'text' });
  const id = 'key:# []"\\/中文';
  h.receive({ apiKeyId: id, requestKind: 'text' });
  h.addCard(id); h.load([{ id: 'stale' }, { id }]);
  assert.equal(h.expanded.has('stale'), false);
  assert.deepEqual(h.lookups, [`codex-api-service-key:${id}`]);
  assert.equal(h.focuses.length, 1);
  const missing = h.receive({ apiKeyId: 'missing', requestKind: 'text' });
  assert.equal(missing.inspectionNoticeKey, 'codex.apiService.inspection.keyUnavailable');
  missing.dismissInspectionNotice(); assert.equal(h.flush().inspectionNoticeKey, null);
  h.receive({ apiKeyId: id, requestKind: 'text' });
  assert.equal(h.flush().inspectionNoticeKey, null);
});
