import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { CANDY_REASONING_PROMPT, PELICAN_BENCHMARKS, pelicanBenchmark, PELICAN_BENCHMARK_IDS, scoredPelicanBenchmark, pelicanBenchmarkLabel } from './pelicanBenchmarks.ts';

test('benchmark selection preserves the animation and recognizes only unedited built-in cases', () => {
  assert.equal(pelicanBenchmark(PELICAN_BENCHMARKS.animation), 'animation');
  assert.equal(pelicanBenchmark(CANDY_REASONING_PROMPT), 'candy');
  assert.equal(pelicanBenchmark(`${CANDY_REASONING_PROMPT}\nUse different quantities.`), 'custom');
  assert.equal(pelicanBenchmark(''), 'custom');
  assert.match(CANDY_REASONING_PROMPT, /圆形 \| 7 \| 9 \| 8/);
  assert.match(CANDY_REASONING_PROMPT, /五角星 \| 7 \| 6 \| 4/);
  assert.match(CANDY_REASONING_PROMPT, /允许按形状选择拿取/);
  assert.doesNotMatch(CANDY_REASONING_PROMPT, /21|12 颗|参考/);
});

test('exhaustive candy outcomes prove 21 is minimal for every shape-selection strategy', () => {
  // Flavor is unobservable, so adapting to flavor is impossible: a strategy has
  // only the counts of circles/stars as feedback. Enumerate all flavor outcomes.
  const failing = new Set<string>();
  for (let roundApple = 0; roundApple <= 7; roundApple++)
    for (let roundPeach = 0; roundPeach <= 9; roundPeach++)
      for (let roundMelon = 0; roundMelon <= 8; roundMelon++)
        for (let starApple = 0; starApple <= 7; starApple++)
          for (let starPeach = 0; starPeach <= 6; starPeach++)
            for (let starMelon = 0; starMelon <= 4; starMelon++) {
              if ((roundApple && starPeach) || (starApple && roundPeach)) continue;
              failing.add(`${roundApple + roundPeach + roundMelon}:${starApple + starPeach + starMelon}`);
            }
  let minimum = Infinity;
  for (let circles = 0; circles <= 24; circles++) {
    for (let stars = 0; stars <= 17; stars++) {
      if (!failing.has(`${circles}:${stars}`)) minimum = Math.min(minimum, circles + stars);
      if (circles + stars <= 20) assert.ok(failing.has(`${circles}:${stars}`));
    }
  }
  assert.equal(minimum, 21);
  assert.ok(!failing.has('9:12'), '12 stars followed by 9 circles always succeeds');
  assert.ok(failing.has('9:11'), '11 stars followed by 9 circles can fail');
});

test('localized references stay separate from request payloads and all locales have short case labels', () => {
  for (const locale of ['ar', 'cs', 'de', 'en', 'en-US', 'es', 'fr', 'id', 'it', 'ja', 'ko', 'pl', 'pt-br', 'ru', 'tr', 'vi', 'zh-CN', 'zh-tw']) {
    const { pelican } = JSON.parse(readFileSync(new URL(`../../../locales/${locale}.json`, import.meta.url), 'utf8'));
    assert.match(pelican.candyReference, /21/);
    assert.match(pelican.candyReference, /12/);
    assert.match(pelican.candyReference, /9/);
    assert.ok(pelican.benchmarks.candy.length < 30, locale);
    assert.ok(pelican.benchmarks.animation.length < 30, locale);
  }
});


test('the curated list removes duplicate templates while frozen historical cases remain recognizable', () => {
  assert.equal(new Set(PELICAN_BENCHMARK_IDS).size, PELICAN_BENCHMARK_IDS.length);
  assert.equal(PELICAN_BENCHMARK_IDS.length, 11, 'ten cases plus custom');
  for (const id of ['ifeval102', 'ifeval1005', 'ifeval1001', 'simple2', 'simple8', 'livezebra2', 'livetriangle', 'livesquare']) {
    assert.ok(!PELICAN_BENCHMARK_IDS.includes(id), id);
    const snapshot = Object.freeze({ prompt: PELICAN_BENCHMARKS[id], rawReply: 'Original saved reply', model: 'saved-model' });
    const before = JSON.stringify(snapshot);
    assert.equal(pelicanBenchmark(snapshot.prompt), id);
    assert.equal(scoredPelicanBenchmark(snapshot.prompt)?.graderVersion, 'cockpit-strict-v1');
    assert.equal(JSON.stringify(snapshot), before);
  }
  for (const id of PELICAN_BENCHMARK_IDS.filter((id) => !['custom', 'animation', 'candy'].includes(id))) {
    const benchmark = scoredPelicanBenchmark(PELICAN_BENCHMARKS[id]);
    assert.ok(benchmark);
    assert.notEqual(benchmark.labelKey, 'pelican.caseNumber');
    assert.ok(pelicanBenchmarkLabel(id, (key) => key).includes(benchmark.suite));
  }
});

test('every active case has a distinct descriptive label in each supported locale', () => {
  for (const locale of ['ar', 'cs', 'de', 'en', 'en-US', 'es', 'fr', 'id', 'it', 'ja', 'ko', 'pl', 'pt-br', 'ru', 'tr', 'vi', 'zh-CN', 'zh-tw']) {
    const data = JSON.parse(readFileSync(new URL(`../../../locales/${locale}.json`, import.meta.url), 'utf8'));
    const labels = PELICAN_BENCHMARK_IDS.map((id) => pelicanBenchmarkLabel(id, (key) => {
      const label = key.split('.').reduce((value: any, part) => value?.[part], data);
      assert.equal(typeof label, 'string', `${locale}:${key}`);
      assert.ok(label.length < 50, `${locale}:${key}: short dropdown label`);
      return label;
    }));
    assert.equal(new Set(labels).size, labels.length, locale);
    assert.equal(typeof data.pelican.gradeRules.jsonFields, 'string');
  }
});
