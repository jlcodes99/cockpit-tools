// Isolated fixture: IPC is intercepted before any component loads. No real app or account access.
import React from 'react';
import { createRoot } from 'react-dom/client';
import { initI18n } from '../../src/i18n';
import { ClaudeSessionHandoff } from '../../src/components/claude/ClaudeSessionHandoff';
import type { ClaudeHandoffRunSummary } from '../../src/types/claudeHandoff';
import '../../src/App.css';

const id = (n: number) => `00000000-0000-4000-8000-${String(n).padStart(12, '0')}`;
const scenario = new URLSearchParams(location.search).get('scenario') || 'normal';
const contractScenario = scenario.startsWith('switch-contract-');
const contractReason = scenario.startsWith('switch-contract-unavailable-') ? 'DESKTOP_CONTRACT_UNAVAILABLE'
  : scenario.startsWith('switch-contract-unsupported-') ? 'DESKTOP_CONTRACT_UNSUPPORTED'
    : scenario === 'switch-contract-changed-pending' ? 'DESKTOP_CONTRACT_CHANGED'
      : scenario === 'switch-contract-default-profile-pending' ? 'DEFAULT_PROFILE_REQUIRED' : null;
const diagnosticVersion = scenario === 'switch-contract-version-unavailable' ? ''
  : scenario === 'switch-contract-future' ? '999.0.future'
    : scenario.includes('unsupported') ? '2.999.0' : '2.110.0';
const accounts = (scenario === 'switch-race' ? [1, 2, 3] : [1, 2]).map(n => ({ id: `account-${n}`, email: `account-${n}@example.test`, auth_mode: 'desktop_oauth' as const,
  account_uuid: id(n), organization_uuid: id(n + 10), created_at: 1, last_used: 1 }));
const initialRun = { id: `run-${id(99)}`, state: 'applied', createdAt: 1789800000000, created: 1, updated: 0, skippedMissing: 0, skippedStale: 0, replacedBranches: 0,
  source: { account: id(1), org: id(11) }, target: { account: id(2), org: id(12) }, backupDir: '/synthetic/backup' };
const switchFlow = scenario.startsWith('switch');
const continuityFlow = scenario.startsWith('switch-continuity');
const verifiedSource = scenario === 'switch-continuity-verified-source';
// Put the CLI row first and give it exactly the Desktop account/org identity.
// Neither row order nor a legacy currentAccountId may substitute for auth mode.
const parentAccounts = verifiedSource ? [{ ...accounts[0], id: 'account-cli-duplicate',
  email: 'duplicate-cli@example.test', auth_mode: 'oauth' as const }, ...accounts] : accounts;
const startWarning = scenario === 'switch-continuity-start-failed' ? 'DESKTOP_START_FAILED' : null;
// Synthetic empty target: 166 latest conversations plus six preserved branches.
// These counts do not claim a mutation or verification of any live account.
const continuityCounts = { created: 172, updated: 0, unchanged: 0, preservedBranches: 6 };
const usesSavedAccountIndex = scenario.startsWith('saved-index');
let run: ClaudeHandoffRunSummary | null = scenario.includes('pending') ? { ...initialRun, state: 'applying', lastError: 'CLAUDE_WRITER_RUNNING' }
  : contractScenario && scenario.endsWith('-applied') ? { ...initialRun } : null;
