const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const test = require('node:test');

const source = fs.readFileSync(path.join(__dirname, '../src-tauri/src/modules/codex_app_injection_dialog.js'), 'utf8');
const labels = JSON.parse(fs.readFileSync(path.join(__dirname, '../src/locales/en.json'), 'utf8'));
const usageLabels = { ...labels.common.codexUsage,
  requestTier: labels.codex.apiService.logs.requestedTier,
  responseTier: labels.codex.apiService.logs.responseTier,
  legacyTier: labels.codex.apiService.logs.legacyTier,
  tierHint: labels.codex.apiService.logs.serviceTierHint,
  tierFast: labels.codex.speed.fast, tierUltrafast: labels.codex.speed.ultrafast,
  tierStandard: labels.codex.speed.standard, tierAuto: labels.codex.contextOverride.automatic,
  tierUnknown: labels.common.requestProxy.unknown,
};
const thread = 'aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee';
const row = (id, extra = {}) => ({ id, timestamp: 1792545981000, accountDisplayName: 'user@example.com',
  model: 'gpt-6.1-sol', reasoningEffort: 'high', success: true, httpStatus: 200,
  inputTokens: 1000, outputTokens: 214, cachedTokens: 700, totalTokens: 1214,
  outputGenerationMs: 3240, latencyMs: 14140, firstResponseMs: 10910, ...extra });

function harness() {
  let now = 1792546000000;
  class Clock extends Date { static now() { return now; } }
  const dispatched = [];
  const window = { __cockpitUsageRequest: (value) => dispatched.push(JSON.parse(value)) };
  const context = { Date: Clock, window, usageLabels, missingLabel: '—', closeLabel: 'Close',
    plans: [], balanceLines: [], grokLines: [], accountCount: 0, availableAccountCount: 0,
    abnormalAccountCount: 0, cooldownAccountCount: 0, accountPoolLabel: 'Accounts', balanceLabel: 'Balance',
    grokEmptyLabel: 'No Grok quota', quotaOverviewLabel: 'Quota', planLabel: 'Plan', fiveHourLabel: '5h',
    weeklyLabel: 'Week', quotaEmptyLabel: 'No quota', officialLabel: 'Official', document: { activeElement: null } };
  const api = vm.runInNewContext(source, context);
  const buttons = [ ['tab','overview'], ['tab','logs'], ['scope','chat'], ['scope','service'],
    ['period','today'], ['period','week'], ['more',null], ['retry',null] ].map(([action,value]) => ({
      disabled: false, getAttribute: (key) => key === 'data-cockpit-dialog-action' ? action : key === 'data-value' ? value : null,
      focus() {}, action, value,
    }));
  const body = { scrollTop: 0 };
  const details = { innerHTML: '', contains: () => false,
    querySelector: (selector) => selector === '.cockpit-usage-body' || selector === '.request-scroll' ? body
      : buttons.find((button) => selector.includes(`"${button.action}"`) && (!button.value || selector.includes(`"${button.value}"`))) || null,
    querySelectorAll: () => buttons,
  };
  const input = { badgeKind: 'codex', averageSpeed: '29 tok/s', averageCache: '74%', speed: '66 tok/s', cache: '74%', formatMoney: () => '—', shortProductLabel: String };
  const root = { threadId: thread, quotaDetailsOpen: true, hostAvailable: true,
    usageState: { __binding: { binding: 'service-key-instance' } } };
  root.render = () => api.render(root, details, input);
  const respond = (patch = {}, query = api.query(root)) => {
    root.usageState.__dialog = { [query.requestToken]: { ...query, available: true, ...patch } };
    api.query(root);
    root.render();
  };
  const action = (name, value) => { root.render(); buttons.find((button) => button.action === name && (value == null || button.value === value)).onclick(); };
  return { api, root, details, input, dispatched, respond, action, advance: (ms) => { now += ms; } };
}

test('first query dispatches immediately once and old binding responses cannot populate a new binding', () => {
  const h = harness();
  const query = h.api.query(h.root);
  assert.equal(h.dispatched.length, 1);
  assert.equal(query.binding, 'service-key-instance');
  h.api.query(h.root);
  assert.equal(h.dispatched.length, 1);
  h.respond({ binding: 'former-key', summary: { requests: 1 } }, query);
  assert.equal(h.root.dialogState.summary, null);
  h.respond({ summary: { requests: 2 } }, query);
  assert.equal(h.root.dialogState.summary.requests, 2);
  h.root.usageState.__binding.binding = 'new-key';
  const next = h.api.query(h.root);
  assert.notEqual(next.requestToken, query.requestToken);
  assert.equal(h.root.dialogState.rows.length, 0);
  h.respond({ ...query, summary: { requests: 1 } }, next);
  assert.equal(h.root.dialogState.summary, null);
});

