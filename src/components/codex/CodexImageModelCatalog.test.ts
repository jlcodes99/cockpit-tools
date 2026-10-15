import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';
function nodes(tree: any): any[] {
  if (Array.isArray(tree)) return tree.flatMap(nodes);
  return tree && typeof tree === 'object' ? [tree, ...nodes(tree.props?.children)] : [];
}
const t = (key: string) => key;
const models = ['gpt-image-2.5', 'gpt-image-2'];
const noopTimers = { setTimeout: () => 1, clearTimeout() {} };

test('catalog management localizes the existing model-configuration busy error', async () => {
  const h = loadHookModule(new URL('../CodexImageModelConfig.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => `translated:${key}` }) },
    './SingleSelectDropdown': { SingleSelectDropdown: 'dropdown' }, './ModalErrorMessage': { ModalErrorMessage: 'error' },
    './codex/useCodexImageModelCatalog': { useCodexImageModelCatalog: () => ({ models, syncError: '', update: async () => { throw new Error('codex.modelConfig.errors.busy'); } }) },
  });
  let tree = h.render(() => h.exports.CodexImageToolModelSelect({ model: 'gpt-image-2.5', disabled: false, onSave() {} }));
  nodes(tree).find((node) => node.props?.className?.includes('codex-image-model-manage')).props.onClick(); tree = h.flush();
  nodes(tree).find((node) => node.props?.['aria-label'] === 'translated:common.delete: gpt-image-2').props.onClick();
  await settlePromises(); tree = h.flush();
  assert.equal(nodes(tree).find((node) => node.type === 'error').props.message, 'translated:codex.modelConfig.errors.busy');
  h.unmount();
});

test('catalog IPC timeout releases discovery and edits; late results never replace a recovered snapshot', async () => {
  const requests: ReturnType<typeof deferred<any>>[] = [];
  const timers = new Map<number, () => void>();
  let nextTimer = 0;
  const h = loadHookModule(new URL('./useCodexImageModelCatalog.ts', import.meta.url), {
    '../../stores/useCodexAccountStore': { useCodexAccountStore: (selector: any) => selector({ accounts: [] }) },
    '../../services/codexLocalAccessService': { updateCodexLocalAccessImageGenerationModelCatalog: () => {
      const request = deferred<any>(); requests.push(request); return request.promise;
    } },
  }, { setTimeout(callback: () => void) { timers.set(++nextTimer, callback); return nextTimer; }, clearTimeout(id: number) { timers.delete(id); } });
  let entry = h.render(() => h.exports.useCodexImageModelCatalog('gpt-image-2.5'));
  await settlePromises(); assert.equal(requests.length, 1);
  timers.get(1)!(); await settlePromises(); entry = h.flush();
  assert.equal(entry.syncError, 'codex.localAccess.imageGenerationModel.syncFailed');
  entry.retry(); h.flush(); await settlePromises(); assert.equal(requests.length, 2);
  const recovered = { collection: { imageGenerationModels: ['gpt-image-2.5'], imageGenerationDeletedModels: ['gpt-image-2'], updatedAt: 2 } };
  requests[1].resolve(recovered); await settlePromises(); entry = h.flush();
  assert.equal(entry.ready, true); assert.deepEqual(entry.deleted, ['gpt-image-2']);
  const edited = entry.update('gpt-image-2.5-high'); await settlePromises(); assert.equal(requests.length, 3);
  requests[2].resolve({ collection: { ...recovered.collection, imageGenerationModels: ['gpt-image-2.5', 'gpt-image-2.5-high'], updatedAt: 3 } });
  await edited; entry = h.flush();
  requests[0].resolve({ collection: { imageGenerationModels: models, imageGenerationDeletedModels: [], updatedAt: 1 } });
  await settlePromises(); entry = h.flush();
  assert.equal(entry.models.includes('gpt-image-2.5-high'), true); assert.deepEqual(entry.deleted, ['gpt-image-2']);
  h.unmount();
});

