import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../../tests/helpers/reactHookHarness';
import { codexProxyCardPresentation } from '../../utils/codexProxyDisplay';
import type { CodexAccount } from '../../types/codex';
import type { CodexProxyRuntimeStatus } from '../../services/codexAccountProxyService';

function elements(value: any): any[] {
  if (!value || typeof value !== 'object') return [];
  if (Array.isArray(value)) return value.flatMap(elements);
  return [value, ...elements(value.props?.children)];
}

const account = { id: 'A', egress_proxy: { protocol: 'resource', name: 'Auto' } } as CodexAccount;
const status: CodexProxyRuntimeStatus = {
  sharedEntry: true, account: 'running', sidecar: 'running', desktop: 'running',
  accountPort: 1234, sidecarPort: 1234, desktopPort: 1234,
  accountNode: 'leaf', sidecarNode: 'leaf', desktopNode: 'leaf',
  accountSelection: { name: 'leaf', delayMs: 100, checkedAt: 1 },
  sidecarSelection: { name: 'leaf', delayMs: 42, checkedAt: 2 },
};

test('preview uses the card current-node measurement, includes its timestamp and measures only on click', () => {
  let measured = 0;
  const h = loadHookModule(new URL('./CodexProxyConnectionSummary.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
  });
  const show = (next: CodexProxyRuntimeStatus | null, failed = false, measuring = false) => h.render(() =>
    h.exports.CodexProxyConnectionSummary({ account, status: next, failed, measuring, onMeasure() { measured++; } }));
  const button = (tree: any) => elements(tree).find((node) => node.type === 'button');
  const view = show(status);
  const selected = codexProxyCardPresentation(account, status).nodes[0].selection!;
  assert.equal(elements(view).find((node) => node.props?.className === 'codex-proxy-preview-current-latency').props.children, `${selected.delayMs} ms`);
  assert.equal(elements(view).find((node) => node.type === 'time').props.dateTime, new Date(selected.checkedAt!).toISOString());
  assert.equal(measured, 0);
  assert.equal(button(view).props.disabled, false);
  button(view).props.onClick(); assert.equal(measured, 1);
  assert.equal(button(show(status, false, true)).props.disabled, true);
  assert.equal(button(show(status, true)).props.disabled, true);
  assert.equal(button(show(null)).props.disabled, true);
  const idle = { ...status, account: 'idle', sidecar: 'idle', desktop: 'idle' } as CodexProxyRuntimeStatus;
  assert.equal(button(show(idle)).props.disabled, true, 'stale selections cannot enable automatic engine startup');
  const failure = show({ ...status, sidecarSelection: { name: 'leaf', delayMs: null, checkedAt: 3 } });
  assert.equal(elements(failure).find((node) => node.props?.className === 'codex-proxy-preview-current-latency').props.children, 'common.failed');
  for (const proxySource of ['none', 'disabled'] as const) {
    const disconnected = show({ ...status, proxySource });
    assert.equal(button(disconnected).props.disabled, true, 'obsolete running fields cannot enable a check after proxy removal');
    assert.equal(elements(disconnected).some((node) => node.props?.className === 'codex-proxy-preview-current-latency'), false);
  }
});
