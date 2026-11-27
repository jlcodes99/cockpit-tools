import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync, readdirSync } from 'node:fs';
import { deferred, loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';
import type { CodexWorkbuddyDelegateCatalog, CodexWorkbuddyDelegateProfile } from '../../types/codex';
import type { WorkbuddyAccount } from '../../types/workbuddy';

type Node = { type: any; props: Record<string, any> };
function nodes(value: any): Node[] {
  if (Array.isArray(value)) return value.flatMap(nodes);
  return value?.props ? [value, ...nodes(value.props.children)] : [];
}
function text(value: any): string {
  if (Array.isArray(value)) return value.map(text).join('');
  if (value?.props) return text(value.props.children);
  return typeof value === 'string' || typeof value === 'number' ? String(value) : '';
}
const source = (id: string): WorkbuddyAccount => ({
  id, email: `${id}@example.invalid`, access_token: 'synthetic-not-a-token',
  created_at: 1, last_used: 1, status: 'normal',
});
const catalog = (model = 'glm-5.3-flash', effort = 'low'): CodexWorkbuddyDelegateCatalog => ({
  models: [{ id: model, name: model }], currentModelId: model,
  reasoningEfforts: effort ? [effort] : [], currentReasoningEffort: effort || null,
});
const profile = (id = 'wb-native-fixture'): CodexWorkbuddyDelegateProfile => ({
  id, apiKeyId: 'synthetic-scoped-key', workbuddyAccountId: 'removed-source',
  profileDir: '/synthetic/owned-codex-home', publicModel: 'workbuddy/glm-5.3-flash',
  modelId: 'glm-5.3-flash', reasoningEffort: 'low',
  launchCommand: "CODEX_HOME='/synthetic/owned-codex-home' codex",
});
interface Services {
  catalog?: (source: string, model?: string) => Promise<CodexWorkbuddyDelegateCatalog>;
  profiles?: () => Promise<CodexWorkbuddyDelegateProfile[]>;
  restore?: (id: string) => Promise<void>;
  create?: (source: string, model: string, effort: string) => Promise<CodexWorkbuddyDelegateProfile>;
}
function harness(accounts: WorkbuddyAccount[] = [source('source-a')], services: Services = {}) {
  const error = {
    message: '', scrollKey: 0,
    clear() { this.message = ''; },
    report(message: string) { this.message = message; this.scrollKey++; },
  };
  const calls = {
    catalogs: [] as [string, string | undefined][], restores: [] as string[],
    creates: [] as [string, string, string][], closed: 0,
  };
  const hook = loadHookModule(new URL('./WorkbuddyCodexNativeModal.tsx', import.meta.url), {
    'react-dom': { createPortal: (node: unknown) => node },
    'react-i18next': { useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }) },
    '../../hooks/useEscClose': { useEscCloseTopmost() {} },
    '../ModalErrorMessage': { ModalErrorMessage: 'modal-error', useModalErrorState: () => error },
    '../SingleSelectDropdown': { SingleSelectDropdown: 'single-select-dropdown' },
    '../../presentation/platformAccountPresentation': {
      buildWorkbuddyAccountPresentation: (account: WorkbuddyAccount) => ({
        planLabel: account.status ?? 'normal',
        planClass: 'free',
      }),
    },
    '../../types/workbuddy': { getWorkbuddyAccountDisplayEmail: (account: WorkbuddyAccount) => account.email },
    '../../services/codexService': {
      getCodexWorkbuddyDelegateCatalog(id: string, model?: string) {
        calls.catalogs.push([id, model]);
        return services.catalog?.(id, model) ?? Promise.resolve(catalog());
      },
      listCodexWorkbuddyDelegateProfiles() { return services.profiles?.() ?? Promise.resolve([]); },
      restoreCodexWorkbuddyDelegateProfile(id: string) {
        calls.restores.push(id);
        return services.restore?.(id) ?? Promise.resolve();
      },
      createCodexWorkbuddyDelegateProfile(id: string, model: string, effort: string) {
        calls.creates.push([id, model, effort]);
        return services.create?.(id, model, effort) ?? Promise.resolve(profile());
      },
    },
  }, { document: { body: {} } });
  let props = { accounts, onClose() { calls.closed++; } };
  const render = () => hook.render(() => hook.exports.WorkbuddyCodexNativeModal(props));
  render();
  const tree = () => nodes(hook.flush());
  const button = (label: string) => {
    const node = tree().find((item) => item.type === 'button' && text(item) === label);
    assert.ok(node, `missing button: ${label}`);
    return node;
  };
  const field = (id: string) => {
    const labelMap: Record<string, string> = {
      'wb-native-account': 'WorkBuddy 来源',
      'wb-native-model': 'CLI 实时模型',
      'wb-native-effort': '推理强度',
    };
    const ariaLabel = labelMap[id] ?? id;
    const node = tree().find((item) => item.type === 'single-select-dropdown' && item.props.ariaLabel === ariaLabel);
    assert.ok(node, `missing dropdown: ${id}`);
    return node;
  };
  return {
    hook, tree, button, field, calls, error,
    setAccounts(next: WorkbuddyAccount[]) { props = { ...props, accounts: next }; render(); },
    createButton: () => button('创建专属 Key 与独立配置'),
  };
}

