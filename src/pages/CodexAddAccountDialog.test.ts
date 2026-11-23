import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';
import { resolveOllamaLocalPlaceholderKey } from '../utils/codexProviderPresets';

type Node = { type: unknown; props: Record<string, any> };
function nodes(value: any): Node[] {
  if (!value || typeof value !== 'object') return [];
  if (Array.isArray(value)) return value.flatMap(nodes);
  return [{ type: value.type, props: value.props ?? {} }, ...nodes(value.props?.children)];
}

function loadDialog(window: EventTarget) {
  return loadHookModule(new URL('./CodexAddAccountDialog.tsx', import.meta.url), {
    'react-dom': { createPortal: (value: unknown) => value },
    '../components/codex/OpenAIUsageSettingsDialog': { OpenAIUsageKeyField: 'UsageKeyField' },
    '../components/codex/ChatGptPlanDialog': { ChatGptPlanDialog: 'PlanDialog' },
    '../components/ModalErrorMessage': { ModalErrorMessage: 'ErrorMessage' },
    '../components/MfaQuickCodeSelect': { MfaQuickCodeSelect: 'MfaSelect' },
    '../components/SingleSelectDropdown': { SingleSelectDropdown: 'Dropdown' },
    '../utils/reducedMotion': { scrollElementIntoView() {} },
    '../hooks/useEscClose': { useEscCloseTopmost() {} },
    '../hooks/useModalFocusTrap': { useModalFocusTrap() {} },
    '../hooks/useModalScrollLock': { useModalScrollLock() {} },
    '../services/codexTempLoginService': { CODEX_TEMP_LOGIN_STEPS: [] },
    '../utils/codexProviderPresets': { resolveOllamaLocalPlaceholderKey },
    '../utils/codexAccountProxy': { canUseCodexOAuthAccountProxy: () => false },
  }, { window, document: { body: {} }, Event });
}

test('the existing add-account dialog no longer embeds official authorization', () => {
  const window = new EventTarget();
  let changed = 0;
  let closed = 0;
  const opened: string[] = [];
  window.addEventListener('chatgpt-plan-accounts-changed', () => changed++);
  const hook = loadDialog(window);
  const tree = nodes(hook.render(() => hook.exports.CodexAddAccountDialog({
    showAddModal: true, addTab: 'token', addStatus: 'idle', tokenInput: '', apiBaseUrlInput: '', apiKeyInput: '',
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

test('the browser authorization error offers device authorization when the proxy is off', () => {
  const hook = loadDialog(new EventTarget());
  const portError = '1457 bind failed (os error 10013)';
  let starts = 0;
  const errorPanel = (oauthProxyEnabled: boolean) => {
    const tree = nodes(hook.render(() => hook.exports.CodexAddAccountDialog({
      showAddModal: true, addTab: 'oauth', addStatus: 'idle', apiBaseUrlInput: '', apiKeyInput: '',
      oauthMethod: 'browser', oauthPrepareError: portError, oauthProxyEnabled,
      oauthCompletingRef: { current: false }, oauthCallbackInput: '',
      t: (key: string) => key,
      handleStartDeviceAuth: () => starts++,
    })));
    const panel = tree.find((node) => node.props.className === 'add-status error'
      && nodes(node.props.children).some((child) => child.props.children === portError));
    assert.ok(panel);
    return nodes(panel.props.children);
  };
  const button = errorPanel(false).find((node) => node.type === 'button'
    && node.props.children === 'common.shared.oauth.deviceAuth');
  assert.ok(button);
  assert.equal(button.props.disabled, false);
  button.props.onClick();
  assert.equal(starts, 1);
  assert.equal(errorPanel(true).some((node) => node.props.children === 'common.shared.oauth.deviceAuth'), false);
  hook.unmount();
});
