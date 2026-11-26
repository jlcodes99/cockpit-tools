const assert = require('node:assert/strict');
const test = require('node:test');
const fs = require('node:fs');
const vm = require('node:vm');
const path = require('node:path');
const script = fs.readFileSync(path.join(__dirname, 'predictions_rspack.js'), 'utf8');

// Synthetic contracts: retain the client call boundaries without bundling its source.
const fixtures = {
  559161: 'const gate=e(g.JW,"4290031608")&&(0,f.CN)(e,(0,u.W)().enabled)&&null!=s.appServices.cumberbatch;',
  563813: 'function C(e){let {hostId:h,pluginHostId:k,roots:c}=e,A=(0,l.gh)(b.PR),I=(0,l.Ul)(w.bz),T=(0,l.Ul)(w.Jn),j=g.appServices.cumberbatch;return j.requestUserMessagePrediction({...T,generationTrigger:"catch_up"});}',
  24191: 'async function run(e,t,target,alive){let service=B.appServices?.cumberbatch;if((0,G.ie)(e,"4290031608"))return (0,G.ie)(e,"4290031608");}const message="Cumberbatch background prediction did not finish";function managerModule(e,t,n){function p(scope,host){return scope.manager(host)}let d=null;n.d(t,{V:()=>p},{p:d})}',
  572868: 'function settings(){let scope=(0,g.gh)(L.u);let f=(0,G.Zj)("4290031608"),service=Z.appServices.cumberbatch;return ["cumberbatch-availability",service];}',
};
function setup() {
  const appended = [];
  const root = { URL, Blob,
    HTMLScriptElement: { supports: () => true }, addEventListener() {},
    document: { documentElement: null,
      createElement: () => ({ appendChild: node => appended.push(node), relList: { supports: () => true } }),
      appendChild(node) { this.documentElement = node; },
    },
  };
  root.window = root;
  vm.runInNewContext(script, root);
  const sources = new Map(Object.entries(fixtures).map(([id, body]) =>
    [`app://-/assets/async/${id}.abcdef1234.js`, `export const __rspack_esm_id=${id};${body}`]));
  sources.set('app://-/assets/123.abcdef1234.js',
    'r.u=e=>"assets/async/"+e+"."+({' + Object.keys(fixtures).map(id => `${id}:"abcdef1234"`).join(',') + '})[e]+".js";r.miniCssF=e=>({24191:"other-css"})[e];');
  return { root, appended, sources, api: root.__cockpitPredictionRspack,
    html: '<script src="/assets/123.abcdef1234.js"></script>',
    read: url => { if (!sources.has(url)) throw Error('missing-chunk'); return sources.get(url); } };
}

test('Rspack prepares every native companion before publishing a single import map', () => {
  const h = setup();
  const prepared = h.api.prepare(h.html, h.read);
  assert.equal(prepared.ok, true, prepared.reason);
  assert.equal(prepared.modules.length, 4);
  assert.equal(h.root.document.documentElement, null);
  assert.equal(h.api.installModules(prepared.modules), true);
  const imports = JSON.parse(h.appended[0].textContent).imports;
  assert.equal(Object.keys(imports).length, 4);
  assert.ok(Object.keys(imports).every(url => url.endsWith('.abcdef1234.js')));
  assert.deepEqual(h.appended.slice(1).map(node => [node.rel, node.href]),
    Object.values(imports).map(url => ['modulepreload', url]));
  Object.values(imports).forEach(url => URL.revokeObjectURL(url));
});

test('an unavailable companion leaves the document and prediction readiness untouched', () => {
  const h = setup();
  h.sources.delete('app://-/assets/async/572868.abcdef1234.js');
  const prepared = h.api.prepare(h.html, h.read);
  assert.equal(prepared.ok, false);
  assert.equal(h.root.document.documentElement, null);
});

