const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const quotaSelection = vm.runInNewContext(
  fs.readFileSync(path.join(__dirname, 't3code_app_injection_selection.js'), 'utf8'),
  { labels: { gatewayName: 'Codex API Service', credits: '积分', weekly: '周', unknownPlan: '未知', accountMismatch: '账号不一致', identityPending: '待确认' } },
);
const { readSelection, relevantWindows, quotaSource, capsuleWindows, centeredCapsulePosition, displayPlan, providerStatus } = quotaSelection;
const window = (id, remainingPercent) => ({ id, label: id, remainingPercent });
const rows = list => Array.from(list, item => [item.id, item.remainingPercent]);

test('resolves quota and refresh for the selected Cockpit ACP provider instance', () => {
  for (const driver of ['workbuddy', 'workbuddy_ai', 'codebuddy', 'codebuddy_cn', 'minimaxcode', 'grok']) {
    const instanceId = 'provider-2';
    const provider = {
      instanceId, driver, boundAccountId: 'account-2', boundAccountEmail: 'b@example.test',
      boundAccountPlan: 'PRO', windows: [{ id: 'credits', label: 'Credits', remainingAmount: 42 }],
    };
    const state = { environmentId: 'local', workspace: { providerInstances: [
      { ...provider, instanceId: 'provider-1', boundAccountId: 'account-1', windows: [window('credits', 90)] },
      provider,
    ] } };
    const pickerProps = {
      isComposerOwned: true, activeInstanceId: instanceId, environmentId: 'local', model: 'deepseek-v4.1-flash',
      instanceEntries: [{ instanceId, driverKind: driver === 'grok' ? 'grok' : 'acpRegistry' }],
    };
    const picker = { __reactFiber$fixture: { memoizedProps: pickerProps } };

    const selection = readSelection(picker, state);
    assert.equal(selection.driver, driver);
    assert.equal(selection.supportsQuota, true);
    const source = quotaSource(state, selection);
    assert.equal(source.refreshAccountId, 'account-2');
    assert.equal(source.account, 'b@example.test');
    assert.equal(capsuleWindows(source)[0].text, '积分 42');

    if (driver !== 'grok') {
      assert.equal(readSelection(picker, { environmentId: 'remote', workspace: state.workspace }).supportsQuota, false);
      assert.equal(readSelection(picker, { ...state, workspace: { providerInstances: [] } }).supportsQuota, false);
    }
  }
});

test('writes only the requested T3 desktop preference and notifies its native stores', async () => {
  let settings = { language: 'en', notificationMode: 'sound', customPreference: true };
  const events = [];
  const edit = vm.runInNewContext(fs.readFileSync(path.join(__dirname, 't3code_native_settings.js'), 'utf8'), {
    window: {
      desktopBridge: {
        getClientSettings: async () => settings,
        setClientSettings: async value => { settings = value; },
      },
      dispatchEvent: event => events.push(event.key),
    },
    StorageEvent: class { constructor(_name, event) { this.key = event.key; } },
  });
  const changedLanguage = await edit({ field: 'language', value: 'zh' });
  assert.equal(changedLanguage.language, 'zh');
  assert.equal(settings.notificationMode, 'sound');
  assert.equal(settings.customPreference, true);
  assert.equal(changedLanguage.customPreference, undefined);
  const changedNotifications = await edit({ field: 'notificationMode', value: 'off' });
  assert.equal(changedNotifications.notificationMode, 'off');
  assert.equal(settings.language, 'zh');
  assert.deepEqual(events, ['t3code:client-settings:v1', 't3code:client-settings:v1']);
});

test('pointer presses outside the quota dialog close it without moving focus', () => {
  let focusCount = 0;
  let queryCancelCount = 0;
  const capsule = { focus: () => focusCount++ };
  const details = {};
  const root = {
    dialog: { cancel: () => queryCancelCount++, reset() {} },
  };
  const listeners = new Map();
  const elements = new Map([
    ['[data-cockpit-t3-style]', {}],
    ['[data-cockpit-t3-capsule]', capsule],
    ['[data-cockpit-t3-details]', details],
  ]);
  const runComposer = vm.runInNewContext(
    fs.readFileSync(path.join(__dirname, 't3code_app_injection_composer.js'), 'utf8'),
    {
      window: {
        __cockpitT3CodeInjection: root,
        addEventListener: (name, fn, capture) => listeners.set(name, { fn, capture }),
      },
      document: { querySelector: selector => elements.get(selector) ?? null, querySelectorAll: () => [] },
      MutationObserver: class { observe() {} },
      quotaSelection: { readSelection: () => null },
      clerkSync: { read: () => null },
      state: {},
      labels: {},
    },
  );
  runComposer();
  root.render = () => {};
  root.quotaDetailsOpen = true;

  const { fn, capture } = listeners.get('pointerdown');
  assert.equal(capture, true);
  fn({ composedPath: () => [{}, details] });
  fn({ composedPath: () => [{}, capsule] });
  assert.equal(root.quotaDetailsOpen, true);
  fn({ composedPath: () => [{}] });
  assert.equal(root.quotaDetailsOpen, false);
  assert.equal(focusCount, 0);
  assert.equal(queryCancelCount, 1);
});