test('an expired queued edit is never started after the previous IPC times out', async () => {
  const requests: ReturnType<typeof deferred<any>>[] = [];
  const timers = new Map<number, () => void>();
  let nextTimer = 0;
  const h = loadHookModule(new URL('./useCodexImageModelCatalog.ts', import.meta.url), {
    '../../stores/useCodexAccountStore': { useCodexAccountStore: (selector: any) => selector({ accounts: [] }) },
    '../../services/codexLocalAccessService': { updateCodexLocalAccessImageGenerationModelCatalog: () => {
      const request = deferred<any>(); requests.push(request); return request.promise;
    } },
  }, { setTimeout(callback: () => void) { timers.set(++nextTimer, callback); return nextTimer; }, clearTimeout(id: number) { timers.delete(id); } });
  const entry = h.render(() => h.exports.useCodexImageModelCatalog('gpt-image-2.5'));
  await settlePromises(); const edit = entry.update('gpt-image-2.5-expired');
  const failure = assert.rejects(edit, /syncFailed/);
  timers.get(2)!(); await failure;
  timers.get(1)!(); await settlePromises();
  assert.equal(requests.length, 1, 'the failed queued edit must not invoke IPC later');
  const retry = entry.update('gpt-image-2.5-new'); await settlePromises(); assert.equal(requests.length, 2);
  requests[1].resolve({ collection: { imageGenerationModels: ['gpt-image-2.5-new'], imageGenerationDeletedModels: [], updatedAt: 2 } });
  await retry; h.unmount();
});

test('catalog discovery is single-flight across mounted entries, persists across account removal, and broadcasts delete/restore', async () => {
  let accounts: any[] = [{ id: 'api', api_model_catalog: ['gpt-image-2.5-high', 'grok-imagine-image-pro', 'grok-imagine-image-video'] }];
  let backendModels = [...models];
  let backendDeleted: string[] = [];
  let stamp = 10;
  const calls: any[] = [];
  const h = loadHookModule(new URL('./useCodexImageModelCatalog.ts', import.meta.url), {
    '../../stores/useCodexAccountStore': { useCodexAccountStore: (selector: any) => selector({ accounts }) },
    '../../services/codexLocalAccessService': { updateCodexLocalAccessImageGenerationModelCatalog: async (discovered?: string[], add?: string, remove?: string) => {
      calls.push({ discovered, add, remove });
      for (const id of discovered ?? []) if (!backendDeleted.includes(id) && !backendModels.includes(id)) backendModels.push(id);
      if (remove) { backendModels = backendModels.filter((id) => id !== remove); backendDeleted = [...new Set([...backendDeleted, remove])]; }
      if (add) { backendModels = [...new Set([...backendModels, add])]; backendDeleted = backendDeleted.filter((id) => id !== add); }
      return { collection: { imageGenerationModels: [...backendModels], imageGenerationDeletedModels: [...backendDeleted], updatedAt: ++stamp } };
    } },
  }, noopTimers);
  let propsDeleted: string[] = [];
  const render = () => [h.exports.useCodexImageModelCatalog('gpt-image-2.5', models, propsDeleted, 9), h.exports.useCodexImageModelCatalog('gpt-image-2.5', models, propsDeleted, 9)];
  let entries = h.render(render);
  await settlePromises(); entries = h.flush();
  assert.equal(calls.length, 1);
  assert.equal(entries[0].models.includes('gpt-image-2.5-high'), true);
  assert.equal(entries[1].models.includes('grok-imagine-image-pro'), true);
  assert.equal(entries[0].models.some((id: string) => id.includes('video')), false);
  await entries[0].update(undefined, 'gpt-image-2.5-high'); entries = h.flush();
  assert.equal(entries[0].models.includes('gpt-image-2.5-high'), false);
  assert.equal(entries[1].models.includes('gpt-image-2.5-high'), false);
  await entries[1].update('gpt-image-2.5-high'); entries = h.flush();
  assert.equal(entries[0].models.includes('gpt-image-2.5-high'), true);
  assert.equal(entries[1].models.includes('gpt-image-2.5-high'), true);
  assert.deepEqual(backendDeleted, []);
  propsDeleted = ['gpt-image-2.5-high']; entries = h.render(render);
  assert.equal(entries[0].models.includes('gpt-image-2.5-high'), true, 'stale props cannot reintroduce a deleted marker');
  accounts = []; entries = h.flush();
  assert.equal(entries[0].models.includes('gpt-image-2.5-high'), true);
  assert.equal(entries[0].models.includes('grok-imagine-image-pro'), true);
  await settlePromises(); assert.equal(calls.length, 3);
  h.unmount();
});

