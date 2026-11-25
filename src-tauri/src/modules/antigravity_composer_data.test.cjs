const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { join } = require('node:path');
const { test } = require('node:test');
const { runInNewContext, runInThisContext } = require('node:vm');

const { quotaWindows, sessionMetrics } = runInThisContext(readFileSync(
  join(__dirname, 'antigravity_composer_data.js'), 'utf8',
));
const formatContextUsage = runInThisContext(readFileSync(join(__dirname, 'composer_context_usage.js'), 'utf8'));

function mountComposer(metadata = null) {
  const focused = [];
  const createNode = (left = 0, right = 100) => {
    const bounds = { left, right, top: 700, height: 30 };
    const attributes = new Map();
    return {
      children: [], style: {}, listeners: new Map(),
      getAttribute: name => attributes.get(name) ?? null,
      setAttribute: (name, value) => attributes.set(name, value),
      removeAttribute: name => attributes.delete(name),
      getBoundingClientRect: () => bounds,
      getClientRects: () => [bounds],
      closest: () => null,
      querySelectorAll: () => [],
      append(...children) { this.children.push(...children); },
      replaceChildren(...children) { this.children = children; },
      addEventListener(name, callback) { this.listeners.set(name, callback); },
      focus() { focused.push(this); },
    };
  };
  const picker = createNode(30, 175);
  picker.innerText = 'Gemini 4 Hydrogen\nHigh';
  const conversation = createNode();
  conversation.setAttribute('data-cascade-id', 'test-conversation');
  const footer = createNode(20, 800);
  footer.parentElement = { querySelectorAll: () => [conversation] };
  footer.querySelector = () => picker;
  const left = createNode(20, 180);
  left.querySelectorAll = () => [picker];
  footer.append(left, createNode(620, 800));
  const body = createNode();
  const listeners = new Map();
  const window = { addEventListener: (name, callback) => listeners.set(name, callback) };
  window.top = window;
  const script = runInNewContext(readFileSync(join(__dirname, 'antigravity_composer_status.js'), 'utf8'), {
    window, location: { protocol: 'https:', hostname: '127.0.0.1' },
    document: {
      body, head: createNode(), querySelector: () => footer, querySelectorAll: () => [footer],
      createElement: () => createNode(),
    },
    localStorage: { getItem: () => metadata ? 'fixture-token' : null },
    fetch: async url => {
      const method = url.split('/').at(-1);
      const body = {
        GetCascadeTrajectory: { numTotalGeneratorMetadata: metadata?.length },
        GetCascadeTrajectoryGeneratorMetadata: { generatorMetadata: metadata },
      }[method] ?? {};
      return { ok: true, json: async () => body };
    },
    AbortSignal,
    getComputedStyle: () => ({ visibility: 'visible' }),
    ResizeObserver: class { disconnect() {} observe() {} },
    MutationObserver: class { observe() {} },
    requestAnimationFrame: callback => callback(), innerWidth: 800, innerHeight: 750,
  });
  const summary = { groups: [
    { modelIds: ['Gemini 4 Hydrogen'], buckets: [{ bucketId: 'gemini-5h', remainingFraction: 0.99 }] },
    { modelIds: ['Gemini 4 Neon'], buckets: [{ bucketId: 'gemini-5h', remainingFraction: 0.42 }] },
  ] };
  const settings = {
    missing: '—', fiveHour: '5h', noData: 'No data', token: 'Token',
    responseSpeed: 'Response speed', overallSpeed: 'Speed incl. thinking', streamingSpeed: 'Streaming speed',
  };
  const data = { quotaWindows: (_summary, status, model) => quotaWindows(summary, status, model), sessionMetrics };

  assert.equal(script(settings, data, [], formatContextUsage), true);
  const [capsule, details] = body.children;
  return {
    picker, capsule, details, listeners, focused,
    refresh: () => window.__cockpitAntigravityComposer.refresh(settings),
  };
}

test('the Antigravity footer uses the visible model label and stays visible while the label is unavailable', () => {
  const { picker, capsule, details, refresh } = mountComposer();
  const metricKeys = () => Array.from(capsule.children, metric => metric.getAttribute('data-cockpit-metric-key'));
  const quotaValue = () => capsule.children[0].children.at(-1).textContent;
  assert.equal(capsule.hidden, false);
  assert.deepEqual(metricKeys(), ['quota', 'context', 'speed']);
  assert.equal(quotaValue(), '99%');

  picker.setAttribute('data-model-label', 'Gemini 4 Neon');
  refresh();
  assert.equal(quotaValue(), '42%');

  picker.removeAttribute('data-model-label');
  picker.innerText = '  ';
  refresh();
  assert.equal(capsule.hidden, false);
  assert.deepEqual(metricKeys(), ['context', 'speed']);
  capsule.listeners.get('click')();
  assert.equal(details.hidden, false);
});

