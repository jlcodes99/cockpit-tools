import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';
import * as diagnostics from '../../utils/codexRequestDiagnostics';

function text(value: any): string {
  if (value == null || typeof value === 'boolean') return '';
  if (Array.isArray(value)) return value.map(text).join('');
  return typeof value === 'object' ? text(value.props?.children) : String(value);
}
const t = (key: string, values?: any) => values?.tier ? `${key}:${values.tier}` : key;
function harness(event: Record<string, unknown>) {
  const hook = loadHookModule(new URL('./CodexRequestDetailModal.tsx', import.meta.url), {
    'react-dom': { createPortal: (content: unknown) => content },
    'react-i18next': { useTranslation: () => ({ t }) },
    'lucide-react': Object.fromEntries(['Activity', 'CheckCircle2', 'ChevronDown', 'Clock3', 'Copy', 'FileJson', 'Gauge', 'Info', 'RefreshCw', 'Route', 'X', 'XCircle'].map((name) => [name, name])),
    '../../services/codexLocalAccessService': { getCodexLocalAccessRequestDetail: async () => null },
    '../../utils/codexRequestDiagnostics': diagnostics,
    '../../hooks/useEscClose': { useEscCloseTopmost: () => {} },
    '../../hooks/useModalFocusTrap': { useModalFocusTrap: () => {} },
    '../../hooks/useModalScrollLock': { useModalScrollLock: () => {} },
    '../ModalErrorMessage': { ModalErrorMessage: 'ModalErrorMessage' },
    '../SingleSelectDropdown': { SingleSelectDropdown: 'SingleSelectDropdown' },
    './CodexRequestTimeoutHint': { CodexRequestTimeoutHint: 'CodexRequestTimeoutHint' },
    './CodexOutputSpeed': { CodexOutputSpeed: 'CodexOutputSpeed' },
  }, { document: { body: {} } });
  hook.render(() => hook.exports.CodexRequestDetailModal({ event: { requestId: 'id', latencyMs: 100, ...event }, maskAccountText: String, onClose: () => {} }));
  return hook;
}

test('request details retain priority and raw default separately even without request-body recording', async () => {
  const hook = harness({ requestedServiceTier: 'priority', responseServiceTier: 'default', serviceTier: 'standard' });
  await settlePromises();
  const content = text(hook.flush());
  assert.match(content, /requestedTier:codex.speed.fast/);
  assert.match(content, /responseTier:default/);
  assert.match(content, /serviceTierHint/);
  assert.equal(content.includes('codex.speed.standard'), false);
  assert.equal(content.includes('legacyTier:'), false);
  hook.unmount();
});

test('legacy request details show unknown request/response sources instead of claiming standard processing', async () => {
  const hook = harness({ serviceTier: 'standard' });
  await settlePromises();
  const content = text(hook.flush());
  assert.match(content, /requestedTier:—/);
  assert.match(content, /responseTier:—/);
  assert.match(content, /legacyTier:standard/);
  assert.match(content, /legacyTierHint/);
  assert.equal(content.includes('codex.speed.standard'), false);
  hook.unmount();
});
