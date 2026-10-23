import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';
import type { DownloadEvent } from '@tauri-apps/plugin-updater';
import { createUpdaterDownloadProgress } from '../src/utils/updaterDownloadProgress';
import * as retry from '../src/utils/updaterRetry';
import { parseUpdaterReleaseNotes } from '../src/utils/updaterReleaseNotes';
import { deferred, settlePromises } from './helpers/reactHookHarness';

// Exercise App's real callbacks without opening a desktop window or downloading a release.
const app = ts.createSourceFile('App.tsx', readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8'),
  ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const callbacks = new Map<string, string>();
let backgroundEffect = '';
function visit(node: ts.Node) {
  if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name)
    && node.initializer && ts.isCallExpression(node.initializer)
    && node.initializer.expression.getText(app) === 'useCallback') {
    callbacks.set(node.name.text, node.initializer.arguments[0].getText(app));
  }
  if (ts.isCallExpression(node) && node.expression.getText(app) === 'useEffect'
    && node.arguments[0]?.getText(app).includes('const UPDATE_POLL_INTERVAL_MS')) {
    backgroundEffect = node.arguments[0].getText(app);
  }
  ts.forEachChild(node, visit);
}
visit(app);
assert.ok(backgroundEffect);

type Action = { state: string; version: string | null; progress: number; requiresInstall: boolean };
function candidate(version = '1.3.70') {
  const done = deferred<void>();
  const started = deferred<void>();
  let onEvent: ((event: DownloadEvent) => void) | undefined;
  return {
    version, currentVersion: '1.3.69', body: '', rawJson: {}, downloads: 0, closes: 0, done, started,
    download(callback: (event: DownloadEvent) => void) {
      this.downloads++; onEvent = callback; started.resolve(); return done.promise;
    },
    async close() { this.closes++; },
    emit(event: DownloadEvent) { assert.ok(onEvent, 'download has started'); onEvent(event); },
  };
}
type Candidate = ReturnType<typeof candidate>;

function harness(options: {
  check?: () => Promise<Candidate | null>;
  settings?: () => Promise<{ auto_install: boolean }>;
  prepareInfo?: () => Promise<unknown>;
} = {}) {
  let action: Action = { state: 'available', version: '1.3.70', progress: 0, requiresInstall: true };
  let retryStatus = '';
  let error = '';
  let checks = 0;
  let settingsReads = 0;
  const timers: (() => void)[] = [];
  const updates: { previous: Action; update: (old: Action) => Action }[] = [];
  const notifications: unknown[] = [];
  const refs = {
    updateCheckInFlightRef: { current: false },
    updateDownloadOwnerRef: { current: 'none' },
    updateDownloadTaskIdRef: { current: 0 },
    updateCancelRequestedRef: { current: false },
    activeUpdateDownloadRef: { current: null as Candidate | null },
    pendingSilentUpdateRef: { current: null as Candidate | null },
  };
  const exports: Record<string, any> = {};
  const context: Record<string, any> = {
    exports, ...refs, ...retry, createUpdaterDownloadProgress, parseUpdaterReleaseNotes,
    UPDATE_DOWNLOAD_RETRY_DELAYS_MS: [0], UPDATE_CHECK_RETRY_DELAYS_MS: [],
    updateRuntimeInfoLoaded: true, isLinuxManagedUpdate: false, performance: { now: () => 0 },
    updatePromptPolicyRef: { current: { enabled: true, revision: 0 } },
    console: { log() {}, warn() {}, error() {} },
    window: { setInterval(callback: () => void) { timers.push(callback); return timers.length; },
      clearInterval() {}, dispatchEvent() {} },
    async invoke(command: string) {
      if (command === 'get_update_settings') {
        settingsReads++;
        return options.settings ? options.settings() : { auto_install: true };
      }
      assert.ok(['save_pending_update_notes', 'update_last_check_time'].includes(command), command);
    },
    async runUpdaterCheck() { checks++; return options.check ? options.check() : null; },
    async closeUpdaterHandle(update?: Candidate | null) { await update?.close(); },
    async fetchRemoteConfigState() { return { updatePromptMode: 'popup' }; },
    async prepareUpdateNotificationInfo(update: Candidate) {
      return options.prepareInfo ? options.prepareInfo() : {
        latest_version: update.version, current_version: update.currentVersion,
        release_notes: '', release_notes_zh: '',
      };
    },
    setUpdateAction(next: Action | ((old: Action) => Action)) {
      if (typeof next === 'function') {
        const first = next(action);
        const replayed = next(action);
        assert.equal(JSON.stringify(first), JSON.stringify(replayed), 'React updater replay must not recount bytes');
        updates.push({ previous: action, update: next });
        action = replayed;
      } else action = next;
    },
    setUpdateRetryStatus(value: string) { retryStatus = value; },
    setUpdateDownloadError(value: string) { error = value; },
    setUpdateErrorDetails() {}, setSilentUpdateVersion() {},
    setUpdateNotificationInfo(value: unknown) { notifications.push(value); },
    applyUpdateReminderPreference() {}, openAutomaticUpdatePrompt() {}, writeUpdateLog() {},
    t: (key: string) => key,
  };
  for (const name of ['runSharedUpdateDownload', 'handleUpdateCheckResult']) {
    assert.ok(callbacks.has(name));
    vm.runInNewContext(ts.transpileModule(`exports.${name} = ${callbacks.get(name)};`, {
      compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
    }).outputText, context);
    context[name] = exports[name];
  }
  vm.runInNewContext(ts.transpileModule(`exports.background = ${backgroundEffect};`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
  }).outputText, context);
  return {
    refs, updates, notifications, start: exports.runSharedUpdateDownload as (version: string, initial?: Candidate) => Promise<void>,
    background: exports.background as () => () => void,
    poll: () => timers.at(-1)!(), state: () => action, retryStatus: () => retryStatus,
    error: () => error, checks: () => checks, settingsReads: () => settingsReads,
  };
}