test('outside pointer presses close the Antigravity quota popover without moving focus', () => {
  const { capsule, details, listeners, focused } = mountComposer();
  capsule.listeners.get('click')();
  assert.equal(details.hidden, false);
  const focusBeforePresses = focused.length;
  const press = nodes => listeners.get('pointerdown')?.({ composedPath: () => nodes });

  press([details.children[0], details]);
  press([capsule.children[0], capsule]);
  assert.equal(details.hidden, false);
  press([{}]);
  assert.equal(details.hidden, true);
  assert.equal(capsule.getAttribute('aria-expanded'), 'false');
  assert.equal(focused.length, focusBeforePresses);
});

test('context rings distinguish unused and unreported context and retain counts in the tooltip', () => {
  const labels = { context: '上下文', token: 'Token', noData: '暂无数据' };
  const amount = `${(315000).toLocaleString()} / ${(490000).toLocaleString()}`;
  assert.deepEqual(formatContextUsage(315000, 490000, labels), {
    percent: '64%', label: '上下文 64%', amount, tooltip: `上下文 64%\n${amount} Token`,
  });
  assert.equal(formatContextUsage(315000, 490000, labels, true).tooltip, `上下文 64%\n≈ ${amount} Token`);
  assert.equal(formatContextUsage(0, 490000, labels).percent, '0%');
  assert.equal(formatContextUsage(500000, 490000, labels).percent, '100%');
  const unreported = { percent: null, label: '上下文 · 暂无数据', amount: null, tooltip: '上下文 · 暂无数据' };
  assert.deepEqual(formatContextUsage(undefined, 490000, labels), unreported);
  assert.deepEqual(formatContextUsage(0, 0, labels), unreported);
});

test('composer quotas follow the selected model’s reported pool and retain real zeroes', () => {
  const summary = { response: { groups: [
    { modelIds: ['MODEL_PLACEHOLDER_H'], buckets: [
      { bucketId: 'gemini-weekly', remainingFraction: 0 },
      { bucketId: 'gemini-5h', remainingFraction: 0.99 },
    ] },
    { description: 'Models within this group: Claude Opus 5.5',
      buckets: [{ bucketId: '3p-5h', remainingFraction: 0.49 }] },
  ] } };
  const status = { userStatus: { cascadeModelConfigData: { clientModelConfigs: [
    { label: 'Gemini 4 Hydrogen', modelOrAlias: { model: 'MODEL_PLACEHOLDER_H' } },
  ] } } };

  assert.deepEqual(quotaWindows(summary, status, '4 Hydrogen High · 1M').map(({ period, percent }) => [period, percent]),
    [['5h', 99], ['weekly', 0]]);
  assert.deepEqual(quotaWindows(summary, status, 'Claude Opus 5.5 Medium').map(({ percent }) => percent), [49]);
  assert.deepEqual(quotaWindows(summary, status, 'Gemini 4 Argon'), []);
});

test('composer quotas skip the retired Argon bucket and fall back to the shared Gemini pool', () => {
  const status = { userStatus: { cascadeModelConfigData: { clientModelConfigs: [] } } };
  const summary = { response: { groups: [
    { description: 'Models within this group: Gemini 4 Carbon',
      buckets: [{ bucketId: 'gemini-carbon-wtu-weekly', remainingFraction: 0.3 }] },
    { description: 'Models within this group: Gemini 4 Neon',
      buckets: [{ bucketId: 'gemini-5h', remainingFraction: 0.71 },
        { bucketId: 'gemini-weekly', remainingFraction: 0.83 }] },
    { description: 'Models within this group: Gemini 4 Argon',
      buckets: [{ bucketId: 'gemini-argon-wtu-weekly', remainingFraction: 0 }] },
  ] } };
  assert.deepEqual(quotaWindows(summary, status, 'Gemini 4 Argon').map(({ period, percent }) => [period, percent]),
    [['5h', 71], ['weekly', 83]]);
});