function mountChatFooter(selection, state, source) {
  const frame = { left: 20, right: 820, top: 10, bottom: 48 };
  const toolbar = { querySelectorAll: () => [], getBoundingClientRect: () => frame };
  const picker = {
    closest: selector => selector.startsWith('[inert]') ? null : toolbar,
    getClientRects: () => [frame], getBoundingClientRect: () => frame,
  };
  const capsule = {
    style: {}, children: [{ getAttribute: () => null, textContent: '' }], setAttribute() {},
    querySelectorAll: () => [], getBoundingClientRect: () => frame,
  };
  const details = {
    dataset: {}, contains: () => false, querySelectorAll: () => [],
    querySelector: selector => selector.includes('-close]') ? {} : null,
  };
  const elements = new Map([
    ['[data-cockpit-t3-style]', {}],
    ['[data-cockpit-t3-capsule]', capsule],
    ['[data-cockpit-t3-details]', details],
  ]);
  const root = {};
  let rendered;
  const labels = {
    missing: '—', noData: 'No data', context: 'Context', speed: 'Output speed', token: 'Token',
    responseSpeed: 'Response speed', overallSpeed: 'Speed incl. thinking', streamingSpeed: 'Streaming speed',
  };
  const usageLabels = { averageSpeed: 'Average speed', averageCache: 'Average cache', firstResponse: 'First response' };
  const dialog = vm.runInNewContext(
    fs.readFileSync(path.join(__dirname, 'codex_app_injection_dialog.js'), 'utf8'),
    { document: {}, window: {}, usageLabels, accountCount: 0, plans: [], missingLabel: labels.missing,
      closeLabel: 'Close', quotaEmptyLabel: 'No quota' },
  );
  const runComposer = vm.runInNewContext(
    fs.readFileSync(path.join(__dirname, 't3code_app_injection_composer.js'), 'utf8'),
    {
      window: { __cockpitT3CodeInjection: root, addEventListener() {} },
      document: {
        querySelector: selector => elements.get(selector) ?? null,
        querySelectorAll: selector => selector.includes('provider-model-picker') ? [picker] : [],
      },
      getComputedStyle: () => ({ visibility: 'visible' }),
      MutationObserver: class { observe() {} },
      ResizeObserver: class { observe() {} },
      quotaSelection: {
        ...quotaSelection,
        readSelection: () => selection,
        quotaSource: () => source,
        centeredCapsulePosition: () => ({ left: 400, top: 24 }),
      },
      createUsageDialog: () => ({
        ...dialog, place() {}, render(root, details, data) {
          rendered = data;
          dialog.render(root, details, data);
        },
      }),
      formatContextUsage: vm.runInNewContext(fs.readFileSync(path.join(__dirname, 'composer_context_usage.js'), 'utf8')),
      clerkSync: { read: () => null }, state, labels, usageLabels,
    },
  );
  runComposer();
  return {
    capsule, details,
    openDetails() {
      capsule.onclick();
      return rendered;
    },
  };
}

test('the native Grok capsule and details show quotas without session metrics or identity diagnostics', () => {
  const { capsule, details, openDetails } = mountChatFooter(
    { driver: 'grok', model: 'grok', name: 'Grok', supportsQuota: true },
    {},
    { account: 'signed-in@example.test', windows: [window('weekly', 98)], providerQuotas: [], identityProblem: '账号不一致' },
  );
  const rendered = openDetails();

  assert.doesNotMatch(capsule.innerHTML, /账号不一致/);
  assert.match(capsule.innerHTML, /98%/);
  assert.doesNotMatch(capsule.innerHTML, /context-ring|tok\/s/);
  assert.doesNotMatch(details.innerHTML, /Context|Output speed|Average speed|Average cache|First response/);
  assert.equal(rendered.direct.account, 'signed-in@example.test');
});

test('WorkBuddy only displays credits and providers without quota keep the capsule hidden', () => {
  const selection = { driver: 'workbuddy', model: 'deepseek-v4.1-flash', supportsQuota: true };
  const source = { windows: [{ id: 'credits', remainingAmount: 8640.91 }], providerQuotas: [] };
  const workbuddy = mountChatFooter(selection, {}, source);
  workbuddy.openDetails();
  assert.ok(workbuddy.capsule.innerHTML.includes((8640.91).toLocaleString()));
  assert.doesNotMatch(workbuddy.capsule.innerHTML, /context-ring|tok\/s/);
  assert.doesNotMatch(workbuddy.details.innerHTML, /Context|Output speed|Average cache|First response/);

  const other = mountChatFooter({ ...selection, supportsQuota: false }, {}, { windows: [], providerQuotas: [] });
  assert.equal(other.capsule.hidden, true);
  assert.equal(other.details.hidden, true);
});

