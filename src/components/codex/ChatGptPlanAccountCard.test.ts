import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';
import type { ChatGptPlanAccount } from '../../services/chatgptPlanService';
import type { CodexLocalAccessUsageStats } from '../../types/codexLocalAccess';
import type { ChatGptPlanAccountCardProps } from './ChatGptPlanAccountCard';

function nodes(value: any): any[] {
  if (!value || typeof value !== 'object') return [];
  if (Array.isArray(value)) return value.flatMap(nodes);
  return [value, ...nodes(value.props?.children)];
}
function text(value: any): string {
  if (value == null || typeof value === 'boolean') return '';
  if (Array.isArray(value)) return value.map(text).join('');
  return typeof value === 'object' ? text(value.props?.children) : String(value);
}
const account: ChatGptPlanAccount = {
  id: '12345678-1234-4234-8234-123456789abc', label: 'Person', email: 'person@example.com',
  subject: 'private-subject', connected: true, needs_reauth: false, created_at: 1, updated_at: 1, in_service: false,
};
function harness(layout: ChatGptPlanAccountCardProps['layout'] = 'grid', serviceCallback = false) {
  const navigations: string[] = [];
  let manage = 0;
  let refresh = 0;
  let service = 0;
  let failed = false;
  let usageDelay: ReturnType<typeof deferred<void>> | null = null;
  let usageCalls = 0;
  let current = account;
  let stats: CodexLocalAccessUsageStats | null | undefined;
  let planType: string | undefined;
  const hook = loadHookModule(new URL('./ChatGptPlanAccountCard.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string, values?: { value?: string; id?: string }) => values ? `${key}:${values.value ?? values.id}` : key, i18n: { language: 'en' } }) },
    '@tauri-apps/plugin-opener': { async openUrl(url: string) {
      usageCalls++; assert.equal(url, 'https://chatgpt.com/settings/usage');
      if (usageDelay) await usageDelay.promise;
      if (failed) throw new Error('synthetic private URL and credentials');
    } },
  }, { window: { dispatchEvent(event: { detail: string }) { navigations.push(event.detail); } }, CustomEvent: class { detail: string; constructor(_name: string, init: { detail: string }) { this.detail = init.detail; } } });
  hook.render(() => hook.exports.ChatGptPlanAccountCard({ account: current, layout, stats, planType,
    onManage() { manage++; }, onRefresh() { refresh++; }, onOpenService: serviceCallback ? () => { service++; } : undefined,
    maskAccountText() { return 'masked'; } }));
  const tree = () => nodes(hook.flush());
  const button = (key: string) => tree().find((node) => node.type === 'button' && node.props['aria-label'] === key);
  return { hook, tree, button, navigations, visible: () => text(hook.flush()), managed: () => manage, refreshed: () => refresh, services: () => service, usageCalls: () => usageCalls,
    setAccount(next: ChatGptPlanAccount) { current = next; }, setStats(next: CodexLocalAccessUsageStats | null) { stats = next; }, setPlan(next?: string) { planType = next; },
    fail(next: boolean) { failed = next; }, delay() { usageDelay = deferred<void>(); return usageDelay; } };
}

