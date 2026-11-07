import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../../tests/helpers/reactHookHarness';

test('official markers distinguish active, pending and invalid links without hiding the original account', () => {
  const hook = loadHookModule(new URL('./CodexOfficialBadge.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
  });
  const account = { id: 'source', email: 'fixture@example.com', plan_type: 'plus' };
  const grant = { id: 'grant', source_account_id: account.id, connected: true, needs_reauth: false, source_binding_valid: true };
  for (const [status, patch, expected] of [
    ['valid', {}, 'is-active'],
    ['pending', { source_binding_valid: false }, 'is-pending'],
    ['invalid', { source_binding_valid: false, source_binding_error: 'CHATGPT_PLAN_SOURCE_CHANGED' }, 'is-invalid'],
  ] as const) {
    const node = hook.render(() => hook.exports.CodexAccountOfficialBadge({ account, grants: [{ ...grant, ...patch }], memberIds: ['chatgpt_plan_grant'] }));
    assert.equal(node.props.status, status);
    const badge = node.type(node.props);
    assert.ok(badge.props.className.includes(expected));
    assert.equal(badge.props.children[1].props.children, 'chatgptPlan.officialFlow.badge');
  }
  assert.equal(account.plan_type, 'plus');
  hook.unmount();
});

test('valid authorization stays highlighted with either service mode or no membership; disconnect removes it', () => {
  const hook = loadHookModule(new URL('./CodexOfficialBadge.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
  });
  const account = { id: 'source' };
  const grant = { id: 'grant', source_account_id: 'source', source_binding_valid: true, connected: true, needs_reauth: false };
  for (const [patch, members, state, className] of [
    [{}, [], 'unused', 'is-active'], [{}, ['source'], 'unused', 'is-active'],
    [{}, ['chatgpt_plan_grant'], 'used', 'is-active'],
    [{ needs_reauth: true }, ['chatgpt_plan_grant'], 'reauthorize', 'is-invalid'],
    [{ check_error: 'CHATGPT_PLAN_NETWORK' }, ['chatgpt_plan_grant'], 'used', 'is-active'],
  ] as const) {
    const node = hook.render(() => hook.exports.CodexAccountOfficialBadge({ account, grants: [{ ...grant, ...patch }], memberIds: [...members] }));
    assert.equal(node.props.state, state);
    const badge = node.type(node.props);
    if (className) assert.ok(badge.props.className.includes(className));
    else assert.equal(badge.props.className, 'codex-official-badge');
  }
  assert.equal(hook.render(() => hook.exports.CodexAccountOfficialBadge({ account, grants: [{ ...grant, connected: false }], memberIds: [] })), null);
  assert.equal(hook.render(() => hook.exports.CodexAccountOfficialBadge({ account, grants: [{ ...grant, source_account_id: null }], memberIds: [] })), null);
  hook.unmount();
});
