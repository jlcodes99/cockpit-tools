import assert from 'node:assert/strict';
import test from 'node:test';
import type { OpenCodeGoConnection } from '../types/openCodeGo.ts';
import {
  createOpenCodeGoConnectionSlots,
  filterAndSortOpenCodeGoConnections,
  normalizeOpenCodeGoConnectionName,
  resolveOpenCodeGoConnectionTier,
} from './openCodeGoConnections.ts';

function connection(
  id: string,
  options: Partial<OpenCodeGoConnection> = {},
): OpenCodeGoConnection {
  return {
    id,
    name: `Connection ${id}`,
    keyHint: `${id.slice(0, 2)}****`,
    createdAt: 100,
    updatedAt: 100,
    ...options,
  };
}

test('every stored connection receives a slot without an artificial cap', () => {
  const connections = Array.from({ length: 5 }, (_, index) => connection(String(index + 1)));
  const slots = createOpenCodeGoConnectionSlots(connections);
  assert.equal(slots.length, 5);
  assert.deepEqual(slots.map((slot) => slot.connection?.id), ['1', '2', '3', '4', '5']);
});

test('blank names receive their one-based connection fallback', () => {
  assert.equal(normalizeOpenCodeGoConnectionName('  ', 2), 'Connection 3');
  assert.equal(normalizeOpenCodeGoConnectionName(' Primary ', 0), 'Primary');
});

test('connection tier reflects quota and error state', () => {
  assert.equal(resolveOpenCodeGoConnectionTier(connection('healthy', {
    quota: {
      rolling: { usedPercent: 25, remainingPercent: 75, resetsAt: 1 },
      weekly: { usedPercent: 50, remainingPercent: 50, resetsAt: 2 },
      monthly: { usedPercent: 75, remainingPercent: 25, resetsAt: 3 },
      status: 'available', queriedAt: 4,
    },
  })), 'available');
  assert.equal(resolveOpenCodeGoConnectionTier(connection('spent', {
    quota: {
      rolling: { usedPercent: 100, remainingPercent: 0, resetsAt: 1 },
      weekly: { usedPercent: 50, remainingPercent: 50, resetsAt: 2 },
      monthly: { usedPercent: 75, remainingPercent: 25, resetsAt: 3 },
      status: 'exhausted', queriedAt: 4,
    },
  })), 'exhausted');
  assert.equal(resolveOpenCodeGoConnectionTier(connection('broken', {
    quotaError: { kind: 'authentication', occurredAt: 4 },
  })), 'error');
  assert.equal(resolveOpenCodeGoConnectionTier(connection('new')), 'pending');
});

test('query filters by name, masked email, key hint, and tier', () => {
  const input = [
    connection('alpha', { name: 'Primary', emailHint: 'p***@example.com', keyHint: 'ocg-****-one' }),
    connection('beta', {
      name: 'Backup', keyHint: 'ocg-****-two',
      quotaError: { kind: 'network', occurredAt: 8 },
    }),
  ];
  const filter = (query: string, tier: 'all' | 'error') =>
    filterAndSortOpenCodeGoConnections(input, {
      query, tier, sortBy: 'name', sortDirection: 'asc',
    }).map((item) => item.id);
  assert.deepEqual(filter('backup', 'all'), ['beta']);
  assert.deepEqual(filter('two', 'all'), ['beta']);
  assert.deepEqual(filter('p***', 'all'), ['alpha']);
  assert.deepEqual(filter('', 'error'), ['beta']);
});

test('sorting is deterministic for name, created time, and remaining quota', () => {
  const input = [
    connection('z', {
      name: 'Zulu', createdAt: 20,
      quota: {
        rolling: { usedPercent: 80, remainingPercent: 20, resetsAt: 1 },
        weekly: { usedPercent: 50, remainingPercent: 50, resetsAt: 2 },
        monthly: { usedPercent: 30, remainingPercent: 70, resetsAt: 3 },
        status: 'available', queriedAt: 4,
      },
    }),
    connection('a', {
      name: 'Alpha', createdAt: 10,
      quota: {
        rolling: { usedPercent: 20, remainingPercent: 80, resetsAt: 1 },
        weekly: { usedPercent: 10, remainingPercent: 90, resetsAt: 2 },
        monthly: { usedPercent: 40, remainingPercent: 60, resetsAt: 3 },
        status: 'available', queriedAt: 4,
      },
    }),
  ];
  const sort = (sortBy: 'name' | 'created_at' | 'remaining', sortDirection: 'asc' | 'desc') =>
    filterAndSortOpenCodeGoConnections(input, {
      query: '', tier: 'all', sortBy, sortDirection,
    }).map((item) => item.id);
  assert.deepEqual(sort('name', 'asc'), ['a', 'z']);
  assert.deepEqual(sort('created_at', 'desc'), ['z', 'a']);
  assert.deepEqual(sort('remaining', 'desc'), ['a', 'z']);
});
