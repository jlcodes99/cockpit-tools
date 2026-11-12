import assert from 'node:assert/strict';
import test from 'node:test';
import type { Account } from '../types/account';
import { createAntigravityAccountComparator } from './antigravityAccountSort';

test('switch-time order uses successful switches and places unknown times last', () => {
  const accounts = [
    { id: 'unknown', created_at: 1, last_used: 300 },
    { id: 'latest', created_at: 2, last_used: 200, last_switched_at: 30 },
    { id: 'oldest', created_at: 3, last_used: 100, last_switched_at: 10 },
  ] as Account[];
  const options = {
    displayGroups: [],
    currentAccountId: 'latest',
  };
  const order = (sortBy: string, sortDirection: 'asc' | 'desc') =>
    accounts.slice().sort(createAntigravityAccountComparator({
      ...options,
      sortBy,
      sortDirection,
    })).map((account) => account.id);

  assert.deepEqual(order('last_switched_at', 'asc'), ['oldest', 'latest', 'unknown']);
  assert.deepEqual(order('last_switched_at', 'desc'), ['latest', 'oldest', 'unknown']);
  assert.deepEqual(order('created_at', 'asc'), ['latest', 'unknown', 'oldest']);
});
