import test from 'node:test';
import assert from 'node:assert/strict';
import { codexRemoteBinding } from './codexRemoteBinding';

test('quota cache identity changes with credentials or service and excludes plaintext keys', async () => {
  const first = await codexRemoteBinding('http://A/v1/', 'test-key-one');
  assert.equal(first, await codexRemoteBinding('http://A/v1', 'test-key-one'));
  assert.notEqual(first, await codexRemoteBinding('http://A/v1', 'test-key-two'));
  assert.notEqual(first, await codexRemoteBinding('http://B/v1', 'test-key-one'));
  assert.ok(!first.includes('test-key'));
});
