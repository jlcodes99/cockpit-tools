import assert from 'node:assert/strict';
import test from 'node:test';
import { createHash } from 'node:crypto';
import { CHALLENGE_BENCHMARKS, buildContextLedgerRecords } from './benchmarkChallenges.ts';

function expected(id: string) {
  const entry = CHALLENGE_BENCHMARKS.find((benchmark) => benchmark.id === id);
  assert.ok(entry);
  const rule = entry.rules[0];
  assert.equal(rule.kind, 'jsonFields');
  if (rule.kind !== 'jsonFields') throw new Error('Expected exact JSON fields');
  return Object.fromEntries(rule.fields.map((field) => [field.name, field.value]));
}

test('mathematical answer agrees with exhaustive individual-ball subsets, not only color formulas', () => {
  let all = 0;
  let eachColor = 0;
  let valid = 0;
  for (let a = 0; a < 11; a++)
    for (let b = a + 1; b < 12; b++)
      for (let c = b + 1; c < 13; c++)
        for (let d = c + 1; d < 14; d++)
          for (let e = d + 1; e < 15; e++) {
            all++;
            const sample = [a, b, c, d, e];
            const red = sample.filter((ball) => ball < 6).length;
            const blue = sample.filter((ball) => ball >= 6 && ball < 11).length;
            const green = sample.filter((ball) => ball >= 11).length;
            if (red && blue && green) {
              eachColor++;
              if (red > blue) valid++;
            }
          }
  assert.equal(eachColor, 2170);
  assert.deepEqual(expected('cockpitMathExactV1'), {
    validSubsets: valid, totalSubsets: all,
    probability: `${valid}/${all}`, conditionalProbability: `${valid / 10}/${eachColor / 10}`,
  });
});

test('code fixture answers cover holes versus null, aliasing, closure bindings and finally completion', () => {
  // Only this repository-authored fixture is executed; model responses are always data.
  const source = [2, , 0, 5];
  const alias = source;
  const copy = source.slice();
  alias[1] = 3;
  delete source[3];
  let calls = 0;
  const mapped = source.map((value, index) => { calls++; return (value ?? 9) + index; });
  const functions = [];
  for (let i = 0; i < 3; i++) functions.push(() => i);
  function settle() { try { return 7; } finally { return 11; } }
  assert.equal(3 in source, false);
  assert.equal(1 in copy, false);
  assert.equal(3 in copy, true);
  assert.equal(3 in mapped, false);
  const serialized = JSON.parse(JSON.stringify({
    source, copy, mapped, keys: Object.keys(mapped), calls,
    closures: functions.map((fn) => fn()), settled: settle(),
  }));
  assert.deepEqual(expected('cockpitCodeTraceV1'), serialized);
});

test('long ledger has dispersed target events and independent latest-version reconstruction', () => {
  const entry = CHALLENGE_BENCHMARKS.find((benchmark) => benchmark.id === 'cockpitContextLedgerV1')!;
  assert.ok(entry.prompt.length >= 25000, `${entry.prompt.length} characters`);
  assert.ok(entry.prompt.length < 50000, 'Keep the long-context fixture within a bounded size');
  const records = buildContextLedgerRecords();
  assert.deepEqual(buildContextLedgerRecords(), records, 'History prompt construction is deterministic');
  assert.equal(records.length, 261);
  const targetPositions = records.flatMap((row, index) => row.project === 'ORBIT' && row.period === '2026-09' ? [index] : []);
  assert.ok(targetPositions[0] < records.length / 10);
  assert.ok(targetPositions[targetPositions.length - 1] > records.length * 0.9);
  assert.ok(records.some((row) => row.project === 'ORBIT-ARCHIVE' && row.period === '2026-09'));
  assert.ok(records.some((row) => row.project === 'ORBIT' && row.period === '2026-08'));
  // Reconstruct by grouping and sorting, independently of the fixture's generation order.
  const target = [...new Map(records.filter((row) => row.project === 'ORBIT' && row.period === '2026-09')
    .map((row) => [row.eventId, row])).values()];
  const invoiceIds = [...new Set(target.map((row) => row.invoice))].sort();
  const posted = invoiceIds.map((id) => target.filter((row) => row.invoice === id)
    .sort((a, b) => b.revision - a.revision)[0]).filter((row) => row.state === 'posted');
  assert.deepEqual(expected(entry.id), {
    uniqueTargetEvents: target.length, activeInvoiceCount: posted.length,
    totalCents: posted.reduce((sum, row) => sum + row.amountCents, 0),
    largestInvoiceId: [...posted].sort((a, b) => b.amountCents - a.amountCents || a.invoice.localeCompare(b.invoice))[0].invoice,
    activeInvoiceIds: posted.map((row) => row.invoice),
  });
  assert.equal(posted.find((row) => row.invoice === 'A103')?.revision, 3, 'A posted revision restores a void');
  assert.ok(!posted.some((row) => row.invoice === 'A101' || row.invoice === 'A105'));
  assert.equal(target.length, 18);
  assert.equal(targetPositions.length, 21, 'Three duplicate deliveries are present');
});

