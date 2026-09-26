import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync, readdirSync } from 'node:fs';
import type { CodexAccount } from '../types/codex';
import { batchBindSkipped, batchBindSummary, batchBindTargets, executeProxyBatch, type BatchBindResult } from './codexProxyBatch';

type Binding = CodexAccount['egress_proxy'];
const account = (id: string): CodexAccount => ({ id } as CodexAccount);
const bound = (sourceId: string, itemId: string): Binding => ({ protocol: 'catalog', sourceId, itemId } as Binding);

const accounts = [account('same'), account('other-source'), account('other-node'), account('unbound')];
const saved = (entry: CodexAccount): Binding => {
  if (entry.id === 'same') return bound('source-a', 'node-1');
  if (entry.id === 'other-source') return bound('source-b', 'node-1');
  if (entry.id === 'other-node') return bound('source-a', 'node-2');
  return null;
};

test('batch binding skips accounts already on that node and reports what it will touch', () => {
  const targets = batchBindTargets(accounts, saved, 'source-a', 'node-1');
  assert.deepEqual(targets.map((entry) => entry.account.id), ['other-source', 'other-node', 'unbound']);
  assert.deepEqual(targets.map((entry) => entry.willOverwrite), [true, true, false]);
  assert.deepEqual(batchBindSkipped(accounts, saved, 'source-a', 'node-1'), { same: 1 });
});

test('batch binding overwrites every different binding by default', () => {
  const targets = batchBindTargets(accounts, saved, 'source-a', 'node-9');
  assert.deepEqual(targets.map((entry) => entry.account.id), accounts.map((entry) => entry.id));
  assert.deepEqual(targets.map((entry) => entry.willOverwrite), [true, true, true, false]);
  assert.deepEqual(batchBindSkipped(accounts, saved, 'source-a', 'node-9'), { same: 0 });
});

test('an identical batch performs no writes and reports unchanged accounts', () => {
  const onlyBound = [account('same')];
  assert.deepEqual(batchBindTargets(onlyBound, saved, 'source-a', 'node-1'), []);
  assert.deepEqual(batchBindSkipped(onlyBound, saved, 'source-a', 'node-1'), { same: 1 });
});

test('batch binding does not skip a manual group when its chosen member changes', () => {
  const onlyBound = [account('same')];
  const grouped = (): Binding => ({ protocol: 'RESOURCE', sourceId: 'source-a', itemId: 'manual', groupId: 'manual', selectedName: 'Alpha' });
  const targets = batchBindTargets(onlyBound, grouped, 'source-a', 'manual', 'manual', { manual: 'Beta' });
  assert.deepEqual(targets.map((entry) => entry.account.id), ['same']);
  assert.deepEqual(batchBindSkipped(onlyBound, grouped, 'source-a', 'manual', 'manual', { manual: 'Beta' }), { same: 0 });
});

test('batch results summarize partial failures instead of hiding them', () => {
  assert.deepEqual(batchBindSummary([
    { accountId: 'a', ok: true },
    { accountId: 'b', ok: false, errorKey: 'codex.proxy.catalog.failed' },
    { accountId: 'c', ok: true },
  ]), { total: 3, success: 2, failed: 1 });
  assert.deepEqual(batchBindSummary([]), { total: 0, success: 0, failed: 0 });
});

