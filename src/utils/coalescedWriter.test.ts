import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createCoalescedWriter } from './coalescedWriter.ts';

test('a burst of 100 cache updates writes the latest value once', (context) => {
  context.mock.timers.enable({ apis: ['setTimeout'] });
  const writes: number[] = [];
  const writer = createCoalescedWriter<number>((value) => { writes.push(value); });
  for (let i = 0; i < 100; i++) writer.schedule(i);
  assert.deepEqual(writes, []);
  context.mock.timers.tick(500);
  assert.deepEqual(writes, [99]);
});

test('continuous updates cannot postpone persistence, and close flushes pending removal', (context) => {
  context.mock.timers.enable({ apis: ['setTimeout'] });
  const writes: string[][] = [];
  const writer = createCoalescedWriter<string[]>((value) => { writes.push(value); });
  writer.schedule(['old']);
  context.mock.timers.tick(400);
  writer.schedule(['new']);
  context.mock.timers.tick(100);
  assert.deepEqual(writes, [['new']]);
  writer.schedule([]);
  writer.flush();
  context.mock.timers.tick(500);
  assert.deepEqual(writes, [['new'], []]);
});
