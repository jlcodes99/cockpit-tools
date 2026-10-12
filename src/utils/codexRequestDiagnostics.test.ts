import assert from 'node:assert/strict';
import test from 'node:test';
import { formatRequestPayloadBody, hasRequestFirstResponse, requestDiagnosticPhaseKey, requestServiceTierPresentation, requestedServiceTierLabelKey } from './codexRequestDiagnostics';

test('requested and returned service tiers stay independent, without inferring old log sources', () => {
  assert.deepEqual(requestServiceTierPresentation({ requestedServiceTier: 'priority', responseServiceTier: 'default', serviceTier: 'standard' }),
    { requested: 'priority', response: 'default', legacy: '' });
  assert.deepEqual(requestServiceTierPresentation({ serviceTier: 'standard' }),
    { requested: '', response: '', legacy: 'standard' });
  assert.deepEqual(requestServiceTierPresentation({ responseServiceTier: 'default' }),
    { requested: '', response: 'default', legacy: '' });
  assert.deepEqual(requestServiceTierPresentation({}), { requested: '', response: '', legacy: '' });
  assert.equal(requestedServiceTierLabelKey('priority'), 'codex.speed.fast');
  assert.equal(requestedServiceTierLabelKey('FAST'), 'codex.speed.fast');
  assert.equal(requestedServiceTierLabelKey('auto'), 'codex.contextOverride.automatic');
  assert.equal(requestedServiceTierLabelKey('default'), 'codex.speed.standard');
  assert.equal(requestedServiceTierLabelKey('flex'), null);
});

test('first-response timing preserves observed zero and leaves legacy or invalid measurements unknown', () => {
  assert.equal(hasRequestFirstResponse(0), true);
  assert.equal(hasRequestFirstResponse(500), true);
  for (const value of [undefined, null, -1, NaN, Infinity]) assert.equal(hasRequestFirstResponse(value), false);
});

test('unknown failure stages do not generate untranslated or untrusted translation keys', () => {
  assert.equal(requestDiagnosticPhaseKey('stream_read'), 'codex.requestDiagnostics.phases.stream_read');
  assert.equal(requestDiagnosticPhaseKey('other.secret'), 'codex.requestDiagnostics.phases.unknown');
  assert.equal(requestDiagnosticPhaseKey(null), 'codex.requestDiagnostics.phases.unknown');
});

test('displaying a truncated or non-JSON snapshot preserves exactly the captured text', () => {
  assert.equal(formatRequestPayloadBody({ body: '{"input":', truncated: true }), '{"input":');
  assert.equal(formatRequestPayloadBody({ body: 'raw data', truncated: false }), 'raw data');
  assert.equal(formatRequestPayloadBody({ body: '{"input":"hello"}', truncated: false }), '{\n  "input": "hello"\n}');
});
