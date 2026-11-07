import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../../tests/helpers/reactHookHarness';
import { requestTimeoutDiagnostic } from '../../utils/codexRequestDiagnostics';

function nodes(value: any): any[] {
  if (!value || typeof value !== 'object') return [];
  if (Array.isArray(value)) return value.flatMap(nodes);
  return [value, ...nodes(value.props?.children)];
}

test('timeout hint shows the recorded threshold and navigates to the matching image/text field', () => {
  let target = '';
  const h = loadHookModule(new URL('./CodexRequestTimeoutHint.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string, args?: any) => `${key}${args?.seconds != null ? `:${args.seconds}` : ''}${args?.setting ? `:${args.setting}` : ''}` }) },
    '../../utils/codexRequestDiagnostics': { requestTimeoutDiagnostic },
  });
  let event: any = { errorMessage: 'upstream timed out in stream_open attempt=1/1 after 1m30s', requestKind: 'text' };
  h.render(() => h.exports.CodexRequestTimeoutHint({ event, onOpenSettings: (field: string) => { target = field; } }));
  let tree = nodes(h.flush());
  assert.ok(tree.some((node) => node.props?.children === 'codex.requestDiagnostics.timeout.threshold:90'));
  tree.find((node) => node.type === 'button').props.onClick();
  assert.equal(target, 'sidecarStreamOpenTimeoutMs');
  event = { ...event, requestKind: 'image_edit' };
  tree = nodes(h.flush());
  assert.ok(tree.some((node) => node.props?.children === 'codex.requestDiagnostics.timeout.open:codex.apiService.timeouts.imageOpen'));
  tree.find((node) => node.type === 'button').props.onClick();
  assert.equal(target, 'sidecarImageStreamOpenTimeoutMs');
  event = { httpStatus: 504, errorMessage: 'upstream unavailable' };
  tree = nodes(h.flush());
  assert.ok(tree.some((node) => node.props?.children === 'codex.requestDiagnostics.timeout.gateway:codex.apiService.timeouts.streamOpen'));
  assert.equal(tree.some((node) => node.type === 'button'), false);
  assert.equal(tree.some((node) => String(node.props?.children).includes('.threshold')), false);
});
