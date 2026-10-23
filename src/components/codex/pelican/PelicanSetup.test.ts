import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule, settlePromises } from '../../../../tests/helpers/reactHookHarness';
import { CANDY_REASONING_PROMPT, PELICAN_BENCHMARKS, PELICAN_BENCHMARK_IDS } from './pelicanBenchmarks.ts';

function nodes(tree: any): any[] {
  if (Array.isArray(tree)) return tree.flatMap(nodes);
  return tree && typeof tree === 'object' ? [tree, ...nodes(tree.props?.children)] : [];
}
const t = (key: string) => key;

test('both account and provider entries submit built-in prompts without reference answers or grading rules', async () => {
  for (const sourceMode of ['accounts', 'providers']) {
    for (const id of PELICAN_BENCHMARK_IDS.filter((key) => key !== 'animation' && key !== 'custom')) {
      const requests: any[] = [];
      const state = {
        sourceMode, providerTargets: [{ providerId: 'p', apiKeyId: 'k', model: 'saved-model' }],
        selectedAccountIds: ['one'], starting: false,
        draft: { model: 'selected-model', effort: 'high', prompt: PELICAN_BENCHMARKS.animation as string, concurrency: 3 },
        updateDraft(patch: any) { Object.assign(state.draft, patch); },
        async start(request: any) { requests.push(request); },
      };
      const store = Object.assign((select: any) => select(state), { getState: () => state });
      const h = loadHookModule(new URL('./PelicanSetup.tsx', import.meta.url), {
        'react-i18next': { useTranslation: () => ({ t }) },
        './PelicanProviderSetup': { PelicanProviderSetup: 'provider-setup' },
        './PelicanAccountSummary': { PelicanAccountSummary: 'account-summary' },
        '../../../stores/useCodexPelicanStore': { useCodexPelicanStore: store },
        '../../../stores/useCodexAccountStore': { useCodexAccountStore: (select: any) => select({ accounts: [{ id: 'one', email: 'one@example.com' }] }) },
        '../../../services/codexWakeupService': { getCodexWakeupState: async () => ({ model_presets: [] }) },
        '../../../types/codex': { isCodexApiKeyAccount: () => false, isCodexWebSessionAccount: () => false },
        '../../SingleSelectDropdown': { SingleSelectDropdown: 'dropdown' },
        '../../ModalErrorMessage': { ModalErrorMessage: 'error', useModalErrorState: () => ({ clear() {}, set() {} }) },
      });
      h.render(() => h.exports.PelicanSetup({ mask: (value: string) => value }));
      const cases = () => nodes(h.flush()).find((node) => node.type === 'dropdown' && node.props.ariaLabel === 'pelican.benchmark');
      assert.equal(cases().props.value, 'animation');
      assert.equal(cases().props.options.length, PELICAN_BENCHMARK_IDS.length);
      assert.ok(!cases().props.options.some((option: any) => option.value === 'livezebra2'));
      assert.equal(nodes(h.flush()).find((node) => node.type === 'textarea').props.maxLength, 100000);
      cases().props.onChange(id);
      assert.equal(cases().props.value, id);
      const textarea = () => nodes(h.flush()).find((node) => node.type === 'textarea');
      assert.equal(textarea().props.value, PELICAN_BENCHMARKS[id]);
      h.flush().props.onSubmit({ preventDefault() {} });
      await settlePromises();
      assert.equal(requests.length, 1);
      assert.equal(requests[0].prompt, id === 'candy' ? CANDY_REASONING_PROMPT : PELICAN_BENCHMARKS[id]);
      assert.equal(requests[0].rules, undefined);
      assert.equal(requests[0].answer, undefined);
      assert.equal(requests[0].effort, 'high');
      assert.equal(requests[0].model, sourceMode === 'providers' ? 'saved-model' : 'selected-model');
      assert.deepEqual(Array.from(requests[0].accountIds), sourceMode === 'providers' ? [] : ['one']);
      assert.equal(requests[0].providerTargets?.[0].apiKeyId, sourceMode === 'providers' ? 'k' : undefined);
      textarea().props.onChange({ target: { value: 'Return only {"answer":42}' } });
      assert.equal(cases().props.value, 'custom');
      assert.equal(state.draft.prompt, 'Return only {"answer":42}');
      h.unmount();
    }
  }
});


test('a restored retired prompt keeps its exact text and a labeled selection without reintroducing it for new tests', () => {
  const prompt = PELICAN_BENCHMARKS.livezebra2;
  const state = {
    sourceMode: 'accounts', providerTargets: [], selectedAccountIds: [], starting: false,
    draft: { prompt, model: 'test', effort: 'high', concurrency: 1 }, updateDraft() {},
  };
  const h = loadHookModule(new URL('./PelicanSetup.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t }) },
    './PelicanProviderSetup': { PelicanProviderSetup: 'provider-setup' },
    './PelicanAccountSummary': { PelicanAccountSummary: 'account-summary' },
    '../../../stores/useCodexPelicanStore': { useCodexPelicanStore: (select: any) => select(state) },
    '../../../stores/useCodexAccountStore': { useCodexAccountStore: (select: any) => select({ accounts: [] }) },
    '../../../services/codexWakeupService': { getCodexWakeupState: async () => ({ model_presets: [] }) },
    '../../../types/codex': { isCodexApiKeyAccount: () => false, isCodexWebSessionAccount: () => false },
    '../../SingleSelectDropdown': { SingleSelectDropdown: 'dropdown' },
    '../../ModalErrorMessage': { ModalErrorMessage: 'error', useModalErrorState: () => ({ clear() {}, set() {} }) },
  });
  h.render(() => h.exports.PelicanSetup({ mask: (value: string) => value }));
  const tree = nodes(h.flush());
  const cases = tree.find((node) => node.type === 'dropdown' && node.props.ariaLabel === 'pelican.benchmark');
  assert.equal(cases.props.value, 'livezebra2');
  assert.match(cases.props.options.find((option: any) => option.value === 'livezebra2').label, /pelican.legacyCase/);
  assert.equal(cases.props.options.length, PELICAN_BENCHMARK_IDS.length + 1);
  assert.equal(tree.find((node) => node.type === 'textarea').props.value, prompt);
  assert.equal(state.draft.prompt, prompt);
  h.unmount();
});
