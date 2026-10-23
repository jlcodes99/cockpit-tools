import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { SCORED_BENCHMARKS, LEGACY_BENCHMARKS } from './benchmarks/benchmarkCases.ts';
import { gradePelicanReply, PELICAN_GRADING_MAX_CHARS } from './pelicanScoring.ts';
import { CANDY_REASONING_PROMPT, PELICAN_BENCHMARKS, pelicanBenchmark } from './pelicanBenchmarks.ts';

const prompt = (id: string) => {
  const benchmark = SCORED_BENCHMARKS.find((entry) => entry.id === id);
  assert.ok(benchmark, id);
  return benchmark.prompt;
};
const grade = (id: string, reply: string) => {
  const result = gradePelicanReply(prompt(id), reply);
  assert.ok(result);
  return result;
};

test('all twelve frozen benchmark cases grade known correct replies locally', () => {
  const replies = {
    ifeval1075: '{"advertisement":"Soft comfort for your baby"}',
    ifeval102: '* A: My gown is ready.\n* B: My sweats are ready.\n* A: Let us go.',
    ifeval1005: Array.from({ length: 12 }, (_, index) => `[field${index}]`).join(' '),
    ifeval1001: 'Hie thee to Kyoto and greet the dawn. Then seek the gardens of Tokyo.',
    simple1: 'The ice melts.\nAnswer: B',
    simple2: 'Both balls have landed.\nAnswer: A',
    simple4: 'Both sisters lie.\nAnswer: C',
    simple8: 'All five fruits are gone.\nAnswer: F',
    livezebra2: 'By elimination.\n<solution>1, filmmaking, police-officer, journalist</solution>',
    livezebra5: '<solution>romance, designer, nurse, 3</solution>',
    livetriangle: 'Parallel cuts create three regions. **3**',
    livesquare: 'The diagonals create four regions. **4**',
  };
  assert.equal(LEGACY_BENCHMARKS.length, 12);
  for (const [id, reply] of Object.entries(replies)) {
    const result = grade(id, reply);
    assert.equal(result.status, 'passed', id);
    assert.equal(result.passed, result.total, id);
    assert.equal(result.version, 'cockpit-strict-v1');
  }
});

test('IFEval checks the declared constraints, including boundary counts and empty output', () => {
  assert.equal(grade('ifeval1075', '```JSON\n{"ad":"soft"}\n```').status, 'passed');
  assert.equal(grade('ifeval1075', 'Here is JSON: {"ad":"soft"}').status, 'failed');
  assert.equal(grade('ifeval1075', '{"ad":undefined}').status, 'failed');
  assert.equal(grade('ifeval1075', '{"ad":"soft"}\nExtra text').status, 'failed');
  assert.equal(grade('ifeval1001', 'Kyoto, then Tokyo.').status, 'failed');
  assert.equal(grade('ifeval1001', '').status, 'failed');
  assert.equal(grade('ifeval1001', ' \n\t').status, 'failed');
  assert.equal(grade('ifeval102', '* one\n* two').status, 'failed');
  assert.equal(grade('ifeval102', '* one\n* two\n* three\n* four').status, 'failed');
  assert.equal(grade('ifeval102', '- one\n- two\n- three').status, 'passed');
  assert.equal(grade('ifeval102', '* one\r\n* two\r\n* three').status, 'passed');
  assert.equal(grade('ifeval102', '**one**\n**two**\n**three**').status, 'failed');
  for (const count of [11, 12, 13]) {
    const result = grade('ifeval1005', '[field] '.repeat(count));
    assert.equal(result.status, count < 12 ? 'failed' : 'passed');
  }
  assert.equal(grade('ifeval1005', '['.repeat(PELICAN_GRADING_MAX_CHARS)).status, 'failed');
  assert.equal(grade('ifeval1005', '[\n]'.repeat(12)).status, 'failed');
});

