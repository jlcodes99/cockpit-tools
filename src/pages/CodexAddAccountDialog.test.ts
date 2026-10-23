import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';

type Node = { type: unknown; props: Record<string, any> };
function nodes(value: any): Node[] {
  if (!value || typeof value !== 'object') return [];
  if (Array.isArray(value)) return value.flatMap(nodes);
  return [{ type: value.type, props: value.props ?? {} }, ...nodes(value.props?.children)];
}

test('the existing add-account dialog no longer embeds official authorization', () => {
  const window = new EventTarget();
  let changed = 0;
  let closed = 0;
  const opened: string[] = [];
  window.addEventListener('chatgpt-plan-accounts-changed', () => changed++);
  const hook = loadHookModule(new URL('./CodexAddAccountDialog.tsx', import.meta.url), {
    'react-dom': { createPortal: (value: unknown) => value },
    '../components/codex/ChatGptPlanDialog': { ChatGptPlanDialog: 'PlanDialog' },
    '../components/ModalErrorMessage': { ModalErrorMessage: 'ErrorMessage' },
    '../components/MfaQuickCodeSelect': { MfaQuickCodeSelect: 'MfaSelect' },
    '../components/SingleSelectDropdown': { SingleSelectDropdown: 'Dropdown' },
    '../utils/reducedMotion': { scrollElementIntoView() {} },
    '../hooks/useEscClose': { useEscCloseTopmost() {} },
    '../hooks/useModalFocusTrap': { useModalFocusTrap() {} },
    '../hooks/useModalScrollLock': { useModalScrollLock() {} },
    '../services/codexTempLoginService': { CODEX_TEMP_LOGIN_STEPS: [] },
    '../utils/codexProviderPresets': {},
    '../utils/codexAccountProxy': { canUseCodexOAuthAccountProxy: () => false },
  }, { window, document: { body: {} }, Event });
  const tree = nodes(hook.render(() => hook.exports.CodexAddAccountDialog({
    showAddModal: true, addTab: 'token', addStatus: 'idle', tokenInput: '',
    CODEX_TOKEN_BATCH_EXAMPLE: '', CODEX_TOKEN_SESSION_EXAMPLE: '', CODEX_TOKEN_SINGLE_EXAMPLE: '',
    codexAddTargetGroup: { name: 'Ordinary Codex group' },
    t: (key: string) => key,
    openCodexAddModal: (tab: string) => opened.push(tab),
    closeCodexAddModal: () => closed++,
  })));
  assert.equal(tree.some((node) => node.type === 'PlanDialog'), false);
  assert.equal(tree.some((node) => node.props.children === 'chatgptPlan.addTab'), false);
  assert.equal(tree.some((node) => node.props.className === 'codex-add-target-group-hint'), true);
  assert.equal(changed, 0);
  assert.equal(closed, 0);
  assert.deepEqual(opened, []);
  hook.unmount();
});