test('Codex displays context and speed only for an active API Service source', () => {
  const conversation = { environmentId: 'local', threadId: 'thread', providerInstanceId: 'codex',
    contextProviderThreadId: 'provider-thread', driver: 'codex' };
  const model = 'gpt-5';
  const selection = { ...conversation, instanceId: conversation.providerInstanceId, model, instanceEntries: [], supportsQuota: true };
  const source = { windows: [], providerQuotas: [] };
  const gateway = { ...source, gateway: { binding: 'api-service' } };
  const thread = { model, conversation, usage: {
    nativeThreadId: 'native-thread', context: { usedTokens: 315000, maxTokens: 490000 },
  }, speed: {
    binding: 'api-service', threadId: 'native-thread', latestTokensPerSecond: 40, tokensPerSecond: 30, samples: 1,
  } };
  const state = { environmentId: 'local', thread };
  const native = mountChatFooter(selection, state, source);
  native.openDetails();
  assert.doesNotMatch(native.capsule.innerHTML, /context-ring|tok\/s/);
  assert.doesNotMatch(native.details.innerHTML, /Context|Output speed|Average speed/);

  const waiting = mountChatFooter(selection, { environmentId: 'local' }, gateway);
  assert.match(waiting.capsule.innerHTML, /data-cockpit-context-ring[^>]*data-cockpit-metric-unknown>\s*<span data-cockpit-metric-value>—/);
  assert.match(waiting.capsule.innerHTML, /—<\/span>\s*<span data-cockpit-metric-unit>tok\/s/);
  const measured = mountChatFooter(selection, state, gateway);
  measured.openDetails();
  const budget = (315000).toLocaleString() + ' / ' + (490000).toLocaleString();
  assert.match(measured.capsule.innerHTML, /data-cockpit-metric-value>64%<\/span>/);
  assert.ok(measured.capsule.innerHTML.includes(budget + ' Token'));
  assert.ok(measured.details.innerHTML.includes(budget));
  assert.match(measured.capsule.innerHTML, /data-cockpit-metric-value>40\.0<\/span>/);
  assert.match(measured.details.innerHTML, /<dt>Output speed<\/dt><dd>40\.0/);
  assert.match(measured.details.innerHTML, /<dt>Average speed<\/dt><dd>30\.0/);
});

test('the T3 Code capsule shows response speed and opens all three Antigravity readings', () => {
  const conversation = { environmentId: 'local', threadId: 'thread', providerInstanceId: 'ag',
    contextProviderThreadId: 'provider-thread', driver: 'antigravity' };
  const model = 'gemini-4-hydrogen';
  const elapsedSeconds = 3.469878834;
  const firstResponseMs = 2816.600167;
  const { capsule, details, openDetails } = mountChatFooter(
    { ...conversation, instanceId: conversation.providerInstanceId, model, instanceEntries: [], supportsQuota: true },
    { environmentId: 'local', thread: { model, conversation, usage: {
      antigravitySpeed: {
        rate: 221 / elapsedSeconds, overallRate: 328 / elapsedSeconds,
        streamingRate: 328 / 0.653278667, outputTokens: 221, elapsedSeconds, firstResponseMs,
      },
    } } },
    { windows: [], providerQuotas: [] },
  );

  const displayRate = (63.7).toLocaleString();
  const displayTime = (3.47).toLocaleString();
  assert.ok(capsule.innerHTML.includes(`data-cockpit-metric-value>${displayRate}</span>`));
  assert.ok(capsule.innerHTML.includes(`221 Token / ${displayTime} s`));
  const overallRate = (94.5).toLocaleString();
  const streamingRate = (502.1).toLocaleString();
  assert.ok(!capsule.innerHTML.includes(overallRate));
  assert.ok(!capsule.innerHTML.includes(streamingRate));
  assert.equal(details.hidden, true);
  const rendered = openDetails();
  assert.equal(details.hidden, false);
  assert.equal(rendered.direct.firstResponseMs, firstResponseMs);
  for (const [label, rate] of [
    ['Response speed', displayRate], ['Speed incl. thinking', overallRate], ['Streaming speed', streamingRate],
  ]) {
    assert.ok(details.innerHTML.includes(`<dt title="${label}">${label}</dt><dd>${rate}<small class="metric-unit"> tok/s</small></dd>`));
  }
  assert.ok(!details.innerHTML.includes('Average speed'));
});

test('reads the selected provider instance and model from the same composer picker', () => {
  const model = 'claude-sonnet-5-5-high';
  const instanceEntries = [
    { instanceId: 'claudeAgent', driverKind: 'claudeAgent' },
    { instanceId: 'antigravity-personal', driverKind: 'antigravity' },
    { instanceId: 'antigravity-work', driverKind: 'antigravity' },
  ];
  const props = {
    environmentId: 'local', activeInstanceId: 'antigravity-work', model, instanceEntries,
    isComposerOwned: true, modelOptionsByInstance: new Map(),
  };
  const picker = {
    textContent: 'Claude Sonnet 5.5 (High)',
    __reactFiber$fixture: { return: { memoizedProps: props, return: { memoizedProps: {
      routeKind: 'server', environmentId: 'local', activeThreadId: 'chat-1', activeThread: { id: 'chat-1' },
    } } } },
  };
  assert.equal(readSelection(picker).instanceId, 'antigravity-work');
  assert.equal(readSelection(picker).model, model);
  assert.equal(readSelection(picker).threadId, 'chat-1');
  picker.__reactFiber$fixture.return.return.memoizedProps.activeThread = { id: 'old-chat' };
  assert.equal(readSelection(picker).threadId, null);
  assert.equal(readSelection({ textContent: picker.textContent }), null);
  props.activeInstanceId = 'unknown';
  assert.equal(readSelection(picker), null);
});

