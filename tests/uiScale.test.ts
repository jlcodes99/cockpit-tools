import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import {
  normalizeUiScale,
  stepUiScale,
  UI_SCALE_DEFAULT,
  UI_SCALE_OPTION_VALUES,
} from '../src/utils/uiScale.ts';

describe('uiScale helpers', () => {
  it('normalizes invalid and out-of-range values', () => {
    assert.equal(normalizeUiScale(undefined), UI_SCALE_DEFAULT);
    assert.equal(normalizeUiScale(Number.NaN), UI_SCALE_DEFAULT);
    assert.equal(normalizeUiScale(0.1), 0.3);
    assert.equal(normalizeUiScale(0.5), 0.5);
    assert.equal(normalizeUiScale(3), 2);
    assert.equal(normalizeUiScale(1.25), 1.25);
  });

  it('steps through configured scale options', () => {
    assert.equal(stepUiScale(1, 1), 1.1);
    assert.equal(stepUiScale(1.1, 1), 1.25);
    assert.equal(stepUiScale(1.5, 1), 1.5);
    assert.equal(stepUiScale(1, -1), 0.9);
    assert.equal(stepUiScale(0.9, -1), 0.7);
    assert.equal(stepUiScale(0.3, -1), 0.3);
  });

  it('snaps intermediate values to the next option', () => {
    assert.equal(stepUiScale(1.05, 1), 1.1);
    assert.equal(stepUiScale(1.05, -1), 1);
    assert.equal(stepUiScale(1.3, 1), 1.5);
    assert.equal(stepUiScale(1.3, -1), 1.25);
  });

  it('keeps the public option list stable', () => {
    assert.deepEqual([...UI_SCALE_OPTION_VALUES], [0.3, 0.5, 0.7, 0.9, 1, 1.1, 1.25, 1.5]);
  });
});