test('tab switches and refresh failures retain verified rows, while a successful empty query clears them', () => {
  const h = harness();
  h.action('tab', 'logs');
  h.respond({ requests: { rows: [row(1)], nextCursor: 'next' } });
  h.action('tab', 'overview');
  h.action('tab', 'logs');
  assert.equal(h.root.dialogState.rows.length, 1);
  h.respond({ error: true });
  assert.equal(h.root.dialogState.rows.length, 1);
  assert.ok(h.details.innerHTML.includes(usageLabels.dataFailed));
  h.action('retry');
  h.respond({ available: false, unavailableReason: 'service_unavailable', requests: { rows: [] } });
  assert.equal(h.root.dialogState.rows.length, 1);
  assert.ok(h.details.innerHTML.includes(usageLabels.serviceUnavailable));
  h.action('retry');
  h.respond({ requests: { rows: [], nextCursor: null } });
  assert.equal(h.root.dialogState.rows.length, 0);
  assert.ok(h.details.innerHTML.includes(usageLabels.noRequests));
});

test('scope snapshots never mix chat rows with all-service rows and restore instantly on return', () => {
  const h = harness();
  h.action('tab', 'logs');
  h.respond({ requests: { rows: [row(1)] } });
  h.action('scope', 'service');
  assert.equal(h.root.dialogState.rows.length, 0);
  h.respond({ requests: { rows: [row(2), row(3)] } });
  h.action('scope', 'chat');
  assert.deepEqual(Array.from(h.root.dialogState.rows, (item) => item.id), [1]);
  h.action('scope', 'service');
  assert.deepEqual(Array.from(h.root.dialogState.rows, (item) => item.id), [2, 3]);
});

test('new thread discards rows and late results even when service binding did not change', () => {
  const h = harness();
  h.action('tab', 'logs');
  const old = h.api.query(h.root);
  h.respond({ requests: { rows: [row(1)] } });
  h.root.threadId = 'bbbbbbbb-cccc-dddd-eeee-ffffffffffff';
  const next = h.api.query(h.root);
  assert.equal(h.root.dialogState.rows.length, 0);
  h.respond({ ...old, requests: { rows: [row(2)] } }, next);
  assert.equal(h.root.dialogState.rows.length, 0);
});

test('pagination deduplicates rows and preserves the snapshot on retryable errors', () => {
  const h = harness();
  h.action('tab', 'logs');
  h.respond({ requests: { rows: [row(1)], nextCursor: 'page-2' } });
  h.action('more');
  assert.equal(h.api.query(h.root).cursor, 'page-2');
  h.respond({ error: true });
  assert.equal(h.root.dialogState.rows.length, 1);
  h.action('retry');
  assert.equal(h.api.query(h.root).cursor, 'page-2');
  h.respond({ requests: { rows: [row(1), row(2)], nextCursor: null } });
  assert.deepEqual(Array.from(h.root.dialogState.rows, (item) => item.id), [1, 2]);
});

test('missing replies time out with retry feedback without replacing existing rows by empty data', () => {
  const h = harness();
  h.action('tab', 'logs');
  h.respond({ requests: { rows: [row(1)] } });
  h.api.refresh(h.root);
  h.advance(10001);
  h.root.render();
  assert.equal(h.root.dialogState.error, true);
  assert.equal(h.root.dialogState.rows.length, 1);
  assert.ok(h.details.innerHTML.includes(usageLabels.dataFailed));
});

test('unlinked chat and unavailable service have different messages, neither claims an empty successful query', () => {
  const h = harness();
  h.action('tab', 'logs');
  h.respond({ available: false, unavailableReason: 'unlinked_session' });
  assert.ok(h.details.innerHTML.includes(usageLabels.unlinkedSession));
  assert.ok(!h.details.innerHTML.includes(usageLabels.noRequests));
  h.action('retry');
  h.respond({ available: false, unavailableReason: 'service_unavailable' });
  assert.ok(h.details.innerHTML.includes(usageLabels.serviceUnavailable));
  assert.ok(!h.details.innerHTML.includes(usageLabels.noRequests));
});

test('tiers show explicit requested aliases separately from raw response, retain legacy provenance and escape content', () => {
  const h = harness();
  h.action('tab', 'logs');
  h.respond({ requests: { rows: [
    row(1, { requestedServiceTier: 'FAST', responseServiceTier: 'default' }),
    row(2, { serviceTier: 'priority' }),
    row(3, { requestedServiceTier: 'auto', serviceTierAdaptation: { requestedTier: 'priority', sentTier: 'default', reason: 'official_default_only' } }),
    row(4, { requestedServiceTier: '<script>bad()</script>', responseServiceTier: 'priority' }),
  ] } });
  const html = h.details.innerHTML;
  assert.ok(html.includes(usageLabels.requestTier.replace('{{tier}}', usageLabels.tierFast)));
  assert.ok(html.includes(usageLabels.responseTier.replace('{{tier}}', 'default')));
  assert.ok(html.includes(usageLabels.legacyTier.replace('{{tier}}', 'priority')));
  assert.ok(html.includes(usageLabels.tierAdjusted));
  assert.ok(html.includes(usageLabels.tierUnknown));
  assert.ok(html.includes('high'));
  assert.ok(!html.includes('<script>bad()'));
  h.action('tab', 'overview');
  assert.ok(h.details.innerHTML.includes(usageLabels.averageSpeed));
  assert.ok(h.details.innerHTML.includes('29'));
});