test('selects Antigravity Gemini, Argon, Carbon and shared Claude windows before truncating', () => {
  const windows = [
    window('gemini-3.5-flash-lite', 15), window('gemini-4-carbon-high', 16),
    window('gemini-5h', 58), window('gemini-weekly', 52),
    window('gemini-argon-wtu-weekly', 22), window('gemini-carbon-wtu-weekly', 30),
    window('3p-5h', 100), window('3p-weekly', 70),
  ];
  const pick = (model, modelLabel) => rows(relevantWindows(windows, {
    driver: 'antigravity', model, modelLabel,
  }));
  assert.deepEqual(pick('gemini-3.5-flash-lite'), [['gemini-5h', 58], ['gemini-weekly', 52]]);
  assert.deepEqual(pick('gemini-4-argon-medium'), [['gemini-argon-wtu-weekly', 22]]);
  assert.deepEqual(pick('gemini-4-carbon-high'), [['gemini-carbon-wtu-weekly', 30]]);
  assert.deepEqual(pick('MODEL_PLACEHOLDER_M405', 'Claude Sonnet 5.5 (High)'), [
    ['3p-5h', 100], ['3p-weekly', 70],
  ]);
  assert.deepEqual(pick('antigravity-default'), []);
  assert.deepEqual(pick('gemini-3.5-flash-lite', 'Gemini 3.5 Flash Lite'),
    pick('gemini-3.5-flash-lite'));
  assert.deepEqual(rows(relevantWindows(windows.slice(0, 1), {
    driver: 'antigravity', model: 'gemini-3.5-flash-lite',
  })), [['gemini-3.5-flash-lite', 15]]);
});

test('switching between T3 Code environments never selects the local account by matching instance ID alone', () => {
  const instance = {
    instanceId: 'codex', driver: 'codex', boundAccountId: 'bound',
    boundAccountEmail: 'personal@example.test', boundAccountPlan: 'Pro',
    windows: [window('5h', 87)],
  };
  const selection = {
    environmentId: 'local', instanceId: 'codex', driver: 'codex', model: 'gpt-6-sol',
  };
  const state = { environmentId: 'local', workspace: { providerInstances: [instance] } };
  const local = quotaSource(state, selection);
  assert.equal(local.account, 'personal@example.test');
  assert.deepEqual(rows(local.windows), [['5h', 87]]);
  const remote = quotaSource(state, { ...selection, environmentId: 'remote', snapshot: {
    auth: { email: 'other@example.test' },
    usageLimits: { windows: [{ id: '5h', usedPercent: 23 }] },
  } });
  assert.equal(remote.account, 'other@example.test');
  assert.equal(remote.instance, null);
  assert.deepEqual(rows(remote.windows), [['5h', 77]]);
});

test('uses the selected gateway snapshot and preserves a pool quota greater than 100 percent', () => {
  const instances = [
    { instanceId: 'codex', driver: 'codex', boundAccountId: 'gateway', boundIsApiService: true },
    { instanceId: 'codex-other', driver: 'codex', boundAccountId: 'gateway', boundIsApiService: true },
  ];
  const state = {
    environmentId: 'local', workspace: { providerInstances: instances },
    gateways: [
      { instanceId: 'codex', usage: {
        origin: 'http://127.0.0.1:12345', windows: [window('5h', 200), window('weekly', null)],
        binding: 'verified-binding', plans: [{ plan: 'TEAM', count: 2 }],
      } },
      { instanceId: 'codex-other', usage: {
        origin: 'http://127.0.0.1:12345', windows: [window('5h', 85)],
      } },
    ],
  };
  const selection = { environmentId: 'local', driver: 'codex', instanceId: 'codex', model: 'gpt-6-sol',
    snapshot: { auth: { endpoint: 'http://127.0.0.1:12345' } } };
  const gateway = quotaSource(state, selection);
  assert.deepEqual(rows(gateway.windows), [['5h', 200]]);
  assert.equal(gateway.gateway.binding, 'verified-binding');
  assert.equal(gateway.gateway.plans[0].count, 2);
  assert.deepEqual(rows(quotaSource(state, { ...selection, instanceId: 'codex-other' }).windows), [['5h', 85]]);
  assert.deepEqual(rows(quotaSource(state, { ...selection, snapshot: {
    auth: { endpoint: 'http://127.0.0.1:12346' },
  } }).windows), []);
});

test('the compact row omits model names and keeps the selected quota windows', () => {
  const display = windows => capsuleWindows({ windows }).map(window => window.text).join(' · ');
  assert.equal(display([window('5h', 200), { ...window('weekly', 181), label: '7d' }]),
    '5h 200% · 周 181%');

  const gemini = { driver: 'antigravity', model: 'gemini-3.5-flash-lite' };
  assert.equal(display(relevantWindows(
    [window('gemini-5h', 58), window('gemini-weekly', 42), window('3p-weekly', 91)], gemini,
  )), '5h 58% · 周 42%');

  const claude = { driver: 'antigravity', model: 'MODEL_PLACEHOLDER_M405', modelLabel: 'Claude Sonnet 5.5' };
  assert.equal(display(relevantWindows([window('3p-5h', 100), window('3p-weekly', 80)], claude)),
    '5h 100% · 周 80%');
  assert.equal(display([window('gemini-3.5-flash-lite', 58)]), '58%');
});