test('known discoveries still hydrate authoritative deleted markers and do not resurrect a deleted preset', async () => {
  const request = deferred<any>();
  let calls = 0;
  let discovered: any;
  const h = loadHookModule(new URL('./useCodexImageModelCatalog.ts', import.meta.url), {
    '../../stores/useCodexAccountStore': { useCodexAccountStore: (selector: any) => selector({ accounts: [{ id: 'api', api_model_catalog: models }] }) },
    '../../services/codexLocalAccessService': { updateCodexLocalAccessImageGenerationModelCatalog: (ids: any) => { calls++; discovered = ids; return request.promise; } },
  }, noopTimers);
  h.render(() => h.exports.useCodexImageModelCatalog('gpt-image-2.5', models, [], 1));
  await settlePromises(); assert.equal(calls, 1); assert.equal(discovered.length, 0);
  request.resolve({ collection: { imageGenerationModels: ['gpt-image-2.5'], imageGenerationDeletedModels: ['gpt-image-2'], updatedAt: 2 } });
  await settlePromises(); const entry = h.flush();
  assert.equal(entry.models.includes('gpt-image-2'), false);
  await settlePromises(); assert.equal(calls, 1);
  h.unmount();
});

test('missing deletion metadata keeps only the selected model visible until background hydration', async () => {
  const request = deferred<any>();
  const accounts: any[] = [];
  const h = loadHookModule(new URL('./useCodexImageModelCatalog.ts', import.meta.url), {
    '../../stores/useCodexAccountStore': { useCodexAccountStore: (selector: any) => selector({ accounts }) },
    '../../services/codexLocalAccessService': { updateCodexLocalAccessImageGenerationModelCatalog: () => request.promise },
  }, noopTimers);
  let entry = h.render(() => h.exports.useCodexImageModelCatalog('gpt-image-2.5', models));
  assert.equal(entry.models.includes('gpt-image-2'), false);
  request.resolve({ collection: { imageGenerationModels: ['gpt-image-2.5'], imageGenerationDeletedModels: ['gpt-image-2'], updatedAt: 2 } });
  await settlePromises(); entry = h.flush(); assert.equal(entry.models.includes('gpt-image-2'), false);
  h.unmount();
});

