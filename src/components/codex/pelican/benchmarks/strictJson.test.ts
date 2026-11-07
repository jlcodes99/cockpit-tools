import assert from 'node:assert/strict';
import test from 'node:test';
import { exactJsonEqual, parseExactJson } from './strictJson.ts';

test('exact JSON accepts equivalent ordering and numeric syntax without coercing values', () => {
  assert.ok(exactJsonEqual(parseExactJson(' {"a":[1e1,2.00,null,true,"x"], "b":{"c":-0}} '), { b: { c: 0 }, a: [10, 2, null, true, 'x'] }));
  assert.ok(exactJsonEqual(parseExactJson('{"__proto__":7,"a\\"b":2}'), JSON.parse('{"__proto__":7,"a\\"b":2}')));
  assert.equal(exactJsonEqual(parseExactJson('{"x":"3"}'), { x: 3 }), false);
  assert.equal(exactJsonEqual(parseExactJson('{"x":[2,1]}'), { x: [1, 2] }), false);
  assert.equal(exactJsonEqual(parseExactJson('{"x":{"a":1,"extra":0}}'), { x: { a: 1 } }), false);
  assert.equal(exactJsonEqual(parseExactJson('{"x":{"0":1}}'), { x: [1] }), false);
});

test('ambiguous keys, rounded numbers, excessive nesting and non-JSON are rejected', () => {
  for (const input of ['{"x":1,"x":2}', '{"x":1,"\\u0078":2}', '{"x":{"a":1,"a":2}}',
    '{"x":59323.00000000000000001}', '9007199254740993', '1e999', '1e-999',
    '{"x":1,}', '[1,]', '{"x":01}', '{"x":true} trailing', 'undefined', 'NaN', '"bad\nstring"',
    '['.repeat(66) + '0' + ']'.repeat(66), '{"x":"unterminated}', '\u00a0{}', '']) {
    assert.throws(() => parseExactJson(input), input);
  }
});


test('long decimals are checked without end-anchored zero-run backtracking', () => {
  const started = performance.now();
  assert.throws(() => parseExactJson('0.1' + '0'.repeat(60000) + '1'));
  assert.equal(parseExactJson('0.1' + '0'.repeat(60000)), 0.1);
  // A generous bound catches the old ~1 second quadratic scan, while avoiding a
  // microbenchmark threshold; normal work is a handful of bounded linear passes.
  assert.ok(performance.now() - started < 800, 'bounded replies must not stall local grading');
});