test('hides binding quota when a running Codex session has a different workspace', () => {
  const state = {
    environmentId: 'local',
    workspace: { providerInstances: [{
      instanceId: 'codex', driver: 'codex', boundAccountId: 'work',
      boundAccountEmail: 'same@example.test', boundAccountWorkspaceId: 'work',
      windows: [window('5h', 20)],
    }] },
  };
  const selection = { environmentId: 'local', driver: 'codex', instanceId: 'codex', model: 'gpt-6-sol',
    snapshot: { auth: { email: 'same@example.test', workspaceId: 'personal' },
      usageLimits: { windows: [{ id: '5h', usedPercent: 30 }] } } };
  assert.deepEqual(rows(quotaSource(state, selection).windows), []);
  assert.equal(quotaSource(state, selection).identityProblem, '账号不一致');
  state.workspace.providerInstances[0].windows = [];
  assert.deepEqual(rows(quotaSource(state, selection).windows), []);
  selection.snapshot.auth.workspaceId = 'work';
  assert.deepEqual(rows(quotaSource(state, selection).windows), [['5h', 70]]);
});

test('keeps verified Grok quota when the runtime reports the matching email without a team ID', () => {
  const email = 'grok@example.test';
  const instance = { instanceId: 'grok', driver: 'grok', boundAccountId: 'grok-account',
    boundAccountEmail: email, boundAccountWorkspaceId: 'team', windows: [window('weekly', 99)] };
  const state = { environmentId: 'local', workspace: { providerInstances: [instance] } };
  const selection = { instanceId: 'grok', driver: 'grok', environmentId: 'local',
    model: 'grok-4.7', snapshot: { auth: { email, status: 'authenticated' } } };

  assert.deepEqual(rows(quotaSource(state, selection).windows), [['weekly', 99]]);
  selection.snapshot.auth.workspaceId = 'other-team';
  assert.deepEqual(rows(quotaSource(state, selection).windows), []);
  selection.snapshot.auth.workspaceId = undefined;
  selection.snapshot.auth.email = 'other@example.test';
  assert.deepEqual(rows(quotaSource(state, selection).windows), []);
});

test('shows the reported Argon quota when T3 Code supplies the thinking level separately', () => {
  const model = 'gemini-4-argon';
  const instanceId = 'argon-account';
  const props = {
    isComposerOwned: true, environmentId: 'local', activeInstanceId: instanceId, model,
    instanceEntries: [{ instanceId, driverKind: 'antigravity' }],
    modelOptionsByInstance: new Map([[instanceId, [{
      slug: model, name: 'Gemini 4 Argon',
      capabilities: { optionDescriptors: [{
        id: 'thought_level', role: 'reasoning', type: 'select', currentValue: 'medium',
        options: [{ id: 'medium', label: 'Medium' }, { id: 'high', label: 'High' }],
      }] },
    }]]]),
  };
  const modelSelection = { instanceId, model, options: [{ id: 'thought_level', value: 'high' }] };
  const picker = { __reactFiber$fixture: { memoizedProps: props, return: {
    memoizedProps: { selectedModelSelection: modelSelection },
  } } };
  const instance = {
    instanceId, driver: 'antigravity', boundAccountId: 'synthetic',
    boundAccountEmail: 'bound@example.test',
    runtime: { accountEmail: 'bound@example.test' },
    windows: [window('gemini-4-argon-medium', 31), window('gemini-4-argon-high', 58)],
  };
  const state = { environmentId: 'local', workspace: { providerInstances: [instance] } };
  const display = () => {
    const selection = readSelection(picker);
    return capsuleWindows(quotaSource(state, selection)).map(window => window.text).join(' · ') || '—';
  };
  assert.equal(display(), '58%');
  modelSelection.options[0].value = 'medium';
  assert.equal(display(), '31%');
  instance.windows.push(window('gemini-argon-wtu-weekly', 42));
  assert.equal(display(), '周 42%');
});

