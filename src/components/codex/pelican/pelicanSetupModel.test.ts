import assert from 'node:assert/strict';
import test from 'node:test';
import type { CodexAccount } from '../../../types/codex.ts';
import { defaultPelicanConcurrency, parsePelicanConcurrency, pelicanAccountSummary } from './pelicanSetupModel.ts';

test('custom concurrency accepts whole numbers across the backend range', () => {
  for (const value of ['1', '3', '6', '10', ' 8 ']) assert.equal(parsePelicanConcurrency(value), Number(value));
  for (const value of ['', ' ', '0', '-1', '11', '20', '1.5', '1e2', 'NaN', 'Infinity', 'abc', '9007199254740993']) assert.equal(parsePelicanConcurrency(value), null, value);
});

test('default concurrency follows the selected account count and caps at ten', () => {
  assert.equal(defaultPelicanConcurrency(0), 1);
  assert.equal(defaultPelicanConcurrency(1), 1);
  assert.equal(defaultPelicanConcurrency(7), 7);
  assert.equal(defaultPelicanConcurrency(10), 10);
  assert.equal(defaultPelicanConcurrency(35), 10);
});

test('subscription badges preserve raw casing and unknown values without inferring a plan', () => {
  const account = { id: 'test', auth_mode: 'oauth', plan_type: 'Business Custom' } as CodexAccount;
  assert.equal(pelicanAccountSummary(account).plan, 'Business Custom');
  assert.equal(pelicanAccountSummary({ ...account, plan_type: undefined, auth_file_plan_type: 'prolite' }).plan, 'prolite');
  assert.equal(pelicanAccountSummary({ ...account, plan_type: undefined }).plan, null);
});

test('quota rows use real windows and preserve zero and missing-data distinctions', () => {
  const account = { id: 'test', auth_mode: 'oauth', plan_type: 'plus' } as CodexAccount;
  assert.deepEqual(pelicanAccountSummary(account).windows, []);
  const weeklyOnly = pelicanAccountSummary({ ...account, quota: { hourly_percentage: 73, weekly_percentage: 0,
    hourly_window_minutes: 10080, hourly_window_present: true, weekly_window_present: false } });
  assert.equal(weeklyOnly.windows.length, 1);
  assert.equal(weeklyOnly.windows[0].label, 'Weekly');
  assert.equal(weeklyOnly.windows[0].percentage, 73);
  const exhausted = pelicanAccountSummary({ ...account, quota: { hourly_percentage: 90, weekly_percentage: 0 } });
  assert.deepEqual(exhausted.windows.map((window) => window.percentage), [0, 0]);
  const unknown = pelicanAccountSummary({ ...account, quota: { hourly_percentage: NaN, weekly_percentage: 44 } });
  assert.equal(unknown.windows[0].percentage, null);
  assert.equal(unknown.windows[1].percentage, 44);
});