const started: DownloadEvent = { event: 'Started', data: { contentLength: 1000 } };
const chunk = (size: number): DownloadEvent => ({ event: 'Progress', data: { chunkLength: size } });

test('manual download followed by the hourly check keeps one task and real progress', async () => {
  const update = candidate();
  const h = harness({ check: async () => update });
  const task = h.start(update.version);
  await update.started.promise;
  update.emit(started); update.emit(chunk(320));
  assert.equal(h.state().progress, 32);
  h.background(); h.poll(); await settlePromises();
  assert.equal(h.settingsReads(), 0);
  assert.equal(h.checks(), 1);
  assert.equal(update.downloads, 1);
  update.emit(chunk(400));
  assert.equal(h.state().progress, 72);
  update.emit(chunk(10));
  assert.equal(h.state().progress, 73);
  update.done.resolve(); await task;
  assert.equal(h.state().state, 'ready');
  assert.equal(h.state().progress, 100);
  h.poll(); await settlePromises();
  await h.start(update.version);
  assert.equal(h.checks(), 1, 'ready update must not be downloaded again');
  assert.equal(h.refs.pendingSilentUpdateRef.current, update);
});

test('rapid manual clicks and background requests reuse the active download', async () => {
  const update = candidate(); const redundant = candidate();
  const h = harness({ check: async () => update });
  const task = h.start(update.version);
  await h.start(update.version, redundant);
  await update.started.promise;
  assert.equal(h.checks(), 1); assert.equal(update.downloads, 1);
  assert.equal(redundant.downloads, 0); assert.equal(redundant.closes, 1);
  await h.start(update.version, update);
  assert.equal(update.closes, 0, 'deduplication must not close the active handle');
  update.done.resolve(); await task;
});

for (const autoInstall of [true, false]) {
  for (const stage of ['check', 'prepare'] as const) {
    test(`late background ${stage} result cannot overwrite a manual task (auto_install=${autoInstall})`, async () => {
      const backgroundUpdate = candidate(); const manualUpdate = candidate();
      const checked = deferred<Candidate>(); const prepared = deferred<unknown>();
      let calls = 0;
      const h = harness({
        settings: async () => ({ auto_install: autoInstall }),
        check: async () => ++calls === 1 ? (stage === 'check' ? checked.promise : backgroundUpdate) : manualUpdate,
        prepareInfo: () => prepared.promise,
      });
      h.background(); await settlePromises();
      assert.equal(h.checks(), 1);
      const task = h.start(manualUpdate.version); await manualUpdate.started.promise;
      manualUpdate.emit(started); manualUpdate.emit(chunk(720));
      if (stage === 'check') checked.resolve(backgroundUpdate);
      else prepared.resolve({ latest_version: backgroundUpdate.version });
      await settlePromises();
      assert.equal(backgroundUpdate.downloads, 0); assert.equal(backgroundUpdate.closes, 1);
      assert.equal(h.state().progress, 72); assert.equal(h.notifications.length, 0);
      assert.equal(h.refs.activeUpdateDownloadRef.current, manualUpdate);
      manualUpdate.done.resolve(); await task;
    });
  }
}