test('shows Antigravity quotas only for the confirmed runtime identity', () => {
  const instance = {
    instanceId: 'antigravity', driver: 'antigravity', boundAccountId: 'old-account',
    boundAccountEmail: 'old@example.test', boundAccountPlan: 'g1-ultra-tier',
    windows: [window('gemini-5h', 58)],
  };
  const selection = {
    environmentId: 'local', instanceId: instance.instanceId, driver: instance.driver, model: 'gemini-3.5-flash-lite',
    threadId: null,
    snapshot: { auth: { email: instance.boundAccountEmail, status: 'authenticated' },
      usageLimits: { windows: [{ id: 'gemini-5h', usedPercent: 42 }] } },
  };
  const state = { environmentId: 'local', workspace: { providerInstances: [instance] } };
  const previous = quotaSource(state, selection);
  assert.equal(previous.account, 'old@example.test');
  assert.equal(previous.plan, 'Ultra');
  assert.deepEqual(rows(previous.windows), [['gemini-5h', 58]]);

  Object.assign(instance, {
    boundAccountId: 'new-account', boundAccountEmail: 'new@example.test',
    boundAccountPlan: 'g1-pro-tier', windows: [window('gemini-5h', 100)],
  });
  const current = quotaSource(state, selection);
  assert.equal(current.account, 'old@example.test');
  assert.equal(current.plan, '');
  assert.equal(current.identityProblem, '账号不一致');
  assert.deepEqual(rows(current.windows), []);
  selection.snapshot.auth = { status: 'authenticated' };
  assert.equal(quotaSource(state, selection).identityProblem, '待确认');
  instance.runtime = { accountEmail: 'new@example.test', authUsesKeyring: true };
  assert.equal(quotaSource(state, selection).identityProblem, '账号不一致');
  instance.runtime.authUsesKeyring = false;
  assert.deepEqual(rows(quotaSource(state, selection).windows), [['gemini-5h', 100]]);
  assert.equal(quotaSource(state, selection).plan, 'Pro');

  Object.assign(instance, {
    boundAccountId: undefined, boundAccountEmail: 'external@example.test',
    boundAccountPlan: undefined, windows: [],
  });
  const external = quotaSource(state, selection);
  assert.equal(external.account, 'new@example.test');
  assert.equal(external.identityProblem, '账号不一致');
  assert.deepEqual(rows(external.windows), []);
});

test('uses current Google group membership before cached model or bucket names', () => {
  const shared = { id: 'gemini', name: 'Gemini Models',
    models: ['Gemini 4 Neon', 'Gemini 4 Argon', 'Gemini 4 Carbon', 'Gemini 4 Hydrogen', 'Gemini 3.8 Flash'],
    windows: [{ id: 'gemini-5h', period: '5h', percentage: 71 },
      { id: 'gemini-weekly', period: 'weekly', percentage: 83 }] };
  const windows = [window('gemini-argon-wtu-weekly', 24), window('gemini-4-argon-high', 51),
    { ...window('gemini-5h', 71), resetTime: '2026-11-24T13:00:00Z' }, window('gemini-weekly', 83)];
  const groupFor = (model, groups, records = windows) => relevantWindows(records,
    { driver: 'antigravity', model }, groups);
  const current = groupFor('gemini-4-argon-high', [shared]);
  assert.deepEqual(rows(current), [['gemini-5h', 71], ['gemini-weekly', 83]]);
  assert.equal(current[0].resetTime, '2026-11-24T13:00:00Z');
  assert.deepEqual(rows(groupFor('gemini-4-carbon', [shared])), rows(current));
  const independent = { id: 'separate', name: 'Argon', models: ['Gemini 4 Argon'],
    windows: [{ id: 'new-weekly-id', period: 'weekly', percentage: 24 }] };
  assert.deepEqual(rows(groupFor('gemini-4-argon-high',
    [{ ...shared, models: ['Gemini 4 Carbon'] }, independent])),
    [['new-weekly-id', 24]]);
  assert.deepEqual(rows(groupFor('gemini-4-argon-high',
    [{ ...shared, models: ['Gemini 4 Carbon'] }], windows.slice(2))), []);
  assert.deepEqual(rows(groupFor('gemini-4-argon-high', undefined, windows.slice(2))),
    [['gemini-5h', 71], ['gemini-weekly', 83]]);
  assert.deepEqual(rows(groupFor('gemini-4-argon-high', undefined)),
    [['gemini-argon-wtu-weekly', 24]]);
});

test('updates gateway quota only for the endpoint T3 Code actually reports', () => {
  const id = 'codex';
  const origin = 'http://127.0.0.1:12345';
  const binding = { instanceId: id, driver: id, boundAccountId: 'gateway', boundIsApiService: true };
  const state = {
    environmentId: 'local', workspace: { providerInstances: [binding] },
    gateways: [{ instanceId: id, usage: { origin, windows: [window('5h', 200)] } }],
  };
  const selection = { environmentId: 'local', instanceId: id, driver: id, model: 'gpt-6.2-sol',
    snapshot: { auth: { email: 'old@example.test', endpoint: 'https://example.invalid' },
      usageLimits: { windows: [{ id: '5h', usedPercent: 24 }] } } };
  const previous = quotaSource(state, selection);
  assert.equal(previous.account, 'old@example.test');
  assert.deepEqual(rows(previous.windows), [['5h', 76]]);
  selection.snapshot.auth.endpoint = origin;
  const current = quotaSource(state, selection);
  assert.equal(current.account, 'Codex API Service');
  assert.deepEqual(rows(current.windows), [['5h', 200]]);
  selection.snapshot.auth.endpoint = 'http://localhost:12345';
  const localhostMatch = quotaSource(state, selection);
  assert.equal(localhostMatch.account, 'Codex API Service');
  assert.deepEqual(rows(localhostMatch.windows), [['5h', 200]]);
});

