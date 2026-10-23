import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../../tests/helpers/reactHookHarness';

function text(node: any): string {
  if (node == null || typeof node === 'boolean') return '';
  if (Array.isArray(node)) return node.map(text).join('');
  return typeof node === 'object' ? text(node.props?.children) : String(node);
}
const h = loadHookModule(new URL('./CodexCapabilityFallbackTrace.tsx', import.meta.url), {
  'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
});
const props = { accountDisplayNames: new Map([['a', 'source@example.com'], ['b', 'backup@example.com']]), maskAccountText: (value: string) => `masked:${value}` };
const trace = { reason: 'preflight', capabilities: ['file_search'], fromAccountId: 'a', fromModel: 'Original-Model', toAccountId: 'b', toModel: 'Mapped-Model' };

test('old attempts do not invent capability fallback from model or account changes', () => {
  assert.equal(h.exports.CodexCapabilityFallbackTrace(props), null);
  assert.equal(h.exports.CodexCapabilityFallbackTrace({ ...props, trace: null }), null);
});

test('fallback details show only the recorded reason, account, model and capabilities and obey privacy masking', () => {
  const content = text(h.exports.CodexCapabilityFallbackTrace({ ...props, trace }));
  for (const value of ['capabilityFallback.reasonPreflight', 'masked:source@example.com', 'masked:backup@example.com', 'Original-Model', 'Mapped-Model', 'capabilityFallback.capability.file_search']) assert.ok(content.includes(value), value);
  const upstream = text(h.exports.CodexCapabilityFallbackTrace({ ...props, trace: { ...trace, reason: 'upstream_unsupported' } }));
  assert.match(upstream, /capabilityFallback.reasonUpstream/);
  const unknown = text(h.exports.CodexCapabilityFallbackTrace({ ...props, trace: { ...trace, reason: 'future', capabilities: ['future_tool'] } }));
  assert.match(unknown, /capabilityFallback.unknown/);
  assert.equal(unknown.includes('reasonPreflight'), false);
  assert.equal(unknown.includes('future_tool'), false);
});