test('batch binding writes one binding per account and keeps the dialog usable', () => {
  const dialog = readFileSync(new URL('../components/codex/CodexProxyBatchBindDialog.tsx', import.meta.url), 'utf8');
  assert.match(dialog, /bindProxyCatalog\(entry\.id, source\.id, itemId, selections \?\? \{\}, groupId\)/);
  assert.match(dialog, /await executeProxyBatch\(targets/);
  assert.match(dialog, /useModalFocusTrap\(dialog, true\)/);
  assert.match(dialog, /useEscCloseTopmost\(true/);
  assert.match(dialog, /cancelProxyCatalog\(requestId\.current\)/);
  assert.match(dialog, /<span className="codex-proxy-batch-progress" role="status">/);
  assert.doesNotMatch(dialog, /onlyUnbound|batchOnlyUnbound|batchUntouched|type="checkbox"/);
  // Overlay clicks must never close a dialog (project modal rule).
  assert.doesNotMatch(dialog, /modal-overlay[^>]*onClick/);
});

test('batch dialog prefills the source default without preselecting accounts', () => {
  const dialog = readFileSync(new URL('../components/codex/CodexProxyBatchBindDialog.tsx', import.meta.url), 'utf8');
  // A source default only fills the draft; an explicit seed from the resources page still wins.
  assert.match(dialog, /sourceDefaultDraft\(initialSource\)/);
  assert.match(dialog, /useState\(initialItemId \?\? initialDefault\?\.itemId \?\? ''\)/);
  assert.match(dialog, /sourceDefaultDraft\(availableSources\.find\(\(entry\) => entry\.id === next\)\)/);
  assert.doesNotMatch(dialog, /bindProxyCatalog\([^)]*\)[\s\S]{0,80}onMount/);
});

test('batch dialog sizing is semantic, viewport-bounded and internally scrollable', () => {
  const css = readFileSync(new URL('../styles/pages/codex-proxy-batch.css', import.meta.url), 'utf8');
  assert.match(css, /\.modal\.codex-proxy-batch \{/);
  // Short content must not leave a tall empty dialog, but the height stays viewport-bounded.
  assert.match(css, /height: auto/);
  assert.match(css, /max-height: min\(820px, calc\(100vh - 48px\)\)/);
  assert.doesNotMatch(css, /max-height:\s*none/);
  assert.match(css, /\.codex-proxy-batch \.modal-body \{[^}]*overflow-y: auto/);
  assert.match(css, /\.codex-proxy-batch \.modal-footer \{ flex: 0 0 auto/);
  assert.match(css, /\.codex-proxy-batch-bar \{[^}]*border-top: 1px solid var\(--border\)/);
  assert.match(css, /\.codex-proxy-account-check \{/);
  const section = readFileSync(new URL('../components/codex/CodexProxyAccountsSection.tsx', import.meta.url), 'utf8');
  assert.match(section, /role="checkbox" aria-checked=\{allVisibleSelected/);
  assert.match(section, /onClick=\{toggleAll\}/);
  assert.match(section, /old\.filter\(\(id\) => !ids\.includes\(id\)\)/);
  assert.doesNotMatch(section, /codex-overview-select-all/);
});

test('account table offers explicit batch assignment and confirmed restoration of the shared default', () => {
  const section = readFileSync(new URL('../components/codex/CodexProxyAccountsSection.tsx', import.meta.url), 'utf8');
  assert.match(section, /codex\.proxy\.batchSelect/);
  assert.match(section, /codex\.proxy\.batchBind/);
  assert.match(section, /useState<string\[\]>\(\[\]\)/);
  assert.match(section, /setFollowOpen\(true\)/);
  assert.match(section, /<CodexProxyFollowDialog accounts=\{pickedAccounts\.filter/);
  const follow = readFileSync(new URL('../components/codex/CodexProxyAccountDialog.tsx', import.meta.url), 'utf8');
  assert.match(follow, /codex\.proxy\.managerAccounts\.followConfirm/);
  assert.match(follow, /bind: \(entry\) => useCodexAccountStore\.getState\(\)\.updateAccountEgressProxy\(entry\.id, null\)/);
  assert.match(follow, /<ModalErrorMessage message=\{error\}/);
  assert.doesNotMatch(follow, /modal-overlay[^>]*onClick/);
  const assignment = readFileSync(new URL('../components/codex/CodexProxyAssignDialog.tsx', import.meta.url), 'utf8');
  assert.match(assignment, /useState<string\[\]>\(\[\]\)/);
  assert.match(assignment, /proxyAssignmentAccounts\(accounts, selectedIds\)/);
  assert.match(assignment, /onClick=\{\(\) => void save\(\)\}/);
  assert.match(assignment, /previewCodexUnifiedProxy/);
  assert.match(assignment, /applyCodexUnifiedProxy/);
  assert.match(assignment, /cancelled: \(\) => cancelled\.current/);
  assert.match(assignment, /applyAccountSnapshot\(updated\)/);
  assert.doesNotMatch(assignment, /modal-overlay[^>]*onClick/);
  // Browsing/importing resources only prepares the local assignment dialog, never writes.
  const resourcesSection = readFileSync(new URL('../components/codex/CodexProxyResourcesSection.tsx', import.meta.url), 'utf8');
  assert.match(resourcesSection, /onAssign=\{.*setAssignment/);
  assert.match(resourcesSection, /<CodexProxyAssignDialog choice=\{assignment\}/);
  assert.doesNotMatch(resourcesSection, /bindProxyCatalog|updateAccountEgressProxy/);
});

test('every locale carries the batch binding keys with real translations', () => {
  const keys = ['batchTitle', 'batchSubtitle', 'batchSelect', 'batchExit', 'batchSelectedCount', 'batchBind',
    'batchClearAll', 'batchSummary', 'batchPickNode', 'batchOverwrite', 'batchAlreadySame',
    'batchEmpty', 'batchApply', 'batchRunning', 'batchProgress', 'batchResult', 'batchUnbindMessage'];
  const dir = new URL('../locales/', import.meta.url);
  const locales = readdirSync(dir).filter((file) => file.endsWith('.json'));
  assert.ok(locales.length >= 18);
  const english = JSON.parse(readFileSync(new URL('en-US.json', dir), 'utf8')).codex.proxy;
  for (const file of locales) {
    const proxy = JSON.parse(readFileSync(new URL(file, dir), 'utf8')).codex.proxy;
    for (const key of keys) assert.ok(proxy[key], `${file} is missing codex.proxy.${key}`);
    assert.ok(proxy.catalog.chooseAccounts, `${file} is missing codex.proxy.catalog.chooseAccounts`);
    if (!file.startsWith('en')) {
      assert.notEqual(proxy.batchTitle, english.batchTitle, `${file} must localize batchTitle`);
    }
  }
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

for (const failure of [false, true]) {
  test(`cancelling an in-flight batch preserves its ${failure ? 'failure' : 'success'} and stops further writes`, async () => {
    const pending = deferred<CodexAccount>();
    const writes: string[] = [];
    const applied: string[] = [];
    const progress: BatchBindResult[][] = [];
    let cancelled = false;
    const run = executeProxyBatch(batchBindTargets(accounts, saved, 'source-a', 'node-9'), {
      cancelled: () => cancelled,
      bind: (entry) => { writes.push(entry.id); return pending.promise; },
      applied: (entry) => { applied.push(entry.id); },
      errorKey: () => 'codex.proxy.catalog.failed',
      progress: (next) => { progress.push(next); },
    });
    assert.deepEqual(writes, ['same']);
    cancelled = true;
    if (failure) pending.reject(new Error('write failed'));
    else pending.resolve(account('same'));
    await run;
    assert.deepEqual(writes, ['same']);
    assert.deepEqual(applied, failure ? [] : ['same']);
    assert.deepEqual(progress, [[failure
      ? { accountId: 'same', ok: false, errorKey: 'codex.proxy.catalog.failed' }
      : { accountId: 'same', ok: true }]]);
  });
}

test('a fresh batch after cancellation runs sequentially and continues after an account fails', async () => {
  const applied: string[] = [];
  const progress: BatchBindResult[][] = [];
  let active = 0;
  await executeProxyBatch(batchBindTargets(accounts, saved, 'source-a', 'node-9'), {
    cancelled: () => false,
    bind: async (entry) => {
      active += 1;
      assert.equal(active, 1);
      await Promise.resolve();
      active -= 1;
      if (entry.id === 'other-source') throw new Error('write failed');
      return entry;
    },
    applied: (entry) => { applied.push(entry.id); },
    errorKey: () => 'codex.proxy.catalog.failed',
    progress: (next) => { progress.push(next); },
  });
  assert.deepEqual(applied, ['same', 'other-node', 'unbound']);
  assert.deepEqual(progress.map((next) => next.length), [1, 2, 3, 4]);
  assert.deepEqual(batchBindSummary(progress[progress.length - 1]), { total: 4, success: 3, failed: 1 });
});

test('batch lifecycle resets StrictMode cleanup and releases busy state even when cancelled', () => {
  const dialog = readFileSync(new URL('../components/codex/CodexProxyBatchBindDialog.tsx', import.meta.url), 'utf8');
  assert.match(dialog, /useEffect\(\(\) => \{\s*mounted.current = true;\s*cancelled.current = false;/);
  assert.match(dialog, /finally \{\s*running.current = false;\s*if \(mounted.current\) \{ setBusy\(false\); setProgress\(null\); \}/);
  assert.match(dialog, /SingleSelectDropdown[^>]*disabled=\{busy \|\| testing \|\| catalogPending\}/);
  assert.match(dialog, /className="btn btn-primary" disabled=\{busy \|\| testing/);
});

test('batch binding saves a different group context even when the node is unchanged', () => {
  assert.equal(batchBindTargets([account('same')], saved, 'source-a', 'node-1', 'usa').length, 1);
  const grouped = () => ({ ...bound('source-a', 'node-1')!, groupId: 'usa' });
  assert.equal(batchBindTargets([account('same')], grouped, 'source-a', 'node-1', 'usa').length, 0);
  assert.deepEqual(batchBindSkipped([account('same')], grouped, 'source-a', 'node-1', 'usa'), { same: 1 });
});