test('shows human subscription labels and does not guess the Antigravity tier', () => {
  assert.equal(displayPlan('antigravity', 'g1-pro-tier'), 'Pro');
  assert.equal(displayPlan('antigravity', 'g1-ultra-tier'), 'Ultra');
  assert.equal(displayPlan('antigravity', 'free-tier'), 'Free');
  assert.equal(displayPlan('antigravity', 'standard-tier'), 'Free');
  assert.equal(displayPlan('antigravity', 'g1-unrecognized-tier'), '未知');
  assert.equal(displayPlan('antigravity', 'Google account'), '未知');
  assert.equal(displayPlan('codex', 'team'), 'Team');
  assert.equal(displayPlan('codex', 'API_KEY'), 'API Key');
  assert.equal(displayPlan('claudeAgent', 'default-claude-max-5x'), 'max-5x');
  assert.equal(displayPlan('claudeAgent', 'default-claude-max-20x'), 'max-20x');
});

test('centers the quota in the widest footer gap as the model picker grows', () => {
  const rect = (left, top, width, height) => ({ left, top, width, height, right: left+width, bottom: top+height });
  const footer = rect(100, 560, 720, 40);
  const attachment = rect(113, 563, 28, 28);
  const send = rect(776, 562, 28, 30);
  const picker = rect(590, 563, 90, 30);
  const longPicker = rect(475, 563, 205, 30);
  const capsule = rect(0, 0, 150, 26);
  const viewport = { width: 1100, height: 650 };
  const longLabel = centeredCapsulePosition(footer, longPicker, [attachment, longPicker, send], capsule, viewport);
  assert.equal(longLabel?.left, 233);
  const centered = centeredCapsulePosition(footer, picker, [attachment, picker, send], capsule, viewport);
  assert.equal(centered.left, 290.5);
  assert.equal(centered.top, 565);

  const controls = [attachment, picker, send, rect(300, 563, 60, 30), rect(370, 520, 220, 30)];
  assert.equal(centeredCapsulePosition(footer, picker, controls, capsule, viewport).left, 400);
  assert.equal(centeredCapsulePosition(footer, picker, controls, capsule, { ...viewport, width: 297 }), null);
  assert.equal(centeredCapsulePosition(footer, picker, controls, capsule, { ...viewport, height: 570 }), null);
});

test('reports provider identity and usable models without treating retained models as ready', () => {
  const entries = [{
    instanceId: 'one', snapshot: {
      auth: { email: 'person@example.test', workspaceId: 'work', status: 'authenticated', endpoint: 'https://example.test' },
      models: [{ slug: 'first' }], status: 'ready',
    },
  }, {
    instanceId: 'two', snapshot: {
      auth: { status: 'unauthenticated' }, models: [{ slug: 'old' }], modelsRetained: true, availability: 'unavailable',
    },
  }];
  const rows = providerStatus(entries, [{ instanceId: 'two', usage: { origin: 'https://example.test' } }]);
  assert.equal(rows[0].accountEmail, 'person@example.test');
  assert.equal(rows[0].workspaceId, 'work');
  assert.equal(rows[0].modelCount, 1);
  assert.equal(rows[0].matchesGateway, false);
  assert.equal(rows[1].modelCount, 0);
  assert.equal(rows[1].availability, 'unavailable');
});

test('keeps the current account badge and quota actions in the details dialog', () => {
  const dialog = vm.runInNewContext(
    fs.readFileSync(path.join(__dirname, 'codex_app_injection_dialog.js'), 'utf8'),
    {
      usageLabels: { title: 'Quota & Usage', refreshData: 'Refresh data' },
      missingLabel: '—', closeLabel: 'Close', quotaEmptyLabel: 'Quota unavailable',
      document: { activeElement: null },
    },
  );
  const selection = { environmentId: 'local', instanceId: 'work', driver: 'antigravity', model: 'gemini-4-hydrogen' };
  const instance = {
    instanceId: 'work', driver: 'antigravity', boundAccountId: 'pro',
    boundAccountEmail: 'pro@example.test', boundAccountPlan: 'g1-pro-tier',
    runtime: { accountEmail: 'pro@example.test' },
    windows: [window('gemini-5h', 98)],
  };
  const state = { environmentId: 'local', workspace: { providerInstances: [instance] } };
  const source = quotaSource(state, selection);
  const details = { querySelector: () => null, querySelectorAll: () => [], contains: () => false };
  const accountBadge = quotaSelection.accountBadge(source, selection, dialog.badge);
  dialog.render({}, details, {
    accountBadge, badgeKind: 'codex',
    direct: { account: source.account, windows: source.windows, balances: [] },
  });
  assert.match(details.innerHTML, /pro@example\.test/);
  assert.match(details.innerHTML, /class="badge pro"[^>]+title="g1-pro-tier">PRO</);
  assert.match(details.innerHTML, /98%/);
  assert.deepEqual([...details.innerHTML.matchAll(/<button[^>]*>([^<]*)<\/button>/g)].map(match => match[1]),
    ['×', 'Refresh data']);

  const codex = { instance: { boundAccountId: 'lite', boundAccountPlan: 'prolite', boundAccountPlanBadgeKind: 'pro-100' } };
  assert.match(quotaSelection.accountBadge(codex, { driver: 'codex' }, dialog.badge, 'outline'),
    /class="badge codex-pro-lite t3-plan-outline" title="prolite">PRO 100</);
  assert.equal(quotaSelection.accountBadge({ ...source, identityProblem: 'Mismatch' }, selection, dialog.badge), '');
});

