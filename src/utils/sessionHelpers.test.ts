import assert from 'node:assert/strict';
import test from 'node:test';
import { formatRelativeTime } from './sessionHelpers';

test('formats session age with the selected language and preserves the unknown-time label', (context) => {
  const now = 1_800_000_000_000;
  context.mock.method(Date, 'now', () => now);
  const russian = new Intl.RelativeTimeFormat('ru', { style: 'short' });
  const english = new Intl.RelativeTimeFormat('en', { style: 'short' });
  const twoHoursAgo = now - 2 * 60 * 60 * 1000;
  assert.equal(formatRelativeTime(twoHoursAgo, russian, 'Время неизвестно'), russian.format(-2, 'hour'));
  assert.equal(formatRelativeTime(twoHoursAgo, english, 'Unknown time'), english.format(-2, 'hour'));
  assert.notEqual(formatRelativeTime(twoHoursAgo, russian, ''), formatRelativeTime(twoHoursAgo, english, ''));
  assert.equal(formatRelativeTime(null, russian, 'Время неизвестно'), 'Время неизвестно');
});
