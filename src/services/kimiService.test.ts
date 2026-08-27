import assert from 'node:assert/strict';
import test from 'node:test';

import { fromRawKimiAccount } from './kimiService.ts';

test('fromRawKimiAccount maps camelCase wire and strips token', () => {
  const mapped = fromRawKimiAccount({
    id: 'kimi-1',
    email: 'user@example.com',
    accessToken: 'must-not-leak',
    userId: 'uid-1',
    nickname: 'nick',
    expiresAt: 123,
    planType: 'MODERATO',
    status: 'active',
    statusReason: null,
    quota: {
      weeklyUsed: 1,
      weeklyLimit: 10,
      limits: [{ used: 2, limit: 20 }],
    },
    createdAt: 1,
    lastUsed: 2,
  });

  assert.equal(mapped.id, 'kimi-1');
  assert.equal(mapped.user_id, 'uid-1');
  assert.equal(mapped.access_token, '');
  assert.equal(mapped.plan_type, 'MODERATO');
  assert.equal(mapped.quota?.weeklyLimit, 10);
  assert.equal(mapped.quota?.limits?.[0]?.used, 2);
});
