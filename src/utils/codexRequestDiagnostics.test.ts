import assert from 'node:assert/strict';
import test from 'node:test';
import { formatRequestPayloadBody, hasRequestFirstResponse, requestDiagnosticPhaseKey } from './codexRequestDiagnostics';

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
