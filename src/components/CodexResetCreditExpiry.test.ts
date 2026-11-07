import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';
import { formatCodexResetTime } from '../types/codex';
import { getResetCreditExpiry } from '../utils/codexResetCreditExpiry';

function setup() {
  let current: any;
  let calls = 0, clicks = 0;
  const listeners = new Set<() => void>();
  const details = {
    subscribe(_id: string, listener: () => void) { listeners.add(listener); return () => listeners.delete(listener); },
    peek(id: string) { return current?.id === id ? current.snapshot : undefined; },
    seed(id: string, _count: number, snapshot: any) { current = { id, snapshot }; },
    async load(id: string, _count: number, force: boolean) {
      calls++;
      await Promise.resolve();
      if (id === 'b') throw Error('offline');
      if (!current || force) {
        current = { id, snapshot: { available_count: 1, credits: [], next_expires_at: Date.now() / 1000 + 86400 } };
        listeners.forEach(listener => listener());
      }
      return current.snapshot;
    },
  };
  const h = loadHookModule(new URL('./CodexResetCreditExpiry.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string, options: any) =>
      key === 'codex.quota.resetCreditNextExpiry' ? `最近重置到期 ${options.time}` : key }) },
    '../types/codex': { formatCodexResetTime },
    '../utils/codexResetCreditExpiry': { getResetCreditExpiry },
    '../services/codexResetCreditService': { codexResetCreditDetails: details },
  }, { window: { setInterval: () => 1, clearInterval() {}, addEventListener() {}, removeEventListener() {} } });
  return { h, details, calls: () => calls, click: () => clicks++, clicks: () => clicks };
}

test('expiry uses quota time format and manual refresh updates the shared result', async () => {
  const s = setup();
  let props: any = { accountId: 'a', quota: { reset_credits_available: 1 }, disabled: false, onClick: s.click };
  const render = () => s.h.exports.CodexResetCreditExpiry(props);
  assert.match(s.h.render(render).props.children, /最近重置到期 .*TimeUnknown/);
  await settlePromises();
  let output = s.h.flush();
  assert.match(output.props.children, /最近重置到期 (1d|23h).*\(\d{2}\/\d{2} \d{2}:\d{2}\)/);
  assert.match(output.props.className, /codex-quota-mini-reset/);
  output.props.onClick(); await settlePromises();
  assert.equal(s.calls(), 2); assert.equal(s.clicks(), 1);
  props = { ...props, accountId: 'b' };
  assert.match(s.h.render(render).props.children, /TimeUnknown/);
  await settlePromises();
  props = { ...props, quota: { reset_credits_available: 0 } };
  assert.equal(s.h.render(render), null);
  s.h.unmount();
});

test('near-expiry text retains a warning color without a separate badge', async () => {
  const s = setup();
  s.details.seed('a', 1, { available_count: 1, credits: [], next_expires_at: Date.now() / 1000 + 3600 });
  const output = s.h.render(() => s.h.exports.CodexResetCreditExpiry({ accountId: 'a', quota: { reset_credits_available: 1 }, onClick() {} }));
  assert.match(output.props.className, /is-urgent/);
  assert.equal(typeof output.props.children, 'string');
  await settlePromises(); s.h.unmount();
});

test('expired credits are never described as a completed quota reset', async () => {
  const s = setup();
  s.details.seed('a', 1, { available_count: 1, credits: [], next_expires_at: Date.now() / 1000 - 60 });
  const output = s.h.render(() => s.h.exports.CodexResetCreditExpiry({ accountId: 'a', quota: { reset_credits_available: 1 }, onClick() {} }));
  assert.match(output.props.children, /resetCreditStatusExpired/);
  assert.doesNotMatch(output.props.children, /resetDone/);
  assert.match(output.props.className, /is-urgent/);
  await settlePromises(); s.h.unmount();
});
