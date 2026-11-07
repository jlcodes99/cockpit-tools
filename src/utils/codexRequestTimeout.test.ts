import assert from 'node:assert/strict';
import test from 'node:test';
import { requestTimeoutDiagnostic } from './codexRequestDiagnostics';

test('stream-open diagnostic reads actual Go duration, including ms and compound units', () => {
  for (const [duration, seconds] of [['60s', 60], ['1500ms', 1.5], ['1m30s', 90], ['10m0s', 600]] as const) {
    assert.deepEqual(requestTimeoutDiagnostic({ httpStatus: 504, errorMessage: `upstream timed out in stream_open attempt=1/1 after ${duration}` }),
      { kind: 'open', seconds, field: 'sidecarStreamOpenTimeoutMs' });
  }
  assert.deepEqual(requestTimeoutDiagnostic({ errorCategory: 'upstream_first_byte_timeout' }),
    { kind: 'open', seconds: undefined, field: 'sidecarStreamOpenTimeoutMs' });
});

test('idle, image open, cancellation and unrelated 504 remain distinct', () => {
  assert.deepEqual(requestTimeoutDiagnostic({ errorMessage: 'upstream timed out in stream_idle after 2m0s' }),
    { kind: 'idle', seconds: 120, field: 'sidecarStreamIdleTimeoutMs' });
  assert.equal(requestTimeoutDiagnostic({ errorMessage: 'upstream timed out in stream_open after 60s', requestKind: 'image_generation' })?.field, 'sidecarImageStreamOpenTimeoutMs');
  assert.equal(requestTimeoutDiagnostic({ errorMessage: 'upstream timed out in stream_idle after 60s', requestKind: 'image_edit' })?.field, 'sidecarImageStreamIdleTimeoutMs');
  assert.deepEqual(requestTimeoutDiagnostic({ httpStatus: 504, failurePhase: 'client_canceled', errorCategory: 'upstream_first_byte_timeout' }), { kind: 'canceled' });
  assert.deepEqual(requestTimeoutDiagnostic({ httpStatus: 499, errorCategory: 'upstream_first_byte_timeout' }), { kind: 'canceled' });
  for (const errorMessage of ['504 Gateway Timeout', 'context canceled', 'deadline exceeded']) {
    assert.deepEqual(requestTimeoutDiagnostic({ httpStatus: 504, errorMessage }), { kind: 'gateway' });
  }
  assert.equal(requestTimeoutDiagnostic({ success: true, errorCategory: 'upstream_first_byte_timeout' }), null);
  assert.equal(requestTimeoutDiagnostic({ httpStatus: 500 }), null);
});