test('composer quotas dedupe a shared group listing Argon from a retired Argon group', () => {
  const status = { userStatus: { cascadeModelConfigData: { clientModelConfigs: [] } } };
  const summary = { response: { groups: [
    { description: 'Models within this group: Gemini 4 Neon, Gemini 4 Argon',
      buckets: [{ bucketId: 'gemini-5h', remainingFraction: 0.71 }] },
    { description: 'Models within this group: Gemini 4 Argon',
      buckets: [{ bucketId: 'gemini-argon-wtu-weekly', remainingFraction: 0 }] },
  ] } };
  assert.deepEqual(quotaWindows(summary, status, 'Gemini 4 Argon').map(({ period, percent }) => [period, percent]),
    [['5h', 71]]);
});

test('composer speed ignores failed, partial, and other-model samples', () => {
  const chatModel = { modelDisplayName: 'Gemini 4 Hydrogen', timeToFirstToken: '1.250s',
    streamingDuration: '2s', usage: { outputTokens: 184, responseOutputTokens: 156 },
    chatStartMetadata: { contextWindowMetadata: { estimatedTokensUsed: '124000', maxContextTokens: '200000' } } };
  const rows = [
    { chatModel },
    { chatModel: { ...chatModel, modelDisplayName: 'Claude Opus 5.5', usage: { outputTokens: 999 } } },
    { chatModel: { ...chatModel, error: 'server failed' } },
    { isStreaming: true, chatModel: { ...chatModel, usage: { outputTokens: 110 } } },
  ];

  assert.deepEqual(sessionMetrics(rows, null, '4 Hydrogen High'), {
    context: { used: 124000, max: 200000 },
    speed: { rate: 48, overallRate: 184 / 3.25, streamingRate: 92,
      first: 1.25, outputTokens: 156, elapsedSeconds: 3.25 },
    hasCompletedSample: true,
  });
  assert.equal(sessionMetrics([{ chatModel: { ...chatModel, streamingDuration: undefined } }], null, '4 Hydrogen').speed, null);
});

const measuredModel = {
  modelDisplayName: 'Gemini 4 Hydrogen', timeToFirstToken: '2.816600167s', streamingDuration: '0.653278667s',
  usage: { outputTokens: '328', thinkingOutputTokens: '107', responseOutputTokens: '221' },
};

test('composer derives three speeds from the same completed model request', () => {
  const model = measuredModel;
  for (const usage of [model.usage, { outputTokens: '328', thinkingOutputTokens: '107' },
    { responseOutputTokens: '221', thinkingOutputTokens: '107' }]) {
    const { speed } = sessionMetrics([{ chatModel: { ...model, usage } }], null, 'Gemini 4 Hydrogen High');
    assert.equal(speed.outputTokens, 221);
    assert.equal(speed.rate.toFixed(1), '63.7');
    assert.equal(speed.overallRate.toFixed(1), '94.5');
    assert.equal(speed.streamingRate.toFixed(1), '502.1');
    assert.equal(speed.elapsedSeconds.toFixed(9), '3.469878834');
  }
  const { speed } = sessionMetrics([
    { chatModel: model }, { chatModel: { ...model, usage: { responseOutputTokens: '221' } } },
  ], null, 'Gemini 4 Hydrogen High');
  assert.equal(speed.rate.toFixed(1), '63.7');
  assert.equal(speed.overallRate, null);
  assert.equal(speed.streamingRate, null);

  for (const incomplete of [
    { ...model, timeToFirstToken: undefined },
    { ...model, streamingDuration: '0.039647417s' },
    { ...model, usage: { outputTokens: 328, thinkingOutputTokens: 229, responseOutputTokens: 99 } },
  ]) {
    const metrics = sessionMetrics([{ chatModel: model }, { chatModel: incomplete }], null, 'Gemini 4 Hydrogen High');
    assert.equal(metrics.speed, null);
    assert.equal(metrics.hasCompletedSample, true);
  }
});

test('the Antigravity capsule displays response speed and opens all three readings', async () => {
  const { capsule, details } = mountComposer([{ chatModel: measuredModel }]);
  await new Promise(resolve => setImmediate(resolve));
  const speeds = [63.7, 94.5, 502.1].map(rate => rate.toLocaleString() + ' tok/s');
  const metric = capsule.children.find(child => child.getAttribute('data-cockpit-metric-key') === 'speed');
  assert.equal(metric.children.at(-1).textContent, speeds[0]);
  assert.equal(details.hidden, true);

  capsule.listeners.get('click')();
  assert.equal(details.hidden, false);
  const rows = details.children[1].children;
  const readings = Object.fromEntries(rows.flatMap((label, index) =>
    index % 2 === 0 ? [[label.textContent, rows[index + 1].textContent]] : []));
  assert.deepEqual([readings['Response speed'], readings['Speed incl. thinking'], readings['Streaming speed']], speeds);
});