test('resource planning answer is the first optimal schedule under exhaustive integer-time search', () => {
  // Independent exact feasibility search: enumerate starts in lexicographic order,
  // checking every occupied time slot. Include intentional idling and simultaneous finishes.
  const jobs = [
    { duration: 3, workers: 2, before: [], tail: 12 },
    { duration: 2, workers: 1, before: [], tail: 10 },
    { duration: 4, workers: 2, before: [0], tail: 9 },
    { duration: 3, workers: 1, before: [1], tail: 8 },
    { duration: 2, workers: 1, before: [0, 1], tail: 5 },
    { duration: 4, workers: 3, before: [2, 3], tail: 5 },
    { duration: 2, workers: 1, before: [4], tail: 3 },
    { duration: 1, workers: 2, before: [5, 6], tail: 1 },
  ];
  const find = (deadline: number): number[] | null => {
    const usage = Array<number>(deadline).fill(0);
    const starts: number[] = [];
    const visit = (index: number): number[] | null => {
      if (index === jobs.length) return [...starts];
      const job = jobs[index];
      const earliest = Math.max(0, ...job.before.map((dependency) => starts[dependency] + jobs[dependency].duration));
      for (let start = earliest; start <= deadline - job.tail; start++) {
        if (usage.slice(start, start + job.duration).some((used) => used + job.workers > 3)) continue;
        for (let time = start; time < start + job.duration; time++) usage[time] += job.workers;
        starts[index] = start;
        const found = visit(index + 1);
        if (found) return found;
        for (let time = start; time < start + job.duration; time++) usage[time] -= job.workers;
      }
      return null;
    };
    return visit(0);
  };
  assert.equal(find(13), null, 'No schedule can finish in 13 time units');
  assert.deepEqual(expected('cockpitMultiStepV1'), {
    minimumFinish: 14, startTimes: find(14),
    totalWorkerTime: jobs.reduce((sum, job) => sum + job.duration * job.workers, 0),
  });
});

test('all original challenges separate local references and expected answers from immutable request prompts', () => {
  assert.equal(CHALLENGE_BENCHMARKS.length, 4);
  assert.equal(new Set(CHALLENGE_BENCHMARKS.map((entry) => entry.prompt)).size, 4);
  for (const entry of CHALLENGE_BENCHMARKS) {
    assert.equal(entry.suite, 'Cockpit');
    assert.equal(entry.graderVersion, 'cockpit-exact-v2');
    assert.equal(entry.rules.length, 1);
    assert.equal(entry.rules[0].kind, 'jsonFields');
    assert.ok(entry.reference);
    assert.ok(!entry.prompt.includes(entry.reference!));
    const answer = JSON.stringify(expected(entry.id));
    assert.ok(!entry.prompt.includes(answer), entry.id);
    assert.equal(entry.sourceRevision, 'local-v1');
  }
});

test('versioned prompt fingerprints protect exact-match historical grading from silent fixture edits', () => {
  assert.deepEqual(CHALLENGE_BENCHMARKS.map((entry) => [entry.id, createHash('sha256').update(entry.prompt).digest('hex')]), [
    ['cockpitMathExactV1', '8b2fb5faa9e2b3dfead85302c39854f758590d206b248c6568657ab0239cf103'],
    ['cockpitCodeTraceV1', '13e1a572d2c6bff939eb4fa18e97e2a18d72cf54e13c6a5cea143da0bda2a8d9'],
    ['cockpitContextLedgerV1', 'f53647a69346981d43deaf17572879f5ead3f2daaa29da3b347ae4630b85efc3'],
    ['cockpitMultiStepV1', '96756e353d7b446db2b86efa5811016b375e677947dcdecfafcdc9b76e650012'],
  ]);
});