test('the 26.1111 desktop prepares its four prediction chunks', () => {
  const h = setup();
  const sources = {
    538993: fixtures[559161],
    899469: 'function C({controller:e,hostId:h,pluginHostId:k,roots:c}){let A=(0,l.gh)(b.PR),I=(0,l.Ul)(w.bz),T=(0,l.Ul)(w.Jn),j=g.appServices.cumberbatch;return j.requestUserMessagePrediction({...T,generationTrigger:"catch_up"});}',
    907538: fixtures[24191].split('function managerModule')[0]
      .replace('async function run(', 'const live=async function(')
      + 'const modules={Dgwo(e,t,n){let metadata=true;n.d(t,{},{V:function(scope,host){return [scope,host]}})}};',
    184348: fixtures[572868],
  };
  const ids = Object.keys(sources);
  h.sources.set('app://-/assets/123.abcdef1234.js',
    'r.u=e=>"assets/async/"+e+"."+({' + ids.map(id => id + ':"abcdef1234"').join(',') + '})[e]+".js"');
  for (const id of ids) {
    h.sources.set(`app://-/assets/async/${id}.abcdef1234.js`, `export const __rspack_esm_ids=[${id}];${sources[id]}`);
  }
  const result = h.api.prepare(h.html, h.read);
  assert.equal(result.ok, true, result.reason);
  assert.equal(result.modules.length, 4);
  assert.match(result.modules[0].source, /null!=s\.appServices\.cumberbatch\|\|/);
  assert.match(result.modules[1].source, /serviceFor\(A,h,g\.appServices\.cumberbatch\)/);
  assert.match(result.modules[2].source, /runtime\.register\(/);
});

test('a changed native contract cannot partially replace an otherwise valid source', () => {
  const h = setup();
  const original = 'export const __rspack_esm_id=559161;' + fixtures[559161].replace('4290031608', 'different-gate');
  const result = h.api.transform(original, 'initial');
  assert.equal(result.ok, false);
  assert.equal(result.source, original);
});

test('Codex 26.1120 prepares wrapped module factories without changing older layouts', () => {
  const h = setup();
  const current = {
    173367: fixtures[559161],
    779071: fixtures[563813].replace('function C(e){let {hostId:h,pluginHostId:k,roots:c}=e,',
      'function C({controller:e,hostId:h,pluginHostId:k,roots:c}){let '),
    49792: fixtures[24191].split('function managerModule')[0]
      + 'const modules={Dgwo:(function(e,t,n){let marker=true;n.d(t,{},{V:function(scope,host){return [scope,host]}})})};',
    25051: fixtures[572868],
  };
  h.sources.set('app://-/assets/123.abcdef1234.js',
    'r.u=e=>"assets/async/"+e+"."+({' + Object.keys(current).map(id => `${id}:"abcdef1234"`).join(',') + '})[e]+".js";');
  for (const [id, body] of Object.entries(current)) {
    h.sources.set(`app://-/assets/async/${id}.abcdef1234.js`, `export const __rspack_esm_ids=[${id}];${body}`);
  }
  const result = h.api.prepare(h.html, h.read);
  assert.equal(result.ok, true, result.reason);
  assert.equal(result.modules.length, 4);
  const host = result.modules.find(module => module.url.includes('/49792.'));
  let bindings;
  const root = { __cockpitNativePredictions: { runtime: { register(value) { bindings = value; } } } };
  const factories = vm.runInNewContext(host.source.replace('export const', 'const') + 'modules;', {window:root});
  const exports = {};
  const require = name => name === 'RxZZ' ? {CN: (get, key) => get(key)} : {W: () => ({enabled:'enabled'})};
  require.d = (target, _, values) => Object.assign(target, values);
  factories.Dgwo(null, exports, require);
  assert.equal(bindings.managerForHost('scope', 'local')[1], 'local');
  assert.equal(bindings.nativeEnabled({get:key=>key==='enabled'}), true);
  const broken = h.api.prepare(h.html, url => h.read(url).replace('Dgwo:', 'Unknown:'));
  assert.equal(broken.ok, false);
});

test('Codex 26.1111 validates predictions independently from Fast for OAuth API Service', async () => {
  const h = setup();
  const host = fixtures[24191].split('function managerModule')[0]
    .replace('async function run(', 'async function(').replace('}const message', '};const message');
  const current = {
    538993: fixtures[559161],
    899469: fixtures[563813].replace('function C(e){let {hostId:h,pluginHostId:k,roots:c}=e,',
      'function C({controller:e,canShowPrediction:t,hostId:h,pluginHostId:k,roots:c}){let '),
    907538: 'const native=' + host
      + 'const modules={Dgwo(e,t,n){let marker=true;n.d(t,{},{V:function(scope,host){return [scope,host]}})}};',
    184348: fixtures[572868],
  };
  h.sources.clear();
  for (const [id, body] of Object.entries(current)) {
    h.sources.set(`app://-/assets/async/${id}.abcdef1234.js`, `export const __rspack_esm_id=${id};${body}`);
  }
  const entry = 'app://-/assets/123.abcdef1234.js';
  h.sources.set(entry, 'r.u=e=>"assets/async/"+e+"."+({'
    + Object.keys(current).map(id => `${id}:"abcdef1234"`).join(',') + '})[e]+".js";');
  h.root.__cockpitEnhancements = { nativeFast:false, nativePredictions:true, apiKeyAuthVerified:false, apiService:true };
  h.root.document.scripts = [{src:entry}];
  h.root.document.querySelectorAll = () => [];
  h.root.AbortSignal = AbortSignal;
  h.root.fetch = async url => ({ok:h.sources.has(url), text:async () => h.read(url)});
  vm.runInNewContext(fs.readFileSync(path.join(__dirname, 'service_rspack.js'), 'utf8'), h.root);

  const verify = h.root.__cockpitServiceRspack.verifyRequestedDocument;
  assert.equal((await verify()).ok, true);
  const prepared = h.api.prepare(h.html, h.read);
  assert.equal(prepared.ok, true, prepared.reason);
  assert.equal(prepared.modules.length, 4);
  const composer = prepared.modules.find(module => module.url.includes('/899469.'));
  assert.match(composer.source, /serviceFor\(A,h,g\.appServices\.cumberbatch\)/);

  const composerUrl = composer.url;
  h.sources.set(composerUrl, h.read(composerUrl).replace('roots:c', 'changed:c'));
  const unknown = await verify();
  assert.equal(unknown.ok, false);
  assert.equal(unknown.reason, 'composer-contract');
  assert.equal(h.root.document.documentElement, null);
});

test('the current Mac and Windows host factories bind the actual exported RPC lazily', () => {
  const h = setup();
  for (const [exportsName, loader] of [['t', 'n'], ['o', 'a']]) {
    const body = fixtures[24191].split('function managerModule')[0]
      .replace('async function run(', 'async function(').replace('}const message', '};const message');
    const source = 'export const __rspack_esm_ids=[182735];'
      + 'const native=' + body
      + 'const modules={Dgwo(e,' + exportsName + ',' + loader + '){let marker=true;'
      + loader + '.d(' + exportsName + ',{},{V:function(scope,host){return [scope,host]}})}};';
    const result = h.api.transform(source, 'host', 182735);
    assert.equal(result.ok, true, result.reason);
    let bindings;
    const root = { __cockpitNativePredictions: { runtime: { register: value => { bindings = value; } } } };
    const factories = vm.runInNewContext(result.source.replace('export const', 'const') + 'modules;', {window:root});
    const exports = {};
    const require = name => name === 'RxZZ' ? {CN: (get, key) => get(key)} : {W:()=>({enabled:'enabled'})};
    require.d = (target, _, values) => Object.assign(target, values);
    factories.Dgwo(null, exports, require);
    assert.equal(bindings.nativeEnabled({get:key=>key==='enabled'}), true);
    assert.equal(bindings.managerForHost('scope','local')[1], 'local');
  }
});
