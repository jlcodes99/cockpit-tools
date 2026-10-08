import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';
import * as enginePrerequisite from '../utils/codexProxyEnginePrerequisite';

const compiled = ts.transpileModule(readFileSync(new URL('./codexProxyCatalogService.ts', import.meta.url), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;
function harness(invoke: (command: string, args?: unknown) => Promise<unknown>) {
  const exports: Record<string, any> = {};
  const timers = new Map<number, () => void>();
  let timerId = 0;
  vm.runInNewContext(compiled, { exports, require: (name: string) => name.endsWith('codexProxyEnginePrerequisite') ? enginePrerequisite : ({ invoke }),
    setTimeout: (callback: () => void) => { timers.set(++timerId, callback); return timerId; },
    clearTimeout: (id: number) => timers.delete(id),
  });
  return { service: exports, timers };
}
const node = { id: 'node', name: 'Tokyo', protocol: 'vless', supported: true, error: null };
const group = (id: string, name: string, kind: string, members: string[]) => ({ id, name, kind, members, supported: true, error: null });
const source = { id: 'source', name: 'Example', nodes: [node], groups: [
  group('root', 'Proxy', 'select', ['Auto', 'Tokyo']), group('auto', 'Auto', 'url-test', ['Nested', 'Tokyo']),
  group('nested', 'Nested', 'select', ['Tokyo']), group('unrelated', 'Other', 'select', ['Tokyo']),
] };

test('selectors follow only the chosen and reachable groups, never unrelated groups', () => {
  const { service } = harness(async () => ({}));
  const ids = (selections: Record<string, string>) => Array.from(service.catalogSelectors(source, 'root', selections), (g: any) => g.id);
  assert.deepEqual(ids({}), ['root']);
  assert.deepEqual(ids({ root: 'Tokyo' }), ['root']);
  assert.deepEqual(ids({ root: 'Auto' }), ['root', 'nested']);
  assert.deepEqual(Array.from(service.catalogSelectors(source, 'node', {})), []);
});

test('selector traversal terminates even for cyclic imported groups', () => {
  const { service } = harness(async () => ({}));
  const cyclic = { ...source, groups: [group('root', 'Proxy', 'select', ['Proxy'])] };
  assert.equal(service.catalogSelectors(cyclic, 'root', { root: 'Proxy' }).length, 1);
});

test('binding sends only identifiers and group choices, never subscription credentials', async () => {
  const calls: unknown[] = [];
  const { service } = harness(async (command, args) => { calls.push({ command, args }); return {}; });
  await service.bindProxyCatalog('account', 'source', 'root', { root: 'Tokyo' });
  await service.probeProxyCatalog('request', 'source', 'node', {});
  assert.equal(JSON.stringify(calls), JSON.stringify([
    { command: 'codex_proxy_catalog_bind', args: { accountId: 'account', sourceId: 'source', itemId: 'root', selections: { root: 'Tokyo' } } },
    { command: 'codex_proxy_catalog_probe', args: { requestId: 'request', sourceId: 'source', itemId: 'node', selections: {} } },
  ]));
});

test('a frontend timeout requests backend cancellation and never displays request data', async () => {
  const calls: { command: string; args?: unknown }[] = [];
  const { service, timers } = harness(async (command, args) => {
    calls.push({ command, args });
    if (command.endsWith('_cancel')) return;
    return new Promise(() => {});
  });
  const pending = service.importProxyCatalog('request', 'Example', 'https://example.com/?token=secret', 'subscription');
  const rejection = assert.rejects(pending, /CATALOG_TIMEOUT/);
  timers.values().next().value!();
  await rejection;
  assert.equal(JSON.stringify(calls[1]), JSON.stringify({ command: 'codex_proxy_catalog_cancel', args: { requestId: 'request' } }));
});

test('catalog list stays single-flight after frontend timeout until backend settles', async () => {
  let complete!: (value: unknown) => void;
  let calls = 0;
  const { service, timers } = harness(() => { calls++; return new Promise((resolve) => { complete = resolve; }); });
  const one = service.getProxyCatalog();
  const failure = assert.rejects(one, /CATALOG_TIMEOUT/);
  timers.values().next().value!();
  await failure;
  const two = service.getProxyCatalog();
  assert.equal(calls, 1);
  complete({ sources: [] });
  assert.equal((await two).sources.length, 0);
});

test('error allowlist never interpolates URLs, file paths, or raw backend messages', () => {
  const { service } = harness(async () => ({}));
  for (const input of ['https://user:secret@host/?token=secret', 'CATALOG_DOWNLOAD: secret', '/Users/user/private-file']) {
    assert.equal(service.catalogErrorKey(input), 'codex.proxy.catalog.failed');
  }
  assert.equal(service.catalogErrorKey(new Error('CATALOG_REDIRECT')), 'codex.proxy.catalog.downloadFailed');
  assert.equal(service.catalogErrorKey('CATALOG_FINISHING'), 'codex.proxy.catalog.busy');
  assert.equal(service.catalogErrorKey('CATALOG_CANCELLED'), 'common.cancelled');
  assert.equal(service.catalogErrorKey('CATALOG_PARTIAL_UNBIND'), 'codex.proxy.catalog.partialDelete');
  assert.equal(service.catalogErrorKey('PROXY_ENGINE_MISSING'), 'codex.proxy.engineMissing');
});

test('unavailable resource reasons distinguish protocol limits, group limits and missing members', () => {
  const { service } = harness(async () => ({}));
  const reasons = {
    SUBSCRIPTION_GROUP_STRATEGY: 'reasonStrategy', SUBSCRIPTION_GROUP_OPTIONS: 'reasonOptions',
    SUBSCRIPTION_GROUP_TIMEOUT: 'reasonGroupTimeout',
    SUBSCRIPTION_PROVIDER_UNSUPPORTED: 'reasonProvider', SUBSCRIPTION_GROUP_UNAVAILABLE: 'reasonMembers',
    PROXY_UNSUPPORTED_OPTION: 'reasonOptions', SUBSCRIPTION_INVALID: 'reasonInvalid',
    PROXY_TLS_INSECURE: 'reasonTlsInsecure', PROXY_TRANSPORT_UNSUPPORTED: 'reasonTransport',
    PROXY_ECH_UNSUPPORTED: 'reasonEch',
  };
  for (const [error, key] of Object.entries(reasons)) {
    assert.equal(service.catalogUnsupportedKey({ error }), `codex.proxy.catalog.${key}`);
  }
  assert.equal(service.catalogUnsupportedKey({ kind: 'relay' }), 'codex.proxy.catalog.reasonStrategy');
  assert.equal(service.catalogErrorKey('SUBSCRIPTION_GROUP_TIMEOUT'), 'codex.proxy.catalog.reasonGroupTimeout');
  assert.equal(service.catalogUnsupportedKey(undefined, 'DIRECT'), 'codex.proxy.catalog.reasonBuiltin');
  assert.equal(service.catalogUnsupportedKey(undefined, 'missing'), 'codex.proxy.catalog.reasonMissing');
});

test('resource reason text never interpolates an untrusted error, protocol or member name', () => {
  const { service } = harness(async () => ({}));
  for (const error of ['https://user:secret@host/?token=secret', 'PROXY_TLS_INSECURE: secret', '/private/file']) {
    assert.equal(service.catalogUnsupportedKey({ error, protocol: error }), 'codex.proxy.catalog.reasonUnknown');
    assert.equal(service.catalogUnsupportedKey(undefined, error), 'codex.proxy.catalog.reasonMissing');
  }
});

test('blocking group members have a fixed explanation while bypass rules remain unsupported', () => {
  const { service } = harness(async () => ({}));
  for (const member of ['REJECT', 'REJECT-DROP']) {
    assert.equal(service.isCatalogBlockingMember(member), true);
    assert.equal(service.catalogUnsupportedKey(undefined, member), 'codex.proxy.catalog.blockingMemberHint');
  }
  for (const member of ['DIRECT', 'PASS', 'PASS-RULE', 'COMPATIBLE']) {
    assert.equal(service.isCatalogBlockingMember(member), false);
    assert.equal(service.catalogUnsupportedKey(undefined, member), 'codex.proxy.catalog.reasonBuiltin');
  }
  assert.equal(service.isCatalogBlockingMember('REJECT secret'), false);
});

test('group browsing includes direct leaves only and keeps child groups separate', () => {
  const { service } = harness(async () => ({}));
  const data = { ...source, nodes: [...source.nodes, { ...node, id: 'other', name: 'Outside' }, { ...node, id: 'bad', name: 'Bad', supported: false }], groups: [
    group('root', 'Proxy', 'select', ['Auto', 'Tokyo', 'Bad', 'DIRECT']), group('auto', 'Auto', 'fallback', ['Proxy', 'Tokyo']),
    group('elsewhere', 'Elsewhere', 'select', ['Outside']),
  ] };
  assert.deepEqual(Array.from(service.catalogGroupNodes(data, 'root')), ['node']);
  assert.deepEqual(Array.from(service.catalogGroupNodes(data, 'missing')), []);
  assert.deepEqual(Array.from(service.catalogGroupNodes(data, 'root', true)), ['node', 'bad']);
});

test('latency candidates stay within the clicked group subtree, deduplicate cycles and skip unsupported nodes', () => {
  const { service } = harness(async () => ({}));
  const data = { ...source, nodes: [node, { ...node, id: 'osaka', name: 'Osaka' }, { ...node, id: 'us', name: 'US' }, { ...node, id: 'bad', name: 'Bad', supported: false }], groups: [
    group('global', 'Global', 'select', ['Japan', 'America']),
    group('japan', 'Japan', 'url-test', ['Tokyo', 'Nested', 'Bad', 'REJECT']),
    group('nested', 'Nested', 'fallback', ['Osaka', 'Tokyo', 'Japan', 'Missing']),
    group('america', 'America', 'url-test', ['US']),
  ] };
  assert.deepEqual(Array.from(service.catalogLatencyCandidates(data, 'japan')), ['node', 'osaka']);
  assert.deepEqual(Array.from(service.catalogLatencyCandidates(data, 'america')), ['us']);
  assert.deepEqual(Array.from(service.catalogLatencyCandidates(data, 'osaka')), ['osaka']);
  for (const id of ['', 'missing', 'bad', 'REJECT']) assert.deepEqual(Array.from(service.catalogLatencyCandidates(data, id)), []);
});

test('latency IPC carries only node identity and source revision', async () => {
  const calls: unknown[] = [];
  const { service } = harness(async (command, args) => { calls.push({ command, args }); return {}; });
  await service.measureProxyLatency('request', 'source', 'node', 'revision');
  assert.equal(JSON.stringify(calls), JSON.stringify([{ command: 'codex_proxy_catalog_latency', args: { requestId: 'request', sourceId: 'source', nodeId: 'node', revision: 'revision' } }]));
  assert.equal(service.catalogErrorKey('IMPORT_AMBIGUOUS'), 'codex.proxy.catalog.ambiguous');
  assert.equal(service.catalogErrorKey('IMPORT_INVALID: user:password'), 'codex.proxy.catalog.failed');
});

test('group latency and certificate permission carry only scoped identities and revision', async () => {
  const calls: unknown[] = [];
  const { service } = harness(async (command, args) => { calls.push({ command, args }); return {}; });
  await service.measureProxyLatency('request', 'source', 'node', 'revision', 'region');
  await service.setProxyGroupInsecure('source', 'region', 'revision', true);
  assert.deepEqual(JSON.parse(JSON.stringify(calls)), [
    { command: 'codex_proxy_catalog_latency', args: { requestId: 'request', sourceId: 'source', nodeId: 'node', revision: 'revision', groupId: 'region' } },
    { command: 'codex_proxy_catalog_group_insecure', args: { sourceId: 'source', groupId: 'region', revision: 'revision', enabled: true } },
  ]);
});


test('regional groups retain direct members without leaking the global auto group', () => {
  const { service } = harness(async () => ({}));
  const data = {...source,nodes:[{...node,id:'us',name:'US'}, {...node,id:'jp',name:'JP'}],groups:[group('us-group','US group','select',['US','Global']),group('global','Global','url-test',['US','JP'])]};
  assert.deepEqual(Array.from(service.catalogGroupNodes(data,'us-group')),['us']);
  assert.deepEqual(Array.from(service.catalogGroupNodes(data,'global')),['us','jp']);
});

test('compatibility IPC is explicit, revision-scoped and errors stay sanitized', async () => {
  const calls:any[]=[];const {service}=harness(async(command,args)=>{calls.push({command,args});return {sources:[]};});
  await service.setProxyNodeInsecure('s','n','r',true);
  assert.equal(calls[0].args.enabled,true);assert.equal(calls[0].args.revision,'r');assert.equal(calls[0].args.nodeId,'n');
  assert.equal(service.catalogErrorKey('PROXY_ECH_DNS'),'codex.proxy.catalog.errorEchDns');
  assert.equal(service.catalogErrorKey('PROXY_ECH_DNS private-url'),'codex.proxy.catalog.failed');
});


test('native automatic groups keep their distinct labels and precise unavailable reasons', () => {
  const { service } = harness(async () => ({}));
  const labels = { select: 'manualGroup', 'url-test': 'latencyGroup', fallback: 'fallbackGroup', 'load-balance': 'loadBalanceGroup' };
  for (const [kind, label] of Object.entries(labels)) {
    assert.equal(service.catalogGroupKindKey(kind), `codex.proxy.catalog.${label}`);
    assert.equal(service.catalogUnsupportedKey({ kind, error: 'SUBSCRIPTION_UNSUPPORTED' }), 'codex.proxy.catalog.reasonOptions');
    assert.equal(service.catalogUnsupportedKey({ kind, error: 'SUBSCRIPTION_GROUP_UNAVAILABLE' }), 'codex.proxy.catalog.reasonMembers');
  }
  for (const kind of ['relay', 'https://secret.invalid', '__proto__', 'constructor']) {
    assert.equal(service.catalogGroupKindKey(kind), 'codex.proxy.catalog.reasonStrategy');
  }
});

test('fallback and load balancing preserve reachable nested manual choices', () => {
  const { service } = harness(async () => ({}));
  for (const kind of ['fallback', 'load-balance']) {
    const data = { ...source, groups: [group('automatic', 'Automatic', kind, ['Nested', 'Tokyo']), ...source.groups] };
    const choices = Array.from(service.catalogSelectors(data, 'automatic', {}), (entry: any) => entry.id);
    assert.deepEqual(choices, ['nested']);
  }
});

test('binding forwards display group context separately from runtime selections', async () => {
  let args: unknown;
  const { service } = harness(async (_command, input) => { args = input; return {}; });
  await service.bindProxyCatalog('account', 'source', 'node', {}, 'usa');
  assert.equal(JSON.stringify(args), JSON.stringify({ accountId: 'account', sourceId: 'source', itemId: 'node', selections: {}, groupId: 'usa' }));
});