test('managed profiles remain visible and restorable after all WorkBuddy sources are deleted', async () => {
  const h = harness([], { profiles: async () => [profile()] });
  await settlePromises();
  assert.deepEqual(h.calls.catalogs, [], 'management must not try to initialize a missing source');
  assert.equal(h.createButton().props.disabled, true);
  assert.ok(h.tree().some((node) => node.type === 'p' && text(node).includes('来源已删除')));
  assert.ok(h.tree().some((node) => node.type === 'pre' && text(node) === profile().launchCommand));
  h.button('撤销专属 Key 并恢复此配置').props.onClick();
  assert.deepEqual(h.calls.restores, [profile().id]);
  await settlePromises();
  assert.equal(h.tree().filter((node) => node.type === 'section').length, 0);
  assert.equal(h.calls.closed, 0);
  h.hook.unmount();
});

test('modified owned files report that the key is already revoked and retain a retryable management entry', async () => {
  const pending = deferred<void>();
  const h = harness([], { profiles: async () => [profile()], restore: () => pending.promise });
  await settlePromises();
  h.button('撤销专属 Key 并恢复此配置').props.onClick();
  assert.equal(h.createButton().props.disabled, true);
  pending.reject(new Error('WORKBUDDY_DELEGATE_KEY_REVOKED_FILES_PRESERVED'));
  await settlePromises();
  assert.match(h.error.message, /专属 Key 已撤销/);
  assert.match(h.error.message, /修改文件与会话数据均已保留/);
  assert.ok(h.tree().some((node) => node.props.role === 'status' && text(node).includes('Key 已撤销')));
  assert.equal(h.tree().some((node) => node.type === 'pre'), false, 'do not advertise a revoked launch command');
  assert.equal(h.button('重试恢复（Key 已撤销）').props.disabled, false);
  assert.equal(h.tree().filter((node) => node.type === 'section').length, 1);
  h.hook.unmount();
});

test('creation stays disabled while catalog, exact model, or exact effort is missing', async () => {
  const pending = deferred<CodexWorkbuddyDelegateCatalog>();
  const h = harness([source('source-a')], { catalog: () => pending.promise });
  assert.equal(h.createButton().props.disabled, true);
  h.createButton().props.onClick();
  assert.deepEqual(h.calls.creates, []);
  pending.resolve(catalog('glm-5.3-flash', ''));
  await settlePromises();
  assert.equal(h.createButton().props.disabled, true);
  h.createButton().props.onClick();
  assert.deepEqual(h.calls.creates, []);
  h.hook.unmount();

  const missingModel = harness([source('source-a')], { catalog: async () => catalog('', 'low') });
  await settlePromises();
  assert.equal(missingModel.createButton().props.disabled, true);
  missingModel.createButton().props.onClick();
  assert.deepEqual(missingModel.calls.creates, []);
  missingModel.hook.unmount();
});

test('creation forwards the newly selected model and its revalidated effort without translating them', async () => {
  const selected = deferred<CodexWorkbuddyDelegateCatalog>();
  const creating = deferred<CodexWorkbuddyDelegateProfile>();
  const h = harness([source('source-a')], {
    catalog: (_id, model) => model ? selected.promise : Promise.resolve(catalog()),
    create: () => creating.promise,
  });
  await settlePromises();
  assert.equal(h.createButton().props.disabled, false);
  assert.ok(h.field('wb-native-account').props.options[0].badge, 'account selector must display tier badge');
  h.field('wb-native-model').props.onChange('exact-model-2');
  assert.equal(h.createButton().props.disabled, true, 'old model effort must not authorize the new model');
  assert.equal(h.field('wb-native-effort').props.value, '');
  assert.deepEqual(h.calls.catalogs, [['source-a', undefined], ['source-a', 'exact-model-2']]);
  selected.resolve(catalog('exact-model-2', 'max'));
  await settlePromises();
  assert.equal(h.createButton().props.disabled, false);
  h.createButton().props.onClick();
  assert.deepEqual(h.calls.creates, [['source-a', 'exact-model-2', 'max']]);
  assert.equal(h.createButton().props.disabled, true, 'pending profile creation cannot submit another request');
  creating.resolve(profile());
  await settlePromises();
  h.hook.unmount();
});

