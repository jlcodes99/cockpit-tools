import assert from 'node:assert/strict';
import test from 'node:test';
import { authorizationFallbackReasonKey, authorizationFallbackSummaryKey, formatRequestPayloadBody, hasRequestFirstResponse, recordedServiceTierAdaptation, requestAuthorizationRouteKey, requestDiagnosticErrorSource, requestDiagnosticPhaseKey, requestServiceTierPresentation, requestedServiceTierLabelKey } from './codexRequestDiagnostics';
import type { CodexLocalAccessRequestDetail } from '../types/codexLocalAccess';

test('authorization fallback labels require recorded same-account route evidence', () => {
  const trace = { reason: 'eligibility_rejected', fromAccountId: 'source-a', toAccountId: 'source-a', fromRoute: 'official', toRoute: 'local_gateway' };
  assert.equal(authorizationFallbackSummaryKey(trace), 'codex.requestDiagnostics.authorizationFallback.summary');
  assert.equal(authorizationFallbackSummaryKey(null), null);
  assert.equal(authorizationFallbackSummaryKey({ ...trace, toAccountId: 'another-account' }), null);
  assert.equal(authorizationFallbackSummaryKey({ ...trace, fromRoute: 'local_gateway' }), null);
  assert.equal(authorizationFallbackSummaryKey({ ...trace, fromAccountId: '', toAccountId: '' }), null);
  for (const [reason, expected] of [
    ['eligibility_rejected', 'notEligible'], ['authorization_failed', 'authorization'],
    ['capability_unsupported', 'unsupported'], ['official_throttled', 'limited'],
    ['official_unavailable', 'unavailable'], ['circuit_open', 'paused'],
    ['constructor', 'unavailable'], ['secrets.other', 'unavailable'],
  ]) assert.equal(authorizationFallbackReasonKey(reason), `codex.requestDiagnostics.authorizationFallback.reasons.${expected}`);
});

test('historical access routes use the recorded endpoint identity, including each retry attempt', () => {
  assert.equal(requestAuthorizationRouteKey('original-account', 'official'), 'codex.requestDiagnostics.routeOfficial');
  assert.equal(requestAuthorizationRouteKey('original-account', 'local_gateway'), 'codex.requestDiagnostics.routeLocal');
  assert.equal(requestAuthorizationRouteKey('chatgpt_plan_saved-grant'), 'codex.requestDiagnostics.routeOfficial');
  assert.equal(requestAuthorizationRouteKey('local-source'), 'codex.requestDiagnostics.routeLocal');
  assert.equal(requestAuthorizationRouteKey('codex_apikey_custom'), 'codex.requestDiagnostics.routeLocal');
  assert.equal(requestAuthorizationRouteKey(undefined), 'common.unknown');
  assert.equal(requestAuthorizationRouteKey('  '), 'common.unknown');
  const attempts = ['local-source', 'chatgpt_plan_saved-grant'];
  assert.deepEqual(attempts.map((id) => requestAuthorizationRouteKey(id)), ['codex.requestDiagnostics.routeLocal', 'codex.requestDiagnostics.routeOfficial']);
});

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
  assert.equal(requestedServiceTierLabelKey('ULTRAFAST'), 'codex.speed.ultrafast');
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
  assert.equal(requestDiagnosticPhaseKey('constructor'), 'codex.requestDiagnostics.phases.unknown');
  assert.equal(requestDiagnosticPhaseKey('routing'), 'codex.requestDiagnostics.phases.routing');
  assert.equal(requestDiagnosticPhaseKey('upstream'), 'codex.requestDiagnostics.phases.upstream');
  assert.equal(requestDiagnosticPhaseKey('dial'), 'codex.requestDiagnostics.phases.upstream_connect');
  assert.equal(requestDiagnosticPhaseKey('client_cancelled'), 'codex.requestDiagnostics.phases.client_canceled');
});

test('error source uses recorded stages and leaves ambiguous native or historical errors unknown', () => {
  for (const [phase, expected] of [
    ['routing', 'gateway'], ['selection', 'gateway'], ['identity', 'gateway'],
    ['upstream_response', 'upstream'], ['upstream', 'unknown'], ['dial', 'transport'],
    ['stream_read', 'transport'], ['client_cancelled', 'client'], ['downstream', 'client'],
    ['unexpected', 'unknown'],
  ]) assert.equal(requestDiagnosticErrorSource({ failurePhase: phase } as CodexLocalAccessRequestDetail), expected);
  assert.equal(requestDiagnosticErrorSource(null), 'unknown');
  assert.equal(requestDiagnosticErrorSource({ attempts: [], payloads: [] } as unknown as CodexLocalAccessRequestDetail), 'unknown');
});

test('displaying a truncated or non-JSON snapshot preserves exactly the captured text', () => {
  assert.equal(formatRequestPayloadBody({ body: '{"input":', truncated: true }), '{"input":');
  assert.equal(formatRequestPayloadBody({ body: 'raw data', truncated: false }), 'raw data');
  assert.equal(formatRequestPayloadBody({ body: '{"input":"hello"}', truncated: false }), '{\n  "input": "hello"\n}');
});


test('speed adjustment requires explicit recorded evidence, independent of default upstream feedback', () => {
  const adaptation = { requestedTier: 'priority', sentTier: 'default', reason: 'official_default_only' } as const;
  assert.equal(recordedServiceTierAdaptation(adaptation), adaptation);
  assert.equal(recordedServiceTierAdaptation(), null);
  assert.equal(recordedServiceTierAdaptation(null), null);
  assert.equal(recordedServiceTierAdaptation({ ...adaptation, requestedTier: '' }), null);
  assert.equal(recordedServiceTierAdaptation({ ...adaptation, requestedTier: '  ' }), null);
  const event = { requestedServiceTier: 'priority', responseServiceTier: 'default' };
  assert.equal(requestServiceTierPresentation(event).requested, 'priority');
});