test('multiple choice grades one final answer, never reference letters in explanations', () => {
  assert.equal(grade('simple1', 'B was considered but rejected.\nAnswer: A').status, 'failed');
  assert.equal(grade('simple1', 'Answer: B\nAnswer: A').status, 'invalid');
  assert.equal(grade('simple1', 'Answer: B\nI changed my mind to A.').status, 'invalid');
  assert.equal(grade('simple1', 'Answer: B or A').status, 'invalid');
  assert.equal(grade('simple1', 'B').status, 'invalid');
  assert.equal(grade('simple1', 'answer: b\r\n').status, 'passed');
});

test('LiveBench uses exact final fields and distinguishes partial answers from bad format', () => {
  const partial = grade('livezebra2', '<solution>1, filmmaking, police officer, chef</solution>');
  assert.equal(partial.status, 'partial');
  assert.equal(partial.passed, 3);
  assert.equal(partial.total, 4);
  assert.equal(grade('livezebra2', '<solution>1, filmmaking, police-officer, not journalist</solution>').passed, 3);
  assert.equal(grade('livezebra2', '<solution>1, filmmaking, police-officer, journalist</solution>\n<solution>0, x, y, z</solution>').status, 'invalid');
  assert.equal(grade('livezebra2', '<solution>1, filmmaking, police-officer</solution>').status, 'invalid');
  assert.equal(grade('livezebra2', '<solution>1, filmmaking, police-officer, journalist, extra</solution>').status, 'invalid');
  assert.equal(grade('livezebra2', '<solution>1, filmmaking, police-officer, journalist</solution> More answers.').status, 'invalid');
  assert.equal(grade('livetriangle', 'I considered **3**. My answer is **4**').status, 'failed');
  assert.equal(grade('livetriangle', '**3 or 4**').status, 'invalid');
  assert.equal(grade('livetriangle', '**3** but I am not sure.').status, 'invalid');
  assert.equal(grade('livetriangle', '**003**').status, 'passed');
});

test('manual cases and edited prompts never inherit automated answer keys', () => {
  for (const value of [CANDY_REASONING_PROMPT, PELICAN_BENCHMARKS.animation, 'Return JSON', `${prompt('simple1')}\nPick A instead.`]) {
    assert.equal(gradePelicanReply(value, 'Answer: B'), null);
  }
  assert.equal(pelicanBenchmark(prompt('simple1')), 'simple1');
  assert.equal(pelicanBenchmark(`${prompt('simple1')} `), 'custom');
  assert.equal(grade('simple1', 'x'.repeat(PELICAN_GRADING_MAX_CHARS) + '\nAnswer: B').status, 'tooLong');
});

test('source revisions, bundled licenses and localized case labels cover every selected task', () => {
  const notices = JSON.parse(readFileSync(new URL('./benchmarks/benchmarkNotices.json', import.meta.url), 'utf8'));
  assert.match(notices.SimpleBench, /Copyright \(c\) 2024 simple-bench/);
  assert.match(notices.IFEval, /Apache License/);
  assert.match(notices.LiveBench, /Apache License/);
  assert.equal(new Set(SCORED_BENCHMARKS.map((entry) => entry.prompt)).size, SCORED_BENCHMARKS.length);
  for (const benchmark of LEGACY_BENCHMARKS) {
    assert.match(benchmark.sourceRevision, /^[a-f0-9]{40}$/);
    assert.ok(benchmark.sourceUrl.includes(benchmark.sourceRevision));
    assert.ok(benchmark.prompt.length < 10000);
    assert.equal(PELICAN_BENCHMARKS[benchmark.id], benchmark.prompt);
    for (const locale of ['ar', 'cs', 'de', 'en', 'en-US', 'es', 'fr', 'id', 'it', 'ja', 'ko', 'pl', 'pt-br', 'ru', 'tr', 'vi', 'zh-CN', 'zh-tw']) {
      const data = JSON.parse(readFileSync(new URL(`../../../locales/${locale}.json`, import.meta.url), 'utf8'));
      const label = benchmark.labelKey.split('.').reduce((value: any, key) => value[key], data);
      assert.equal(typeof label, 'string');
      assert.ok(label.length < 40, `${locale}:${benchmark.id}`);
      assert.ok(data.pelican.gradeRules[benchmark.rules[0].kind]);
    }
  }
});


