import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';
import * as enginePrerequisite from './codexProxyEnginePrerequisite';
import type { ProxyCatalogNode, ProxyCatalogGroup, ProxyCatalogSource } from '../services/codexProxyCatalogService';
import {
  strategyCandidates, filterStrategyCandidates, isPossibleProxyNotice, strategyEditorMembers, strategyErrorKey, strategyKindKey, strategyMemberViews, strategyNameTaken, strategyNoticeKey,
  strategyOptionErrors, strategyOptions, strategyOptionsForm, strategyOrderedMembers, strategyViews,
  type ProxyStrategyOptionsForm,
} from '../services/codexProxyStrategyService';

/** The IPC boundary is exercised through the same VM harness the catalog service test uses. */
const compiled = ts.transpileModule(readFileSync(new URL('../services/codexProxyStrategyService.ts', import.meta.url), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;
function harness(invoke: (command: string, args?: unknown) => Promise<unknown>) {
  const exports: Record<string, any> = {};
  const timers = new Map<number, () => void>();
  let timerId = 0;
  vm.runInNewContext(compiled, {
    exports,
    require: (name: string) => name.endsWith('codexProxyEnginePrerequisite') ? enginePrerequisite : ({ invoke }),
    setTimeout: (callback: () => void) => { timers.set(++timerId, callback); return timerId; },
    clearTimeout: (id: number) => timers.delete(id),
  });
  return { service: exports, timers };
}

const node = (id: string, name: string, supported = true): ProxyCatalogNode => ({ id, name, protocol: 'socks5', supported, error: null });
const group = (id: string, kind: string, members: string[]): ProxyCatalogGroup => ({ id, name: id, kind, members, supported: true, error: null });
const source = (id: string, name: string, kind: ProxyCatalogSource['kind'], nodes: ProxyCatalogNode[], groups: ProxyCatalogGroup[] = [],
  strategyMembers: ProxyCatalogSource['strategyMembers'] = null): ProxyCatalogSource => ({
  id, name, kind, nodes, groups, updatedAt: 1, lastAttemptAt: null, revision: '1', autoUpdate: false, error: null, default: null, defaultInvalidated: false, strategyMembers,
});
const emptyForm = (): ProxyStrategyOptionsForm => ({ url: '', interval: '', timeout: '', tolerance: '', lazy: true });

test('member search matches node endpoints, independently of source names and selection', () => {
  const candidates = strategyCandidates([
    source('a', '苏菲家宽', 'subscription', [
      { ...node('one', '日本节点'), server: 'jp.example.com', port: 443 },
      { ...node('two', '美国家宽'), server: '192.0.2.1', port: 1080 },
    ]),
    source('b', '其他来源', 'manual', [{ ...node('three', '家宽备用'), server: '2001:db8::1', port: 443 }]),
  ]);
  const ids = (query: string, sourceId = '') => filterStrategyCandidates(candidates, query, sourceId).map((entry) => entry.itemId);
  assert.deepEqual(ids('家宽'), ['two', 'three']);
  assert.deepEqual(ids('家宽', 'a'), ['two']);
  assert.deepEqual(ids('', 'a'), ['one', 'two']);
  assert.deepEqual(ids(' JP.EXAMPLE.COM:443 '), ['one']);
  assert.deepEqual(ids('192.0.2.1'), ['two']);
  assert.deepEqual(ids('1080'), ['two']);
  assert.deepEqual(ids('2001:db8::1'), ['three']);
  assert.deepEqual(ids('不存在'), []);
  assert.deepEqual(ids('socks5'), []);
  assert.deepEqual(ids('美国 家宽'), ['two']);
  assert.deepEqual(ids('家宽 美国'), ['two']);
  assert.deepEqual(ids('  美国   家宽  '), ['two']);
  assert.deepEqual(ids('美国\t家宽\n1080'), ['two']);
  assert.deepEqual(ids('美国　家宽'), ['two']);
  assert.deepEqual(ids('美国 家宽 192.0.2.1 1080', 'a'), ['two']);
  assert.deepEqual(ids('美国 家宽', 'b'), []);
  assert.deepEqual(ids('美国 日本'), []);
  assert.deepEqual(ids('家宽 443'), ['three']);
  assert.deepEqual(ids('  \t  ', 'a'), ['one', 'two']);
});

test('subscription notices are advisory only and remain selectable', () => {
  assert.equal(isPossibleProxyNotice('剩余流量：580 GB'), true);
  assert.equal(isPossibleProxyNotice('苏菲家宽官网地址：example.com'), true);
  assert.equal(isPossibleProxyNotice('美国家宽 01'), false);
  const candidates = strategyCandidates([source('a', '订阅', 'subscription', [node('notice', '套餐到期：2026-10-17')])]);
  assert.equal(filterStrategyCandidates(candidates, '', '').length, 1);
});

/** One saved strategy source as `codex_proxy_catalog_list` delivers it. The view is free-form JSON,
 * so a numeric field may still arrive as text even though the type declares a number. */
const savedSource = (strategyOptions: Record<string, unknown> | null | undefined): ProxyCatalogSource =>
  ({ ...source('strategy', 'Primary', 'strategy', [node('n1', 'Alpha')]), strategyOptions: strategyOptions as ProxyCatalogSource['strategyOptions'] });

test('member order is serialized as entered, without duplicates, blanks or overflow', () => {
  assert.deepEqual(
    strategyOrderedMembers([
      { sourceId: 'a', itemId: '1' }, { sourceId: 'b', itemId: '2' }, { sourceId: 'a', itemId: '1' },
      { sourceId: '', itemId: '9' }, { sourceId: 'c', itemId: '' },
    ]),
    [{ sourceId: 'a', itemId: '1' }, { sourceId: 'b', itemId: '2' }],
  );
  const many = Array.from({ length: 80 }, (_, index) => ({ sourceId: 's', itemId: `n${index}` }));
  const capped = strategyOrderedMembers(many);
  assert.equal(capped.length, 64);
  assert.equal(capped[0].itemId, 'n0');
  assert.equal(capped[63].itemId, 'n63');
});

test('a one-member strategy is announced as a fixed node, never as failover', () => {
  assert.equal(strategyNoticeKey('fallback', 1), 'codex.proxy.catalog.strategySingleNotice');
  assert.equal(strategyNoticeKey('url-test', 0), 'codex.proxy.catalog.strategySingleNotice');
  assert.equal(strategyNoticeKey('fallback', 2), 'codex.proxy.catalog.strategyHintFallback');
  assert.equal(strategyNoticeKey('url-test', 3), 'codex.proxy.catalog.strategyHintUrlTest');
  assert.equal(strategyNoticeKey('load-balance', 3), 'codex.proxy.catalog.strategyHintLoadBalance');
  assert.equal(strategyNoticeKey('select', 4), 'codex.proxy.catalog.strategyHintSelect');
  assert.equal(strategyKindKey('fallback'), 'codex.proxy.catalog.fallbackGroup');
  assert.equal(strategyKindKey('url-test'), 'codex.proxy.catalog.latencyGroup');
  assert.equal(strategyKindKey('load-balance'), 'codex.proxy.catalog.loadBalanceGroup');
  assert.equal(strategyKindKey('select'), 'codex.proxy.catalog.manualGroup');
  // An unknown group kind must not be rendered as an unsupported-strategy warning.
  assert.equal(strategyKindKey('relay'), 'codex.proxy.catalog.strategyTag');
});

test('advanced fields accept only their documented ranges', () => {
  assert.deepEqual(strategyOptionErrors(emptyForm(), 'url-test'), {});
  assert.equal(strategyOptionErrors({ ...emptyForm(), url: 'ftp://host/check' }, 'url-test').url, 'codex.proxy.catalog.strategyErrorUrl');
  assert.equal(strategyOptionErrors({ ...emptyForm(), url: 'https://user:secret@host/check' }, 'url-test').url, 'codex.proxy.catalog.strategyErrorUrl');
  assert.equal(strategyOptionErrors({ ...emptyForm(), url: 'https://host/check#fragment' }, 'url-test').url, 'codex.proxy.catalog.strategyErrorUrl');
  assert.equal(strategyOptionErrors({ ...emptyForm(), url: `https://host/${'a'.repeat(2100)}` }, 'url-test').url, 'codex.proxy.catalog.strategyErrorUrl');
  assert.equal(strategyOptionErrors({ ...emptyForm(), interval: '29' }, 'url-test').interval, 'codex.proxy.catalog.strategyErrorRange');
  assert.equal(strategyOptionErrors({ ...emptyForm(), interval: '3601' }, 'url-test').interval, 'codex.proxy.catalog.strategyErrorRange');
  assert.equal(strategyOptionErrors({ ...emptyForm(), interval: '2.5' }, 'url-test').interval, 'codex.proxy.catalog.strategyErrorRange');
  assert.equal(strategyOptionErrors({ ...emptyForm(), timeout: '0' }, 'url-test').timeout, 'codex.proxy.catalog.strategyErrorRange');
  assert.equal(strategyOptionErrors({ ...emptyForm(), timeout: '31' }, 'url-test').timeout, 'codex.proxy.catalog.strategyErrorRange');
  assert.equal(strategyOptionErrors({ ...emptyForm(), tolerance: '1001' }, 'url-test').tolerance, 'codex.proxy.catalog.strategyErrorRange');
  assert.deepEqual(strategyOptionErrors({ url: 'https://host/check', interval: '30', timeout: '1', tolerance: '0', lazy: true }, 'url-test'), {});
  // A value a kind never reads must not block saving it.
  assert.deepEqual(strategyOptionErrors({ ...emptyForm(), tolerance: '9999' }, 'fallback'), {});
});

test('blank advanced fields keep the engine default instead of sending zero', () => {
  assert.deepEqual(strategyOptions({ ...emptyForm(), lazy: false }, 'url-test'), { lazy: false });
  assert.deepEqual(
    strategyOptions({ url: ' https://host/check ', interval: '600', timeout: '5', tolerance: '50', lazy: true }, 'url-test'),
    { lazy: true, url: 'https://host/check', interval: 600, timeout: 5, tolerance: 50 },
  );
  assert.deepEqual(strategyOptions({ ...emptyForm(), interval: '1' }, 'url-test'), { lazy: true });
});

test('a saved strategy restores every advanced field into the form', () => {
  // The view already divided the stored milliseconds by 1000, so 5 here is the saved 5000 ms.
  const form = strategyOptionsForm(savedSource({ kind: 'url-test', url: 'https://host/check', interval: 600, timeout: 5, tolerance: 50, lazy: false }));
  assert.deepEqual(form, { url: 'https://host/check', interval: '600', timeout: '5', tolerance: '50', lazy: false });
  // Text and number inputs end up as the same form text; neither shape may become "NaN".
  assert.deepEqual(
    strategyOptionsForm(savedSource({ url: 'https://host/check', interval: '600', timeout: '5', tolerance: '50', lazy: false })),
    form,
  );
  // A null field is blank rather than the literal text "null".
  assert.deepEqual(strategyOptionsForm(savedSource({ url: null, interval: null, timeout: null, tolerance: null, lazy: null })), emptyForm());
});

test('a source without saved options restores a blank form, never undefined or NaN', () => {
  const form = strategyOptionsForm(savedSource({}));
  assert.deepEqual(form, emptyForm());
  assert.deepEqual(strategyOptionsForm(savedSource(undefined)), form);
  assert.deepEqual(strategyOptionsForm(savedSource(null)), form);
  assert.deepEqual(strategyOptionsForm(source('sub', 'Example', 'subscription', [node('n1', 'Alpha')])), form);
  assert.equal(Object.values(form).some((value) => /NaN|undefined/.test(String(value))), false);
  // A blank field means "keep the engine default" and carries no number at all.
  assert.deepEqual(strategyOptions(form, 'url-test'), { lazy: true });
  assert.deepEqual(strategyOptionErrors(form, 'url-test'), {});
});

test('restoring a saved strategy re-submits the same parameters, in seconds', () => {
  const form = strategyOptionsForm(savedSource({ kind: 'url-test', url: 'https://host/check', interval: 600, timeout: 5, tolerance: 50, lazy: false }));
  // Both the form and the payload are seconds; only the encoder turns them into milliseconds.
  assert.deepEqual(strategyOptions(form, 'url-test'),
    { lazy: false, url: 'https://host/check', interval: 600, timeout: 5, tolerance: 50 });
  // A kind only submits what it applies, so its own round trip stays exact as well.
  const fallback = strategyOptionsForm(savedSource({ kind: 'fallback', url: 'https://host/check', interval: 600, timeout: 5, lazy: true }));
  assert.deepEqual(strategyOptions(fallback, 'fallback'), { lazy: true, url: 'https://host/check', interval: 600, timeout: 5 });
  // `select` never reads its advanced fields, so a saved value is deliberately not re-submitted.
  assert.deepEqual(strategyOptions(form, 'select'), {});
});

test('a stored value the engine would reject stays visible instead of being re-submitted', () => {
  // `lazy` was never stored, so the form fills the preset and the payload states it explicitly.
  const withoutLazy = strategyOptionsForm(savedSource({ kind: 'url-test', url: 'https://host/check', interval: 600 }));
  assert.equal(withoutLazy.lazy, true);
  assert.deepEqual(strategyOptions(withoutLazy, 'url-test'), { lazy: true, url: 'https://host/check', interval: 600 });
  // An out-of-range value from disk stays in its field with its error, and never reaches the payload.
  const stale = strategyOptionsForm(savedSource({ interval: 20, timeout: 0 }));
  assert.deepEqual(stale, { ...emptyForm(), interval: '20', timeout: '0' });
  assert.equal(strategyOptionErrors(stale, 'url-test').interval, 'codex.proxy.catalog.strategyErrorRange');
  assert.equal(strategyOptionErrors(stale, 'url-test').timeout, 'codex.proxy.catalog.strategyErrorRange');
  assert.deepEqual(strategyOptions(stale, 'url-test'), { lazy: true });
});

test('each kind only carries the options the engine applies to it', () => {
  const full = { url: 'https://host/check', interval: '600', timeout: '5', tolerance: '50', lazy: false };
  assert.deepEqual(strategyOptions(full, 'select'), {});
  assert.deepEqual(strategyOptions(full, 'fallback'), { lazy: false, url: 'https://host/check', interval: 600, timeout: 5 });
  assert.deepEqual(strategyOptions(full, 'url-test'), { lazy: false, url: 'https://host/check', interval: 600, timeout: 5, tolerance: 50 });
  assert.deepEqual(strategyOptions(full, 'load-balance'), { lazy: false, url: 'https://host/check', interval: 600 });
});

test('a strategy name may not shadow one of its member nodes', () => {
  assert.equal(strategyNameTaken('Alpha', ['Alpha', 'Beta']), true);
  assert.equal(strategyNameTaken('  Alpha  ', ['Alpha']), true);
  assert.equal(strategyNameTaken('Primary', ['Alpha', 'Beta']), false);
  assert.equal(strategyNameTaken('   ', ['']), false);
});

test('strategy failures reuse the catalog allowlist with strategy-specific input copy', () => {
  assert.equal(strategyErrorKey('CATALOG_INVALID'), 'codex.proxy.catalog.strategyErrorInvalid');
  assert.equal(strategyErrorKey('CATALOG_NAME'), 'codex.proxy.catalog.strategyErrorInvalid');
  assert.equal(strategyErrorKey(new Error('CATALOG_TIMEOUT')), 'codex.proxy.catalog.timeout');
  assert.equal(strategyErrorKey('CATALOG_PARTIAL_UNBIND'), 'codex.proxy.catalog.partialDelete');
  assert.equal(strategyErrorKey('https://user:secret@host/?token=secret'), 'codex.proxy.catalog.failed');
});

test('save and remove map to the documented IPC commands', async () => {
  const calls: { command: string; args?: unknown }[] = [];
  const { service } = harness(async (command, args) => { calls.push({ command, args }); return { sources: [] }; });
  const draft = { name: '  Primary  ', kind: 'fallback', members: [{ sourceId: 'a', itemId: '1' }, { sourceId: 'a', itemId: '1' }], options: { lazy: true } };
  await service.saveProxyStrategy(draft);
  await service.saveProxyStrategy({ ...draft, id: 'strategy', options: { url: 'https://host/check', interval: 600, timeout: 5, tolerance: 50, lazy: false } });
  await service.removeProxyStrategy('strategy');
  assert.equal(JSON.stringify(calls), JSON.stringify([
    { command: 'codex_proxy_strategy_save', args: { name: 'Primary', kind: 'fallback', members: [{ sourceId: 'a', itemId: '1' }], options: { lazy: true } } },
    { command: 'codex_proxy_strategy_save', args: { id: 'strategy', name: 'Primary', kind: 'fallback', members: [{ sourceId: 'a', itemId: '1' }], options: { url: 'https://host/check', interval: 600, timeout: 5, tolerance: 50, lazy: false } } },
    { command: 'codex_proxy_strategy_remove', args: { id: 'strategy' } },
  ]));
});

test('saved strategies keep their order and expose members whose source is gone', () => {
  const subscription = source('sub', 'Example', 'subscription', [node('n1', 'Alpha'), node('n2', 'Beta')]);
  const manual = source('manual', 'Manual', 'manual', [node('n3', 'Gamma'), node('n4', 'Broken', false)]);
  const strategy = source('strategy', 'Primary', 'strategy', [node('n1', 'Alpha'), node('n2', 'Beta')],
    [group('group', 'fallback', ['Beta', 'Alpha', 'Gone'])]);
  const views = strategyViews([subscription, manual, strategy]);
  assert.equal(views.length, 1);
  assert.equal(views[0].kind, 'fallback');
  assert.deepEqual(views[0].members.map((member) => [member.name, member.matched, member.sourceId, member.itemId]),
    [['Beta', true, 'sub', 'n2'], ['Alpha', true, 'sub', 'n1'], ['Gone', false, '', '']]);
  // Candidate members never come from another strategy and never include unsupported nodes.
  assert.deepEqual(strategyCandidates([subscription, manual, strategy]).map((entry) => entry.itemId), ['n1', 'n2', 'n3']);
});

test('a member name shared by two sources is only matched by its stable id', () => {
  const first = source('first', 'First', 'manual', [node('x1', 'Tokyo')]);
  const second = source('second', 'Second', 'manual', [node('y1', 'Tokyo')]);
  const strategy = source('strategy', 'Pair', 'strategy', [node('y1', 'Tokyo')], [group('group', 'select', ['Tokyo'])]);
  const views = strategyViews([first, second, strategy]);
  assert.deepEqual(views[0].members.map((member) => [member.sourceId, member.itemId, member.matched]), [['second', 'y1', true]]);
});

test('saved member identities are read back exactly, even when sources share a name', () => {
  const first = source('first', 'First', 'manual', [node('x1', 'Hong Kong 01')]);
  const second = source('second', 'Second', 'manual', [node('y1', 'Hong Kong 01')]);
  const strategy = source('strategy', 'Pair', 'strategy', [node('x1', 'Hong Kong 01'), node('y1', 'Hong Kong 01')],
    [group('group', 'select', ['Hong Kong 01'])],
    [{ sourceId: 'second', itemId: 'y1', name: 'Hong Kong 01', sourceName: 'Second' }]);
  const views = strategyViews([first, second, strategy]);
  // 记录指向 second/y1：绝不能因为先出现的 first/x1 同名就串位。
  assert.deepEqual(views[0].members.map((member) => [member.sourceId, member.itemId, member.matched, member.originRemoved, member.sourceName]),
    [['second', 'y1', true, false, 'Second']]);
});

test('a strategy without identity records keeps the legacy name fallback', () => {
  const subscription = source('sub', 'Example', 'subscription', [node('n1', 'Alpha')]);
  const strategy = source('strategy', 'Primary', 'strategy', [node('copy', 'Alpha')], [group('group', 'fallback', ['Alpha'])]);
  const views = strategyViews([subscription, strategy]);
  assert.deepEqual(views[0].members.map((member) => [member.sourceId, member.itemId, member.matched, member.originRemoved]),
    [['sub', 'n1', true, false]]);
});

test('a member whose original source is gone still works from its saved copy', () => {
  const strategy = source('strategy', 'Pair', 'strategy', [node('copy', 'Tokyo')], [group('group', 'select', ['Tokyo'])],
    [{ sourceId: 'gone', itemId: 'g1', name: 'Tokyo', sourceName: 'Gone source' }]);
  const views = strategyViews([strategy]);
  assert.deepEqual(views[0].members.map((member) => [member.matched, member.originRemoved, member.sourceName, member.itemId]),
    [[true, true, 'Gone source', 'g1']]);
  assert.equal(views[0].members.some((member) => !member.matched), false, '原来源已删除不能显示为失效');
});

test('a member whose candidate is missing stays unmatched instead of being replaced', () => {
  const subscription = source('sub', 'Example', 'subscription', [node('n1', 'Alpha')]);
  const other = source('other', 'Other', 'manual', [node('t9', 'Tokyo')]);
  const strategy = source('strategy', 'Pair', 'strategy', [node('copy', 'Tokyo')], [group('group', 'select', ['Tokyo'])],
    [{ sourceId: 'sub', itemId: 'missing', name: 'Tokyo', sourceName: 'Example' }]);
  const views = strategyViews([subscription, other, strategy]);
  // 来源仍在、节点 ID 已消失：保持未匹配，绝不能改绑到同名节点 other/t9。
  assert.deepEqual(views[0].members.map((member) => [member.matched, member.originRemoved, member.sourceId, member.itemId]),
    [[false, false, 'sub', 'missing']]);
});

test('members whose original source is gone enter the editor as saved copies', () => {
  const strategy = source('strategy', 'Pair', 'strategy', [node('copy-a', 'Alpha'), node('copy-b', 'Beta')],
    [group('group', 'select', ['Alpha', 'Beta'])],
    [
      { sourceId: 'gone', itemId: 'g1', name: 'Alpha', sourceName: 'Gone source' },
      { sourceId: 'gone', itemId: 'g2', name: 'Beta', sourceName: 'Gone source' },
    ]);
  const editor = strategyEditorMembers(strategyMemberViews(strategy, [strategy]));
  // 仍然可用的副本必须能原样提交，并带上“使用已保存副本”的标记与可回读的真实身份。
  assert.deepEqual(editor.members.map((member) => [member.name, member.savedCopy, member.sourceId, member.itemId, member.sourceName]),
    [['Alpha', true, 'gone', 'g1', 'Gone source'], ['Beta', true, 'gone', 'g2', 'Gone source']]);
  assert.deepEqual(editor.kept.map((member) => member.name), ['Alpha', 'Beta']);
  assert.deepEqual(editor.unmatched, []);
});

test('members that no longer resolve never enter the editor list', () => {
  const subscription = source('sub', 'Example', 'subscription', [node('n1', 'Alpha')]);
  const other = source('other', 'Other', 'manual', [node('t9', 'Tokyo')]);
  const strategy = source('strategy', 'Pair', 'strategy', [node('copy', 'Tokyo')], [group('group', 'select', ['Tokyo'])],
    [{ sourceId: 'sub', itemId: 'missing', name: 'Tokyo', sourceName: 'Example' }]);
  const editor = strategyEditorMembers(strategyMemberViews(strategy, [subscription, other, strategy]));
  assert.deepEqual(editor.members, []);
  assert.deepEqual(editor.kept, []);
  assert.deepEqual(editor.unmatched, ['Tokyo']);
});

test('a strategy without identity records keeps the legacy name match in the editor', () => {
  const subscription = source('sub', 'Example', 'subscription', [node('n1', 'Alpha')]);
  const strategy = source('strategy', 'Primary', 'strategy', [node('copy', 'Alpha')], [group('group', 'fallback', ['Alpha'])]);
  const editor = strategyEditorMembers(strategyMemberViews(strategy, [subscription, strategy]));
  assert.deepEqual(editor.members.map((member) => [member.name, member.sourceId, member.itemId, member.savedCopy]),
    [['Alpha', 'sub', 'n1', false]]);
  assert.deepEqual(editor.unmatched, []);
  assert.deepEqual(editor.kept, []);
});