test('enterprise source cannot enable profile creation even if metadata was returned', async () => {
  const h = harness([{ ...source('enterprise'), enterprise_id: 'synthetic-enterprise' }]);
  await settlePromises();
  assert.equal(h.createButton().props.disabled, true);
  assert.ok(h.tree().some((node) => node.props.role === 'note' && text(node).includes('仅支持个人')));
  h.hook.unmount();
});

test('a late catalog for a removed source cannot overwrite the replacement source selection', async () => {
  const old = deferred<CodexWorkbuddyDelegateCatalog>();
  const replacement = deferred<CodexWorkbuddyDelegateCatalog>();
  const h = harness([source('source-a')], {
    catalog: (id) => id === 'source-a' ? old.promise : replacement.promise,
  });
  h.setAccounts([source('source-b')]);
  assert.equal(h.field('wb-native-account').props.disabled, false);
  h.field('wb-native-account').props.onChange('source-b');
  assert.equal(h.createButton().props.disabled, true);
  replacement.resolve(catalog('replacement-model', 'high'));
  await settlePromises();
  assert.equal(h.field('wb-native-model').props.value, 'replacement-model');
  assert.equal(h.field('wb-native-effort').props.value, 'high');
  old.resolve(catalog('retired-model', 'low'));
  await settlePromises();
  assert.equal(h.field('wb-native-account').props.value, 'source-b');
  assert.equal(h.field('wb-native-model').props.value, 'replacement-model');
  assert.equal(h.field('wb-native-effort').props.value, 'high');
  assert.deepEqual(h.calls.catalogs, [['source-a', undefined], ['source-b', undefined]]);
  assert.equal(h.error.message, '');
  h.hook.unmount();
});

test('closing the modal ignores late catalog success and failure without changing selections or errors', async () => {
  for (const fails of [false, true]) {
    const pending = deferred<CodexWorkbuddyDelegateCatalog>();
    const h = harness([source('source-a')], { catalog: () => pending.promise });
    h.button('关闭').props.onClick();
    assert.equal(h.calls.closed, 1);
    h.hook.unmount(); // The owner removes the modal after onClose.
    const errorSequence = h.error.scrollKey;
    if (fails) pending.reject(new Error('WORKBUDDY_DELEGATE_SOURCE_UNAVAILABLE'));
    else pending.resolve(catalog('late-model', 'max'));
    await settlePromises();
    assert.equal(h.field('wb-native-model').props.value, '');
    assert.equal(h.field('wb-native-effort').props.value, '');
    assert.equal(h.error.message, '');
    assert.equal(h.error.scrollKey, errorSequence);
    assert.deepEqual(h.calls.creates, []);
  }
});

test('native bridge and text-only entry labels are registered in every shipped locale', () => {
  const sources = [
    new URL('./WorkbuddyCodexNativeModal.tsx', import.meta.url),
    new URL('../../pages/WorkbuddyAccountsPage.tsx', import.meta.url),
  ].map((url) => readFileSync(url, 'utf8')).join('\n');
  const keys = new Set([...sources.matchAll(/\bt\('(workbuddy\.(?:nativeCodex|exposeToCodex)\.[^']+)',/g)]
    .map((match) => match[1]));
  assert.equal(keys.size, 28); // Includes page.t() labels, not just bare t() calls.
  const locales = new URL('../../locales/', import.meta.url);
  const files = readdirSync(locales).filter((name) => name.endsWith('.json'));
  assert.equal(files.length, 18);
  for (const name of files) {
    const locale = JSON.parse(readFileSync(new URL(name, locales), 'utf8'));
    for (const key of keys) {
      const value = key.split('.').reduce((entry, part) => entry?.[part], locale);
      assert.ok(typeof value === 'string' && value.trim(), `${name}: missing ${key}`);
    }
  }
});