let mutated = false;
let recoveryFailed = false;
const calls: { command: string; args: unknown }[] = [];
const switchCalls: string[] = [];
const completedSwitches: string[] = [];
type Progress = { stage: string; completed: number; total: number };
const callbacks = new Map<number, { callback: (event: unknown) => void; once: boolean }>();
const listeners = new Map<number, { event: string; handler: number }>();
const events: { payload: Progress; at: number; delivered: number }[] = [];
const busyTransitions: { busy: boolean; at: number }[] = [];
let nextCallback = 1;
let nextListener = 1;
let applyAttempts = 0;
let previewApprovals = 0;
let contractApproval: string | null = null;
const delay = (ms: number) => new Promise(resolve => setTimeout(resolve, ms));
const emitProgress = (payload: Progress) => {
  let delivered = 0;
  for (const [eventId, listener] of listeners) {
    if (listener.event !== 'claude-handoff-progress') continue;
    const entry = callbacks.get(listener.handler);
    if (!entry) throw new Error('Event listener has no transformed callback');
    entry.callback({ event: listener.event, id: eventId, payload });
    if (entry.once) callbacks.delete(listener.handler);
    delivered += 1;
  }
  events.push({ payload, at: performance.now(), delivered });
};
const simulateProgress = async () => {
  const stages: Progress[] = [
    { stage: 'checking', completed: 0, total: 172 },
    { stage: 'stopping', completed: 0, total: 0 },
    { stage: 'hashing', completed: 0, total: 172 },
    { stage: 'backup', completed: 0, total: 172 },
    { stage: 'writing', completed: 0, total: 172 },
    { stage: 'writing', completed: 86, total: 172 },
    { stage: 'writing', completed: 172, total: 172 },
    { stage: 'verifying', completed: 172, total: 172 },
    { stage: 'switching', completed: 0, total: 0 },
    ...(scenario === 'switch-continuity-uncertain' ? [] : [{ stage: 'confirming', completed: 0, total: 0 }]),
  ];
  for (const progress of stages) {
    emitProgress(progress);
    await delay(550);
    if (scenario === 'switch-continuity-retry' && applyAttempts === 1 && progress.stage === 'hashing') {
      throw 'PREVIEW_CHANGED';
    }
    if (scenario === 'switch-continuity-verification-error' && progress.stage === 'verifying') {
      run = { ...initialRun, ...continuityCounts, state: 'applying', lastError: 'POST_IMAGE_MISMATCH' };
      throw 'POST_IMAGE_MISMATCH';
    }
  }
};
Object.assign(window, {
  __handoffCalls: calls, __switchCalls: switchCalls, __completedSwitches: completedSwitches,
  __handoffFixtureAccounts: parentAccounts,
  __handoffBusy: false, __handoffConfirm: true, __handoffEvents: events, __handoffBusyTransitions: busyTransitions,
  __emitHandoffProgress: emitProgress,
  __handoffEventState: () => ({ listeners: listeners.size, callbacks: callbacks.size, applyAttempts }),
  __TAURI_EVENT_PLUGIN_INTERNALS__: {
    unregisterListener: (event: string, eventId: number) => {
      const listener = listeners.get(eventId);
      if (listener?.event === event) {
        callbacks.delete(listener.handler);
        listeners.delete(eventId);
      }
    },
  },
});
Object.assign(window, { __TAURI_INTERNALS__: {
  transformCallback: (callback: (event: unknown) => void, once = false) => {
    const callbackId = nextCallback++;
    callbacks.set(callbackId, { callback, once });
    return callbackId;
  },
  unregisterCallback: (callbackId: number) => { callbacks.delete(callbackId); },
  invoke: async (command: string, args: Record<string, unknown>) => {
    calls.push({ command, args });
    if (command === 'plugin:event|listen') {
      if (args.event !== 'claude-handoff-progress' || !callbacks.has(args.handler as number)) {
        throw new Error('Invalid synthetic event subscription');
      }
      const eventId = nextListener++;
      listeners.set(eventId, { event: args.event, handler: args.handler as number });
      return eventId;
    }
    if (command === 'plugin:event|unlisten') {
      const listener = listeners.get(args.eventId as number);
      if (listener) callbacks.delete(listener.handler);
      listeners.delete(args.eventId as number);
      return;
    }
    if (command === 'plugin:dialog|message') {
      const buttons = args.buttons as { OkCancelCustom?: [string, string] } | string;
      return (window as any).__handoffConfirm
        ? (typeof buttons === 'object' ? buttons.OkCancelCustom?.[0] : 'Ok')
        : (typeof buttons === 'object' ? buttons.OkCancelCustom?.[1] : 'Cancel');
    }
    if (command === 'claude_handoff_status') {
      if (scenario === 'saved-index-unavailable') throw 'ACCOUNT_INDEX_UNAVAILABLE: /private/synthetic/index.json account-private';
      if (scenario === 'refresh-error' && mutated) throw 'STATUS_FAILED';
      if (scenario === 'partial-rollback-refresh-error' && recoveryFailed) throw 'STATUS_FAILED';
      return { supported: contractScenario ? !contractReason : !scenario.includes('unsupported'),
        reason: contractScenario ? contractReason : scenario.includes('unsupported') ? 'DESKTOP_VERSION_REQUIRES_REVIEW' : null,
        desktopVersion: diagnosticVersion,
        // Only this scenario supplies a synthetic optional process-identity hint. Legacy scenarios
        // intentionally omit the field and still require manual source selection.
        ...(verifiedSource ? {
          currentIdentity: { account: id(mutated ? 2 : 1), org: id(mutated ? 12 : 11) },
          currentAccountId: mutated ? 'account-2' : 'account-cli-duplicate',
        } : {}),
        ...(usesSavedAccountIndex ? { usesSavedAccountIndex: true } : {}),
        accounts: parentAccounts.map(a => ({ id: a.id, label: a.email, eligible: true, reason: null,
          ...(usesSavedAccountIndex ? { identity: { account: a.account_uuid, org: a.organization_uuid } } : {}) })), runs: run ? [run] : [] };
    }
    if (command === 'claude_handoff_preview') {
      if (contractReason) throw contractReason;
      if (scenario === 'slow-preview') await new Promise(resolve => setTimeout(resolve, 800));
      if (scenario === 'switch-race' && args.sourceAccountId === 'account-1') await new Promise(resolve => setTimeout(resolve, 800));
      if (args.sourceAccountId === args.targetAccountId) throw 'SAME_ACCOUNT';
      const conflicting = scenario === 'switch-conflict' || scenario === 'conflict' || scenario === 'privacy-conflict';
      const missingOnly = scenario === 'switch-missing';
      const unsupportedState = scenario === 'switch-contract-native-state';
      // Approval is an opaque nonce independent of the unchanged synthetic plan.
      if (contractScenario) contractApproval = `synthetic-approval-${++previewApprovals}`;
      return { fingerprint: contractApproval ?? (args.sourceAccountId === 'account-3' ? 'third-plan' : 'synthetic-plan'), baselineChanged: true,
        desktopVersion: diagnosticVersion ?? '', created: args.sourceAccountId === 'account-3' ? 7 : 1, updated: conflicting ? 1 : 0, unchanged: 3,
        missing: conflicting || missingOnly || unsupportedState ? 1 : 0, stale: conflicting ? 1 : 0, replacedBranches: conflicting ? 1 : 0,
        issues: unsupportedState ? [{ sessionId: `local_${id(40)}`, title: 'Synthetic conversation with new native state', reason: 'UNSUPPORTED_PERSISTED_FIELD' }]
          : conflicting ? [{ sessionId: `local_${id(40)}`, title: 'Synthetic planning conversation', reason: 'DIVERGENT_METADATA' }] : [],
        warnings: conflicting ? [{ sessionId: `local_${id(41)}`, title: 'Synthetic source continuation', reason: 'SOURCE_BRANCH_SELECTED' }] : [], quotaPausesCleared: 1,
        ...(continuityFlow ? continuityCounts : {}) };
    }
    if (command === 'claude_handoff_apply' || command === 'claude_handoff_apply_and_switch') {
      if (contractReason) throw contractReason;
      if (args.fingerprint !== (contractScenario ? contractApproval : 'synthetic-plan') || args.sourceAccountId !== 'account-1' || args.targetAccountId !== 'account-2'
        || (switchFlow !== (command === 'claude_handoff_apply_and_switch'))) throw 'INVALID_FIXTURE_INPUT';
      applyAttempts += 1;
      if (continuityFlow) await simulateProgress();
      else await delay(250);
      if (scenario === 'switch-contract-changed' && applyAttempts === 1) {
        throw JSON.stringify({ code: 'DESKTOP_CONTRACT_CHANGED', message: '/private/synthetic/storage account-private' });
      }
      if (scenario === 'saved-index-running') throw { code: 'EXTERNAL_COCKPIT_RUNNING', message: '/private/synthetic/Cockpit account-private' };
      if (scenario === 'switch-rejected') throw 'DESKTOP_QUIT_FAILED';
      if (scenario === 'stale') throw 'PREVIEW_CHANGED';
      run = { ...initialRun, ...(continuityFlow ? continuityCounts : {}) }; mutated = true;
      const uncertain = scenario.includes('fail') || scenario === 'switch-continuity-uncertain';
      if (continuityFlow && !uncertain && !startWarning) emitProgress({ stage: 'complete', completed: 0, total: 0 });
      return { run, reopened: !uncertain && !startWarning, warning: startWarning ?? (uncertain ? 'ACCOUNT_SWITCH_UNCERTAIN' : null),
        ...(switchFlow ? { accountSwitched: !uncertain && !startWarning } : {}) };
    }
    if (command === 'claude_handoff_rollback') {
      if (scenario.startsWith('partial-rollback') && !recoveryFailed) {
        run = { ...initialRun, state: 'rolling_back' };
        recoveryFailed = true;
        throw 'IO_ERROR';
      }
      run = { ...initialRun, state: 'rolled_back' }; mutated = true;
      return { run, reopened: false, warning: 'RECOVERED_APP_CLOSED' };
    }
    throw new Error(`Unmocked IPC is prohibited in this fixture: ${command}`);
  },
} });