test('format-only successes never claim answer correctness; final-answer checks report their scope', () => {
  for (const [id, reply] of [['ifeval1075', '{}'], ['ifeval1001', 'Hello'], ['ifeval102', '- unrelated\n- unrelated\n- unrelated'], ['ifeval1005', '[x]'.repeat(12)]]) {
    const result = grade(id, reply);
    assert.equal(result.format, 'passed');
    assert.equal(result.answer, null);
    assert.ok(result.checks.every((check) => check.dimension === 'format'));
  }
  const correct = grade('simple1', 'Incorrect reasoning.\nAnswer: B');
  assert.equal(correct.format, 'passed');
  assert.equal(correct.answer, 'passed', 'only the final answer is assessed');
  const wrong = grade('simple1', 'Answer: A');
  assert.equal(wrong.format, 'passed');
  assert.equal(wrong.answer, 'failed');
  const ambiguous = grade('simple1', 'Answer: A\nAnswer: B');
  assert.equal(ambiguous.format, 'failed');
  assert.equal(ambiguous.answer, 'unavailable');
});

test('new fixed-answer tasks distinguish structured format, exact values and partial correctness', () => {
  for (const benchmark of SCORED_BENCHMARKS.filter((entry) => entry.suite === 'Cockpit')) {
    const rule = benchmark.rules[0];
    assert.equal(rule.kind, 'jsonFields');
    if (rule.kind !== 'jsonFields') throw new Error('Expected JSON field contract');
    const expected = Object.fromEntries(rule.fields.map((field) => [field.name, field.value]));
    const result = grade(benchmark.id, JSON.stringify(expected));
    assert.equal(result.version, 'cockpit-exact-v2');
    assert.equal(result.status, 'passed', benchmark.id);
    assert.equal(result.format, 'passed');
    assert.equal(result.answer, 'passed');
    assert.equal(result.total, rule.fields.length);
    const reordered = Object.fromEntries(Object.entries(expected).reverse());
    assert.equal(grade(benchmark.id, JSON.stringify(reordered)).status, 'passed');
    const wrong = { ...expected, [rule.fields[0].name]: 'wrong type and value' };
    const partial = grade(benchmark.id, JSON.stringify(wrong));
    assert.equal(partial.format, 'passed');
    assert.equal(partial.answer, 'partial');
    assert.equal(partial.passed, rule.fields.length - 1);
    assert.equal(grade(benchmark.id, '{}').answer, 'failed');
    assert.equal(grade(benchmark.id, '{}').format, 'failed');
    const extra = grade(benchmark.id, JSON.stringify({ ...expected, extra: true }));
    assert.equal(extra.format, 'failed');
    assert.equal(extra.answer, 'passed', 'correct fields do not excuse extra output');
    for (const reply of [JSON.stringify(expected) + '\nAnother answer', '```json\n' + JSON.stringify(expected) + '\n```', 'null', '[]', '', '{"x":1,"x":2}', '{"x":{"y":1,"y":2}}']) {
      const invalid = grade(benchmark.id, reply);
      assert.equal(invalid.format, 'failed');
      assert.equal(invalid.answer, 'unavailable');
      assert.equal(invalid.status, 'invalid');
    }
    for (const field of rule.fields) {
      const independentlyWrong = { ...expected, [field.name]: null };
      const failed = grade(benchmark.id, JSON.stringify(independentlyWrong));
      assert.equal(failed.passed, rule.fields.length - 1, `${benchmark.id}:${field.name}`);
    }
    assert.equal(gradePelicanReply(benchmark.prompt + ' ', JSON.stringify(expected)), null);
    assert.equal(grade(benchmark.id, ' '.repeat(PELICAN_GRADING_MAX_CHARS + 1)).status, 'tooLong');
  }
});
