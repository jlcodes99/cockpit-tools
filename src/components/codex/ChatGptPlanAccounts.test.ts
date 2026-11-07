import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';
import { withChatGptPlanMembership, chatGptPlanStatusKey } from '../../services/chatgptPlanService';

type Node = { type: unknown; props: Record<string, any> };
function nodes(value: any): Node[] {
  if (!value || typeof value !== 'object') return [];
  if (Array.isArray(value)) return value.flatMap(nodes);
  return [{ type: value.type, props: value.props ?? {} }, ...nodes(value.props?.children)];
}
const account = { id: 'saved', email: 'user@example.com', label: 'Example', connected: true, verified_at: 1, needs_reauth: false, in_service: false };

function harness(read: () => Promise<unknown>, initialContext: { email?: string } | null, initialMembers?: string[], extra: Record<string, unknown> = {}) {
  const window = new EventTarget();
  let pending = initialContext;
  let members = initialMembers;
  let tauriUpdated: (() => void) | undefined;
  const hook = loadHookModule(new URL('./ChatGptPlanAccounts.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../ModalErrorMessage': { ModalErrorMessage: 'ErrorMessage' },
    './ChatGptPlanDialog': { ChatGptPlanDialog: 'PlanDialog' },
    './ChatGptPlanAccountCard': { ChatGptPlanAccountCard: 'PlanCard' },
    '../../services/chatgptPlanService': {
      listChatGptPlanAccounts: read,
      chatGptPlanStatusKey,
      withChatGptPlanMembership,
      chatGptPlanErrorKey: () => 'chatgptPlan.error.generic',
    },
    '../../utils/chatgptPlanNavigation': {
      CHATGPT_PLAN_OPEN_REQUEST: 'chatgpt-plan-open-request',
      takePendingChatgptPlanAuthorization() { const context = pending; pending = null; return context; },
    },
    '../../utils/tauriEventListener': { subscribeTauriEvent(_name: string, callback: () => void) { tauriUpdated = callback; return () => { tauriUpdated = undefined; }; } },
  }, { window });
  hook.render(() => hook.exports.ChatGptPlanAccounts({ onChanged() {}, memberAccountIds: members, ...extra }));
  const tree = () => nodes(hook.flush());
  const dialog = () => tree().find((node) => node.type === 'PlanDialog');
  const request = (context: { email?: string }) => { pending = context; window.dispatchEvent(new Event('chatgpt-plan-open-request')); };
  return { hook, tree, dialog, request, setMembers(next: string[]) { members = next; return tree(); }, updated() { tauriUpdated?.(); } };
}

test('cold shortcut opens immediately and then reuses the sole matching registration', async () => {
  const read = deferred<unknown>();
  const ctx = harness(() => read.promise, { email: 'USER@example.com' });
  assert.equal(ctx.dialog()!.props.expectedEmail, 'USER@example.com');
  assert.equal(ctx.dialog()!.props.account, undefined);
  read.resolve([account]);
  await settlePromises();
  assert.equal(ctx.dialog()!.props.account.id, 'saved');
  ctx.hook.unmount();
});

test('late discovery never replaces an authorization the user already started', async () => {
  const read = deferred<unknown>();
  const ctx = harness(() => read.promise, { email: 'user@example.com' });
  ctx.dialog()!.props.onLoginStart();
  read.resolve([account]);
  await settlePromises();
  assert.equal(ctx.dialog()!.props.account, undefined);
  ctx.dialog()!.props.onClose();
  assert.equal(ctx.dialog(), undefined);
  ctx.hook.unmount();
});

test('a loaded unique account is reused without another storage read, while ambiguous accounts are never guessed', async () => {
  let reads = 0;
  let data = [account];
  const ctx = harness(async () => { reads++; return data; }, null);
  await settlePromises(); ctx.tree();
  ctx.request({ email: 'USER@example.com' });
  assert.equal(ctx.dialog()!.props.account.id, 'saved');
  assert.equal(reads, 1);
  ctx.dialog()!.props.onClose();
  data = [account, { ...account, id: 'another' }];
  ctx.tree().find((node) => node.type === 'button' && Array.isArray(node.props.children) && node.props.children.includes('common.refresh'))!.props.onClick();
  await settlePromises(); ctx.tree();
  ctx.request({ email: 'user@example.com' });
  assert.equal(ctx.dialog()!.props.account, undefined);
  assert.equal(ctx.dialog()!.props.expectedEmail, 'user@example.com');
  ctx.hook.unmount();
});

