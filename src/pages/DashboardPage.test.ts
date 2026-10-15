import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import postcss from 'postcss';

const stylesheet = postcss.parse(
  readFileSync(new URL('./DashboardPage.css', import.meta.url), 'utf8'),
);

function declarations(selector: string): Record<string, string> {
  const values: Record<string, string> = {};
  stylesheet.walkRules(selector, (rule) => {
    rule.walkDecls((declaration) => {
      values[declaration.prop] = declaration.value;
    });
  });
  return values;
}

test('stacked quota tracks keep a non-shrinking visible height in a column', () => {
  assert.equal(declarations('.mini-quota-row-stacked')['flex-direction'], 'column');
  const track = {
    ...declarations('.mini-progress-track'),
    ...declarations('.mini-quota-row-stacked > .mini-progress-track'),
  };
  // A flex: 1 child in an auto-height column can collapse despite height: 3px.
  assert.equal(track.flex, 'none');
  assert.equal(track.height, '3px');
  assert.equal(track['min-height'], '3px');
  assert.equal(track.width, '100%');
});

test('horizontal quota tracks retain their flexible width', () => {
  const track = declarations('.mini-progress-track');
  assert.equal(track.flex, '1');
  assert.equal(track.height, '3px');
  assert.equal(track.width, undefined);
});

test('track visibility does not impose a minimum fill on zero or missing quota', () => {
  const fill = declarations('.mini-progress-bar');
  assert.equal(fill.height, '100%');
  assert.equal(fill.width, undefined);
  assert.equal(fill['min-width'], undefined);
  assert.equal(declarations('.mini-progress-track').overflow, 'hidden');
});
