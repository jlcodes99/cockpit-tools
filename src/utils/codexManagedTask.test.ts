import assert from 'node:assert/strict';
import test from 'node:test';

import type { CodexAccount } from '../types/codex';
import type { ManagedCodexTaskEvidence } from '../types/codexManagedTask';
import {
  buildManagedTaskInput,
  canCancelManagedTask,
  emptyManagedTaskDraft,
  isEligibleManagedCodexAccount,
  mergeManagedTaskEvidence,
  validateManagedTaskDraft,
} from './codexManagedTask.ts';

function account(values: Partial<CodexAccount>): CodexAccount {
  return { id: 'account-oauth', auth_mode: 'oauth', ...values } as CodexAccount;
}

function evidence(id: string, observedAt: number): ManagedCodexTaskEvidence {
  return {
    id,
    observedAt,
    source: 'exec_json',
    kind: 'activity',
    confidence: 'informational',
    terminal: false,
  };
}

test('only injectable, ready OAuth accounts are eligible for managed execution', () => {
  assert.equal(isEligibleManagedCodexAccount(account({})), true);
  assert.equal(isEligibleManagedCodexAccount(account({ requires_reauth: true })), false);
  assert.equal(isEligibleManagedCodexAccount(account({ auth_mode: 'apikey' })), false);
  assert.equal(
    isEligibleManagedCodexAccount(account({ token_source_mode: 'chatgpt_web_session' })),
    false,
  );
  assert.equal(
    isEligibleManagedCodexAccount(account({
      agent_identity: {
        agent_runtime_id: 'runtime',
        agent_private_key: 'private',
        account_id: 'account',
        chatgpt_user_id: 'user',
      },
    })),
    false,
  );
  assert.equal(
    isEligibleManagedCodexAccount(account({ auth_mode: 'oauth', authorization_status: 'pending' })),
    false,
  );
});

test('validates required fields, fixed scopes, and non-negative switch limits', () => {
  const draft = emptyManagedTaskDraft();
  assert.equal(validateManagedTaskDraft(draft), 'cwd');
  draft.cwd = ' C:/临时任务 ';
  assert.equal(validateManagedTaskDraft(draft), 'objective');
  draft.objective = ' finish the marker ';
  draft.scopeKind = 'selected';
  assert.equal(validateManagedTaskDraft(draft), 'accounts');
  draft.accountIds = ['a'];
  draft.maxSwitches = '-1';
  assert.equal(validateManagedTaskDraft(draft), 'max_switches');
  draft.maxSwitches = '2';
  assert.equal(validateManagedTaskDraft(draft), null);
});

test('builds a normalized fixed-scope request without duplicating accounts', () => {
  const draft = {
    ...emptyManagedTaskDraft(),
    cwd: ' C:/工作区 ',
    objective: ' complete task ',
    scopeKind: 'selected' as const,
    accountIds: ['a', 'b', 'a'],
    initialAccountId: 'a',
    model: ' gpt-5.6-sol ',
    reasoningEffort: 'high',
    maxSwitches: '3',
  };
  assert.deepEqual(buildManagedTaskInput(draft), {
    objective: 'complete task',
    cwd: 'C:/工作区',
    accountScope: { kind: 'selected', accountIds: ['a', 'b'] },
    initialAccountId: 'a',
    model: 'gpt-5.6-sol',
    reasoningEffort: 'high',
    maxSwitches: 3,
  });
});

test('deduplicates live evidence and returns deterministic chronological order', () => {
  assert.deepEqual(
    mergeManagedTaskEvidence(
      [evidence('late', 20), evidence('replace', 10)],
      [evidence('early', 5), { ...evidence('replace', 10), kind: 'quota_warning' }],
    ).map((item) => [item.id, item.kind]),
    [
      ['early', 'activity'],
      ['replace', 'quota_warning'],
      ['late', 'activity'],
    ],
  );
});

test('allows cancellation only while queued or runtime-active', () => {
  for (const status of ['queued', 'preparing', 'running', 'draining', 'switching', 'resuming'] as const) {
    assert.equal(canCancelManagedTask(status), true);
  }
  for (const status of ['completed', 'failed', 'cancelled', 'needs_attention'] as const) {
    assert.equal(canCancelManagedTask(status), false);
  }
});
