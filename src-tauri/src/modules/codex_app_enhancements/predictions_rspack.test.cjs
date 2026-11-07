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
  const ready = [], appended = [];
  const root = { URL, Blob, __cockpitEnhancements: {},
    __cockpitNativePredictions: { setReady: value => ready.push(value), runtime: { dispose() {} } },
    HTMLScriptElement: { supports: () => true }, addEventListener() {},
    document: { documentElement: null,
      createElement: () => ({ appendChild: node => appended.push(node) }),
      appendChild(node) { this.documentElement = node; },
    },
  };
  root.window = root;
  vm.runInNewContext(script, root);
  const sources = new Map(Object.entries(fixtures).map(([id, body]) =>
    [`app://-/assets/async/${id}.abcdef1234.js`, `export const __rspack_esm_id=${id};${body}`]));
  sources.set('app://-/assets/123.abcdef1234.js',
    'r.u=e=>"assets/async/"+e+"."+({' + Object.keys(fixtures).map(id => `${id}:"abcdef1234"`).join(',') + '})[e]+".js";r.miniCssF=e=>({24191:"other-css"})[e];');
  return { root, ready, appended, sources, api: root.__cockpitPredictionRspack,
    html: '<script src="/assets/123.abcdef1234.js"></script>',
    read: url => { if (!sources.has(url)) throw Error('missing-chunk'); return sources.get(url); } };
}

test('Rspack prepares every native companion before publishing a single import map', () => {
  const h = setup();
  const prepared = h.api.prepare(h.html, h.read);
  assert.equal(prepared.ok, true, prepared.reason);
  assert.equal(prepared.modules.length, 4);
  assert.deepEqual(h.ready, [], 'source discovery alone must not enable predictions');
  assert.equal(h.api.install(prepared), true);
  assert.deepEqual(h.ready, [true]);
  const imports = JSON.parse(h.appended[0].textContent).imports;
  assert.equal(Object.keys(imports).length, 4);
  assert.ok(Object.keys(imports).every(url => url.endsWith('.abcdef1234.js')));
  Object.values(imports).forEach(url => URL.revokeObjectURL(url));
});

test('an unavailable companion leaves the document and prediction readiness untouched', () => {
  const h = setup();
  h.sources.delete('app://-/assets/async/572868.abcdef1234.js');
  const prepared = h.api.prepare(h.html, h.read);
  assert.equal(prepared.ok, false);
  assert.equal(h.api.install(prepared), false);
  assert.equal(h.root.document.documentElement, null);
  assert.deepEqual(h.ready, []);
});

test('a changed native contract cannot partially replace an otherwise valid source', () => {
  const h = setup();
  const original = 'export const __rspack_esm_id=559161;' + fixtures[559161].replace('4290031608', 'different-gate');
  const result = h.api.transform(original, 'initial');
  assert.equal(result.ok, false);
  assert.equal(result.source, original);
});