test('top presets stay as families while management persists actual IDs and custom input selects an exact full model', async () => {
  let catalog = [...models];
  const operations: any[] = [];
  const saved: string[] = [];
  let selected = 'gpt-image-2.5';
  const h = loadHookModule(new URL('../CodexImageModelConfig.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t }) },
    './SingleSelectDropdown': { SingleSelectDropdown: 'dropdown' },
    './ModalErrorMessage': { ModalErrorMessage: 'error' },
    './codex/useCodexImageModelCatalog': { useCodexImageModelCatalog: () => ({ models: catalog, syncError: '', update: async (add?: string, remove?: string) => {
      operations.push({ add, remove });
      if (add) catalog = [...new Set([...catalog, add])];
      if (remove) catalog = catalog.filter((id) => id !== remove);
    } }) },
  });
  let tree = h.render(() => h.exports.CodexImageToolModelSelect({ model: selected, disabled: false, onSave: async (id: string) => { selected = id; saved.push(id); } }));
  const dropdown = (tree: any) => nodes(tree).find((node) => node.type === 'dropdown');
  nodes(tree).find((node) => node.props?.className?.includes('codex-image-model-manage')).props.onClick(); tree = h.flush();
  nodes(tree).find((node) => node.type === 'input').props.onChange({ target: { value: 'gpt-image-2.5-high' } }); tree = h.flush();
  nodes(tree).find((node) => node.type === 'button' && [node.props.children].flat().includes('common.add')).props.onClick();
  await settlePromises(); tree = h.flush();
  assert.equal(dropdown(tree).props.value, 'gpt-image-2.5');
  assert.equal(dropdown(tree).props.options.some((option: any) => option.value === 'gpt-image-2.5-high'), false);
  assert.equal(catalog.includes('gpt-image-2.5-high'), true); assert.deepEqual(saved, []);
  dropdown(tree).props.onChange('__custom__'); tree = h.flush();
  const custom = nodes(tree).find((node) => node.props?.className === 'codex-image-model-custom-row');
  nodes(custom).find((node) => node.type === 'input').props.onChange({ target: { value: 'gpt-image-2.5-high' } }); tree = h.flush();
  nodes(tree).find((node) => node.type === 'button' && [node.props.children].flat().includes('codex.localAccess.imageGenerationModel.save')).props.onClick();
  await settlePromises(); tree = h.flush();
  assert.equal(dropdown(tree).props.value, '__custom__');
  assert.equal(nodes(nodes(tree).find((node) => node.props?.className === 'codex-image-model-custom-row')).find((node) => node.type === 'input').props.value, 'gpt-image-2.5-high');
  assert.deepEqual(saved, ['gpt-image-2.5-high']);
  nodes(tree).find((node) => node.props?.['aria-label'] === 'common.delete: gpt-image-2.5-high').props.onClick();
  await settlePromises(); tree = h.flush();
  assert.equal(nodes(tree).find((node) => node.type === 'error').props.message, 'codex.localAccess.imageGenerationModel.deleteSelected');
  assert.equal(operations.filter((operation) => operation.remove).length, 0);
  nodes(tree).find((node) => node.props?.['aria-label'] === 'common.delete: gpt-image-2').props.onClick();
  await settlePromises(); tree = h.flush();
  assert.equal(catalog.includes('gpt-image-2'), false);
  assert.equal(dropdown(tree).props.options.some((option: any) => option.value === 'gpt-image-2'), true, 'family remains selectable independently of the actual bare tombstone');
  h.unmount();
});

test('manual invalid model IDs stay near the field and never invoke catalog or selection writes', async () => {
  let writes = 0;
  let focuses = 0;
  const h = loadHookModule(new URL('../CodexImageModelConfig.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t }) },
    './SingleSelectDropdown': { SingleSelectDropdown: 'dropdown' },
    './ModalErrorMessage': { ModalErrorMessage: 'error' },
    './codex/useCodexImageModelCatalog': { useCodexImageModelCatalog: () => ({ models, syncError: '', update: async () => { writes++; } }) },
  });
  let tree = h.render(() => h.exports.CodexImageToolModelSelect({ model: 'gpt-image-2.5', disabled: false, onSave: async () => { writes++; } }));
  nodes(tree).find((node) => node.props?.className?.includes('codex-image-model-manage')).props.onClick(); tree = h.flush();
  const input = nodes(tree).find((node) => node.type === 'input');
  input.props.ref.current = { focus() { focuses++; } };
  input.props.onChange({ target: { value: 'gpt-image bad' } }); tree = h.flush();
  nodes(tree).find((node) => node.type === 'button' && [node.props.children].flat().includes('common.add')).props.onClick();
  await settlePromises(); tree = h.flush();
  assert.equal(writes, 0); assert.equal(focuses, 1);
  assert.equal(nodes(tree).find((node) => node.type === 'input').props['aria-invalid'], true);
  assert.equal(nodes(tree).find((node) => node.type === 'error').props.message, 'codex.localAccess.imageGenerationModel.invalid');
  assert.equal(nodes(tree).find((node) => node.props?.className?.includes('codex-image-model-manage')).props['aria-expanded'], true);
  h.unmount();
});