for (const layout of ['grid', 'compact', 'table'] as const) {
  test(`${layout} shares ordinary account layout, keeps identity concise and disables ordinary batch selection`, () => {
    const h = harness(layout);
    const tree = h.tree();
    const root = tree[0];
    assert.equal(root.type, layout === 'table' ? 'tr' : 'div');
    if (layout === 'grid') {
      assert.match(root.props.className, /\bcodex-account-card\b/);
      for (const name of ['card-top', 'card-select', 'account-email', 'account-sub-line', 'codex-quota-section', 'codex-card-bottom', 'card-date', 'card-footer', 'card-actions']) {
        assert.ok(tree.some((node) => String(node.props?.className).split(' ').includes(name)), name);
      }
    } else if (layout === 'compact') {
      for (const name of ['codex-compact-row', 'codex-compact-header', 'codex-compact-select', 'codex-compact-email', 'codex-compact-quotas', 'codex-compact-actions']) {
        assert.ok(tree.some((node) => String(node.props?.className).split(' ').includes(name)), name);
      }
    } else {
      assert.equal(root.props.children.filter((node: any) => node?.type === 'td').length, 6);
      assert.ok(tree.some((node) => node.props.className === 'sticky-action-cell table-action-cell'));
      assert.ok(tree.some((node) => node.props.className === 'account-cell'));
    }
    const selection = tree.find((node) => node.type === 'input');
    assert.equal(selection.props.type, 'checkbox');
    assert.equal(selection.props.disabled, true);
    assert.equal(selection.props.checked, false);
    assert.equal(selection.props.onChange, undefined);
    assert.equal(selection.props.title, 'chatgptPlan.card.manageSeparately');
    assert.equal((h.visible().match(/masked/g) ?? []).length, 1, 'email appears only once');
    assert.equal((h.visible().match(/chatgptPlan.status.ready/g) ?? []).length, 1, 'authorization and pool status share one label');
    assert.ok(h.visible().includes(account.id.slice(0, 8)), 'authorization short ID distinguishes registrations sharing email');
    const raw = JSON.stringify(tree);
    for (const secret of [account.email!, account.subject, account.id]) assert.equal(raw.includes(secret), false, secret);
    for (const key of ['chatgptPlan.title', 'chatgptPlan.card.apiOnly', 'chatgptPlan.imagePolicyLocked', 'chatgptPlan.imagePolicyHint', 'chatgptPlan.card.subscriptionUnknown']) assert.ok(raw.includes(key), key);
    assert.equal(raw.includes('quota-bar'), false);
    assert.equal(raw.includes('100%'), false);
    assert.equal(raw.includes('chatgptPlan.card.requests:—'), true);
    assert.equal(raw.includes('chatgptPlan.card.tokens:—'), true);
    assert.ok(h.visible().includes('chatgptPlan.card.localUsage'), 'local usage is visibly scoped in every layout');
    const actionClass = layout === 'grid' ? 'card-action-btn' : layout === 'compact' ? 'codex-compact-note-btn' : 'action-btn';
    assert.ok(tree.filter((node) => node.type === 'button').every((node) => node.props.className === actionClass));
    h.hook.unmount();
  });

  test(`${layout} actions open authorization management, refresh summaries and manage joined service members without model verification`, () => {
    const h = harness(layout, true);
    const allowed = ['chatgptPlan.card.manageAuthorization', 'chatgptPlan.officialFlow.useOfficial', 'common.refresh', 'chatgptPlan.manageUsage'];
    assert.deepEqual(h.tree().filter((node) => node.type === 'button').map((node) => node.props['aria-label']), allowed);
    h.button('chatgptPlan.officialFlow.useOfficial').props.onClick();
    h.button('chatgptPlan.card.manageAuthorization').props.onClick();
    h.button('common.refresh').props.onClick();
    assert.equal(h.managed(), 2);
    assert.equal(h.refreshed(), 1);
    assert.equal(h.usageCalls(), 0, 'management never probes or verifies a model');
    assert.deepEqual(h.navigations, []);
    h.setAccount({ ...account, in_service: true });
    h.button('chatgptPlan.card.manageService').props.onClick();
    assert.equal(h.services(), 1);
    assert.equal(h.button('chatgptPlan.officialFlow.useOfficial'), undefined);
    assert.deepEqual(h.navigations, [], 'an injected navigation callback is respected');
    h.setAccount({ ...account, in_service: true, needs_reauth: true });
    h.button('common.reauthorize').props.onClick();
    assert.equal(h.managed(), 3);
    assert.equal(h.button('chatgptPlan.card.manageService'), undefined);
    assert.equal(h.button('chatgptPlan.officialFlow.useOfficial'), undefined);
    assert.equal(h.tree().some((node) => node.type === 'button' && /switch|launch|instance|delete|export/i.test(node.props['aria-label'])), false);
    h.hook.unmount();
  });

  test(`${layout} displays original subscription and trusted local usage without fabricating a balance`, () => {
    const h = harness(layout);
    h.setPlan('self_serve_business_usage_based');
    h.setStats({ requestCount: 4, totalTokens: 1250 } as CodexLocalAccessUsageStats);
    assert.ok(h.visible().includes('self_serve_business_usage_based'));
    assert.ok(h.visible().includes('chatgptPlan.card.requests:4'));
    assert.ok(h.visible().includes('chatgptPlan.card.tokens:1.3K'));
    assert.equal(h.visible().includes('subscriptionUnknown'), false);
    assert.ok(JSON.stringify(h.tree()).includes('chatgptPlan.localStatsHint'));
    h.setPlan('');
    h.setStats({ requestCount: 0, totalTokens: 0 } as CodexLocalAccessUsageStats);
    assert.ok(h.visible().includes('subscriptionUnknown'));
    assert.ok(h.visible().includes('chatgptPlan.card.requests:0'));
    assert.ok(h.visible().includes('chatgptPlan.card.tokens:0'));
    h.setStats({ requestCount: Number.NaN, totalTokens: -1 } as CodexLocalAccessUsageStats);
    assert.ok(h.visible().includes('chatgptPlan.card.requests:—'));
    assert.ok(h.visible().includes('chatgptPlan.card.tokens:—'));
    h.setAccount({ ...account, id: '87654321-1234-4234-8234-123456789abc' });
    assert.ok(h.visible().includes('87654321'));
    assert.equal(h.visible().includes('12345678'), false);
    h.hook.unmount();
  });

  test(`${layout} usage errors stay at their action context and repeated clicks share one browser opening`, async () => {
    const h = harness(layout);
    const opening = h.delay(); h.fail(true);
    const click = h.button('chatgptPlan.manageUsage').props.onClick;
    click(); click();
    assert.equal(h.usageCalls(), 1);
    assert.equal(h.button('chatgptPlan.manageUsage').props.disabled, true);
    opening.resolve(); await settlePromises();
    assert.equal(h.tree().find((node) => node.props?.role === 'alert')?.props.children, 'chatgptPlan.error.generic');
    assert.equal(JSON.stringify(h.tree()).includes('synthetic'), false);
    h.fail(false);
    h.button('chatgptPlan.manageUsage').props.onClick();
    assert.equal(h.tree().some((node) => node.props?.role === 'alert'), false);
    await settlePromises();
    assert.equal(h.tree().some((node) => node.props?.role === 'alert'), false);
    assert.equal(h.button('chatgptPlan.manageUsage').props.disabled, false);
    h.hook.unmount();
  });
}

test('service navigation retains the existing common page fallback when no callback is supplied', () => {
  const h = harness();
  h.setAccount({ ...account, in_service: true });
  h.button('chatgptPlan.card.manageService').props.onClick();
  assert.deepEqual(h.navigations, ['codex-api-service']);
  h.hook.unmount();
});

test('the plan card has no independent surface skin or large textual actions', () => {
  const css = readFileSync(new URL('./ChatGptPlanAccountCard.css', import.meta.url), 'utf8');
  const source = readFileSync(new URL('./ChatGptPlanAccountCard.tsx', import.meta.url), 'utf8');
  assert.equal(css.includes('chatgpt-plan-section--cards'), false);
  assert.equal(css.includes('chatgpt-plan-card-grid'), false);
  assert.equal(/\.chatgpt-plan-account-card[^}]+(?:padding|box-shadow|min-height|border-radius):/.test(css), false);
  assert.equal(/className="btn\s/.test(source), false);
  assert.equal(/Terminal|Trash2|Download|onSwitch|onLaunch/.test(source), false);
});