test('closing the immediate dialog prevents a late local lookup from reopening it', async () => {
  const read = deferred<unknown>();
  const ctx = harness(() => read.promise, { email: 'user@example.com' });
  ctx.dialog()!.props.onClose();
  read.resolve([account]);
  await settlePromises();
  assert.equal(ctx.dialog(), undefined);
  ctx.hook.unmount();
});

test('the plan panel follows the same live member IDs as the member list, not its stale summary', async () => {
  let reads = 0;
  const ctx = harness(async () => { reads++; return [account]; }, null, ['chatgpt_plan_saved']);
  await settlePromises();
  assert.equal(ctx.tree().some((node) => node.props.children === 'chatgptPlan.status.joined'), true);
  ctx.setMembers([]);
  assert.equal(ctx.tree().some((node) => node.props.children === 'chatgptPlan.status.ready'), true);
  ctx.updated();
  await settlePromises();
  assert.equal(reads, 2);
  assert.equal(ctx.tree().some((node) => node.props.children === 'chatgptPlan.status.ready'), true);
  ctx.hook.unmount();
});

test('a new authorization without an email skips account discovery and opens immediately', async () => {
  let reads = 0;
  const ctx = harness(async () => { reads++; return []; }, null);
  await settlePromises();
  ctx.request({});
  await settlePromises();
  assert.equal(reads, 1, 'only the independent initial panel read is needed');
  assert.equal(Boolean(ctx.dialog()), true);
  assert.equal(ctx.dialog()!.props.discoveryError, undefined);
  ctx.hook.unmount();
});

test('shortcut account discovery failures stay in the current dialog and clear on a new login attempt', async () => {
  let reads = 0;
  const ctx = harness(async () => { if (++reads === 1) return []; throw new Error('storage unavailable'); }, null);
  await settlePromises();
  ctx.request({ email: 'user@example.com' });
  await settlePromises();
  assert.equal(ctx.dialog()!.props.discoveryError, 'chatgptPlan.error.generic');
  assert.equal(ctx.tree().find((node) => node.type === 'ErrorMessage')!.props.message, '');
  ctx.dialog()!.props.onLoginStart();
  assert.equal(ctx.dialog()!.props.discoveryError, undefined);
  assert.equal(ctx.dialog()!.props.expectedEmail, 'user@example.com');
  ctx.hook.unmount();
});


test('overview cards include saved unjoined accounts and preserve privacy/statistics props', async () => {
  const counts: number[] = [];
  const mask = (value: string) => `masked:${value}`;
  const usage = { requestCount: 12, totalTokens: 34 };
  const ctx = harness(async () => [account], null, [], {
    variant: 'cards', maskAccountText: mask,
    accountStats: [{ accountId: 'chatgpt_plan_saved', usage }], onCountChange: (count: number) => counts.push(count),
  });
  await settlePromises();
  const card = ctx.tree().find((node) => node.type === 'PlanCard')!;
  assert.equal(card.props.account.in_service, false);
  assert.equal(card.props.maskAccountText, mask);
  assert.equal(card.props.stats, usage);
  assert.deepEqual(counts, [0, 1]);
  card.props.onManage();
  assert.equal(ctx.dialog()!.props.account.id, 'saved');
  ctx.hook.unmount();
});

test('overview cards never consume API Service navigation requests and filter without changing total count', async () => {
  const counts: number[] = [];
  const ctx = harness(async () => [account], { email: account.email }, [], {
    variant: 'cards', searchQuery: 'missing', onCountChange: (count: number) => counts.push(count),
  });
  await settlePromises();
  ctx.request({ email: account.email });
  assert.equal(ctx.dialog(), undefined);
  assert.equal(ctx.tree().some((node) => node.type === 'PlanCard'), false);
  assert.equal(ctx.tree().some((node) => node.props.children === 'chatgptPlan.card.searchEmpty'), true);
  assert.deepEqual(counts, [0, 1]);
  ctx.hook.unmount();
});
