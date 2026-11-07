import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../../../tests/helpers/reactHookHarness';
import { PELICAN_BENCHMARKS } from './pelicanBenchmarks.ts';

function nodes(tree: any): any[] {
  if (Array.isArray(tree)) return tree.flatMap(nodes);
  return tree && typeof tree === 'object' ? [tree, ...nodes(tree.props?.children)] : [];
}

test('scores require completed artifacts; animation remains manual and transport errors have no zero grade', () => {
  const h = loadHookModule(new URL('./PelicanScore.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
  });
  let props = { prompt: PELICAN_BENCHMARKS.simple1, reply: undefined as string | undefined, status: 'completed' };
  h.render(() => h.exports.PelicanScore(props));
  assert.equal(h.flush().props.children, 'pelican.grading.waiting');
  props = { ...props, reply: 'Answer: B' };
  assert.equal(h.flush().props.className, 'pelican-score pelican-score-passed');
  assert.ok(nodes(h.flush()).some((node) => node.props.children === 'pelican.grading.formatPassed'));
  props = { ...props, reply: 'Answer: A' };
  assert.equal(h.flush().props.className, 'pelican-score pelican-score-failed');
  props = { ...props, reply: 'Answer: A or B' };
  assert.equal(h.flush().props.className, 'pelican-score pelican-score-invalid');
  for (const status of ['queued', 'running', 'failed', 'cancelled', 'interrupted']) {
    props = { ...props, reply: 'Answer: B', status };
    assert.equal(h.flush(), null);
  }
  props = { ...props, prompt: PELICAN_BENCHMARKS.animation, status: 'completed' };
  assert.equal(h.flush().props.children, 'pelican.grading.manual');
  props = { ...props, prompt: 'Custom prompt' };
  assert.equal(h.flush().props.children, 'pelican.grading.manual');
  h.unmount();
});


test('format-only replies and wrong final answers have distinct visible labels', () => {
  const h = loadHookModule(new URL('./PelicanScore.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
  });
  let props = { prompt: PELICAN_BENCHMARKS.ifeval1075, reply: '{}', status: 'completed' };
  h.render(() => h.exports.PelicanScore(props));
  const content = () => JSON.stringify(nodes(h.flush()).map((node) => node.props.children));
  assert.match(content(), /pelican.grading.formatPassed/);
  assert.match(content(), /pelican.grading.formatOnly/);
  assert.doesNotMatch(content(), /pelican.grading.answerPassed/);
  assert.doesNotMatch(content(), /pelican.grading.passed/);
  props = { ...props, prompt: PELICAN_BENCHMARKS.simple1, reply: 'Answer: A' };
  assert.match(content(), /pelican.grading.formatPassed/);
  assert.match(content(), /pelican.grading.answerFailed/);
  assert.match(content(), /pelican.grading.answerScope/);
  props = { ...props, reply: 'Answer: B' };
  assert.match(content(), /pelican.grading.answerPassed/);
  props = { ...props, reply: 'Answer: A or B' };
  assert.match(content(), /pelican.grading.answerUnavailable/);
  const summary = nodes(h.flush()).find((node) => node.type === 'summary');
  assert.doesNotMatch(JSON.stringify(summary), /0\/1/, 'ungraded answers must not display a zero grade');
  h.unmount();
});