test('automatic download uses the same transaction and already checked handle', async () => {
  const update = candidate();
  const h = harness({ check: async () => update });
  h.background(); await update.started.promise;
  await h.start(update.version);
  assert.equal(h.checks(), 1); assert.equal(update.downloads, 1);
  update.emit(started); update.emit(chunk(320));
  assert.equal(h.state().progress, 32);
  update.done.resolve(); await settlePromises();
  assert.equal(h.refs.pendingSilentUpdateRef.current, update);
  assert.equal(h.state().state, 'ready');
  assert.equal(h.refs.updateCheckInFlightRef.current, false);
});

test('effect recreation cannot run a second background check while settings are slow', async () => {
  const settings = deferred<{ auto_install: boolean }>();
  const h = harness({ settings: () => settings.promise });
  const cleanup = h.background(); cleanup(); h.background(); h.poll();
  assert.equal(h.settingsReads(), 1);
  settings.resolve({ auto_install: true }); await settlePromises();
  assert.equal(h.checks(), 1); assert.equal(h.refs.updateCheckInFlightRef.current, false);
  h.poll(); await settlePromises();
  assert.equal(h.checks(), 2, 'later checks still run after completion');
});

test('retry starts fresh progress and ignores callbacks from the failed attempt', async () => {
  const first = candidate(); const second = candidate(); let calls = 0;
  const h = harness({ check: async () => ++calls === 1 ? first : second });
  const task = h.start(first.version); await first.started.promise;
  first.emit(started); first.emit(chunk(720)); assert.equal(h.state().progress, 72);
  first.done.reject(new Error('network connection reset'));
  await second.started.promise;
  assert.equal(first.closes, 1);
  assert.equal(h.retryStatus(), 'update_notification.downloadRetrying');
  assert.equal(h.state().progress, 0);
  second.emit(started); second.emit(chunk(320)); assert.equal(h.state().progress, 32);
  first.emit(chunk(100)); assert.equal(h.state().progress, 32);
  second.done.resolve(); await task;
  assert.equal(h.refs.pendingSilentUpdateRef.current, second);
  assert.equal(h.retryStatus(), ''); assert.equal(h.error(), '');
});

test('old queued progress and late completion cannot overwrite a replacement task', async () => {
  const first = candidate(); const second = candidate(); let calls = 0;
  const h = harness({ check: async () => ++calls === 1 ? first : second });
  const oldTask = h.start(first.version); await first.started.promise;
  first.emit(started); first.emit(chunk(720));
  const queued = h.updates.at(-1)!;
  // Model cancellation invalidating the task before the transport finishes.
  h.refs.updateDownloadTaskIdRef.current++;
  h.refs.updateDownloadOwnerRef.current = 'none';
  const newTask = h.start(second.version); await second.started.promise;
  second.emit(started); second.emit(chunk(320));
  assert.equal(queued.update(h.state()), h.state());
  first.emit(chunk(100)); assert.equal(h.state().progress, 32);
  first.done.resolve(); await oldTask;
  assert.equal(h.state().progress, 32); assert.equal(h.retryStatus(), '');
  assert.equal(h.refs.activeUpdateDownloadRef.current, second);
  second.done.resolve(); await newTask;
});

test('missing content length stays bounded and each new download resets its counter', () => {
  const track = createUpdaterDownloadProgress();
  assert.equal(track({ event: 'Started', data: {} }), 0);
  for (let i = 0; i < 110; i++) assert.equal(track(chunk(10)), Math.min(i + 1, 95));
  assert.equal(track({ event: 'Finished' }), 100);
  assert.equal(track(started), 0);
  assert.equal(track(chunk(320)), 32);
  assert.equal(track(chunk(400)), 72);
  assert.equal(track(chunk(1000)), 100);
});
