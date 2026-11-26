import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';
import type { ManagedApiKey } from '../services/apiKeyFunVault';


function nodes(tree: any): any[] {
  if (Array.isArray(tree)) return tree.flatMap(nodes);
  return tree?.props ? [tree, ...nodes(tree.props.children)] : [];
}


function setup() {
  let keys: ManagedApiKey[] = [{ id: 'test-key', name: 'Example key', key: 'synthetic-key', createdAt: 1, lastUsedAt: 1 }];
  let onKeysChanged: (keys: ManagedApiKey[]) => void;
  const writes: ReturnType<typeof deferred<void>>[] = [];
  const t = (key: string) => key;
  const hook = loadHookModule(new URL('./ApiKeyFunPage.tsx', import.meta.url), {
    '@tauri-apps/api/core': { isTauri: () => true },
    '@tauri-apps/plugin-dialog': {},
    '@tauri-apps/plugin-opener': {},
    'react-i18next': { useTranslation: () => ({ t }) },
    '../services/modelProviderUsageService': {},
    '../utils/apikeyFunLinks': { buildApiKeyFunProviderBaseUrl: () => 'https://example.test/v1' },
    '../utils/apiKeyFunPrefill': {},
    '../utils/apikeyFunClientConfigs': {
      isApiKeyFunGeminiModel: () => false,
      resolveApiKeyFunGrokModel: () => null,
    },
    '../components/apiKeyFun/ApiKeyFunKeyConfigModal': { ApiKeyFunKeyConfigModal: 'KeyConfig' },
    '../components/apiKeyFun/ApiKeyFunPromotionHeader': { ApiKeyFunPromotionHeader: 'PromotionHeader' },
    '../components/ModalErrorMessage': {
      ModalErrorMessage: 'ModalErrorMessage',
      useModalErrorState: () => {
        const [message, setMessage] = hook.hooks.useState(null);
        return { message, clear: () => setMessage(null), report: setMessage };
      },
    },
    '../stores/useSponsorStore': { useSponsorStore: (select: (value: unknown) => unknown) => select({ state: {} }) },
    '../services/apiKeyFunVault': {
      initializeApiKeyFunVault: async () => {},