test('provider control asks for a ticket only after matching the local T3 environment', async () => {
  const calls = [];
  let bearerRequests = 0;
  const connect = vm.runInNewContext(
    fs.readFileSync(path.join(__dirname, 't3code_provider_control_ticket.js'), 'utf8'),
    {
      URL,
      AbortSignal,
      window: { desktopBridge: {
        getLocalEnvironmentBootstraps: () => [{ id: 'primary', httpBaseUrl: 'http://127.0.0.1:6733' }],
        getLocalEnvironmentBearerToken: async () => { bearerRequests++; return 'synthetic-bearer'; },
      } },
      fetch: async (url, options) => {
        calls.push([url.pathname, options]);
        return {
          ok: true,
          json: async () => url.pathname.includes('websocket-ticket')
            ? { ticket: 'synthetic-ticket' }
            : { environmentId: 'local', orchestrationProtocolVersion: 2 },
        };
      },
    },
  );
  assert.equal(await connect('remote'), null);
  assert.equal(bearerRequests, 0);
  const socket = new URL(await connect('local'));
  assert.equal(socket.protocol, 'ws:');
  assert.equal(socket.pathname, '/ws');
  assert.equal(socket.searchParams.get('wsTicket'), 'synthetic-ticket');
  assert.equal(socket.searchParams.get('orchestrationProtocol'), '2');
  assert.equal(bearerRequests, 1);
  assert.equal(calls[2][0], '/api/auth/websocket-ticket/rpc');
  assert.equal(calls[2][1].method, 'POST');
});

const { relaySnapshot } = vm.runInNewContext(
  fs.readFileSync(path.join(__dirname, 't3code_app_injection_clerk.js'), 'utf8'),
);

test('reads official managed tunnel usage and the exact account limit', () => {
  const environments = { environments: [
    { endpoint: { providerKind: 'cloudflare_tunnel' } },
    { endpoint: { providerKind: 'direct' } },
  ] };
  const linked = relaySnapshot(environments, { available: true, link: { plan: 'pro' } });
  assert.equal(linked.tunnelsUsed, 1);
  assert.equal(linked.tunnelsLimit, 10);
  assert.equal(linked.planType, 'pro');
  const unlinked = relaySnapshot(environments, { available: true, link: null });
  assert.equal(unlinked.tunnelsUsed, 1);
  assert.equal(unlinked.tunnelsLimit, 3);
  assert.equal(unlinked.t3chatLinked, false);
  const overridden = relaySnapshot({ environments: [] }, {
    available: true, link: { plan: 'premier' }, maxTunnelsOverride: 0,
  });
  assert.equal(overridden.tunnelsUsed, 0);
  assert.equal(overridden.tunnelsLimit, 0);
  assert.equal(overridden.planType, 'premier');
});

test('keeps failed or incomplete Relay responses unknown', () => {
  const environments = { environments: [] };
  assert.equal(relaySnapshot(environments, null), null);
  assert.equal(relaySnapshot(null, { available: true, link: null }), null);
  assert.equal(relaySnapshot(environments, { available: true }), null);
  assert.equal(relaySnapshot(environments, { available: true, link: { plan: 'future' } }), null);
});

test('discards Relay results from the previous Clerk session', async () => {
  const window = { Clerk: { loaded: true, user: null, session: null } };
  const pendingResponses = new Map();
  const { read } = vm.runInNewContext(
    fs.readFileSync(path.join(__dirname, 't3code_app_injection_clerk.js'), 'utf8'),
    {
      window, setTimeout, clearTimeout, AbortSignal,
      fetch: (url, options) => new Promise(resolve => {
        pendingResponses.set(options.headers.Authorization + new URL(url).pathname, resolve);
      }),
    },
  );
  const root = {};
  const setSession = (id) => {
    window.Clerk.user = { id, primaryEmailAddress: { emailAddress: id + '@example.test' } };
    window.Clerk.session = { id: 'session_' + id, status: 'active', getToken: async () => id };
  };
  const reply = (id, limit) => {
    pendingResponses.get('Bearer ' + id + '/v1/environments')({
      ok: true, json: async () => ({ environments: [] }),
    });
    pendingResponses.get('Bearer ' + id + '/v1/client/t3chat-link')({
      ok: true, json: async () => ({ available: true, link: null, maxTunnelsOverride: limit }),
    });
  };
  const settle = () => new Promise(resolve => setImmediate(resolve));

  setSession('account_one');
  assert.equal(read(root, {}).email, 'account_one@example.test');
  await settle();
  setSession('account_two');
  assert.equal(read(root, {}).tunnelsLimit, undefined);
  await settle();
  reply('account_one', 10);
  await settle();
  assert.equal(read(root, {}).tunnelsLimit, undefined);
  reply('account_two', 3);
  await settle();
  assert.equal(read(root, {}).tunnelsLimit, 3);
  window.Clerk.session = null;
  assert.equal(read(root, {}).signedIn, false);
});