localStorage.setItem('app-language', new URLSearchParams(location.search).get('lang') || 'en');
await initI18n();
createRoot(document.getElementById('root')!).render(
  <React.StrictMode><main style={{ padding: 32 }}>
    <p>Isolated synthetic fixture · no account access</p><h1>Claude Desktop</h1>
    <div className="toolbar"><div className="toolbar-left">Local Code conversations</div><div className="toolbar-right">
      <ClaudeSessionHandoff accounts={usesSavedAccountIndex || scenario === 'empty-parent' ? [] : parentAccounts}
        // Deliberately disagree with the fresh Status identity in verifiedSource.
        currentAccountId={verifiedSource ? 'account-2' : switchFlow ? scenario === 'switch-no-source' ? null : 'account-1' : 'account-2'}
        switchRequest={switchFlow ? { targetAccountId: 'account-2', sequence: 1 } : null}
        onSwitchAccount={switchFlow ? async id => { switchCalls.push(id); return !scenario.includes('fail'); } : undefined}
        onSwitchCompleted={accountId => { completedSwitches.push(accountId); }}
        privacyModeEnabled={scenario.startsWith('privacy') || scenario === 'saved-index-privacy'}
        onBusyChange={busy => {
          (window as any).__handoffBusy = busy;
          busyTransitions.push({ busy, at: performance.now() });
        }} />
    </div></div>
  </main></React.StrictMode>,
);
