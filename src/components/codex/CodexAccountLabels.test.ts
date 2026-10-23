import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';
import { buildCodexAccountPresentation } from '../../presentation/platformAccountPresentation';

function nodes(tree: any): any[] {
  if (Array.isArray(tree)) return tree.flatMap(nodes);
  return tree && typeof tree === 'object' ? [tree, ...nodes(tree.props?.children)] : [];
}
const t = (key: string) => key === 'common.apiKeyAccount' ? 'API Key 账号' : key;

test('image picker renders real account labels and searches and sorts by the visible name', () => {
  const accounts = [
    { id: 'a', auth_mode: 'apikey', email: 'api-key-ffffffff', api_base_url: 'https://alpha.example/v1', api_model_catalog: ['gpt-image-2'] },
    { id: 'b', auth_mode: 'apikey', email: 'api-key-00000000', account_name: 'Zeta Images', api_base_url: 'https://beta.example/v1', api_model_catalog: ['gpt-image-2'] },
  ];
  const h = loadHookModule(new URL('./CodexImageAccountPickerModal.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t }) },
    '../../hooks/useModalFocusTrap': { useModalFocusTrap() {} },
    '../../hooks/useEscClose': { useEscCloseTopmost() {} },
    '../../hooks/usePagination': { usePagination: ({ items }: any) => ({ pageItems: items }) },
    './useCodexImageModelCatalog': { useCodexImageModelCatalog: () => ({ deleted: [], ready: true, syncError: '', retry() {} }) },
    '../MultiSelectFilterDropdown': { MultiSelectFilterDropdown: 'filter' },
    '../SingleSelectDropdown': { SingleSelectDropdown: 'dropdown' },
    '../ModalErrorMessage': { ModalErrorMessage: 'modal-error' },
    '../PaginationControls': { PaginationControls: 'pagination' },
    '../SingleSelectFilterDropdown': { SingleSelectFilterDropdown: 'sort' },
    '../../presentation/platformAccountPresentation': { buildCodexAccountPresentation },
    '../../utils/privacy': { isPrivacyModeEnabledByDefault: () => false, maskSensitiveValue: (value: string) => value },
  });
  h.render(() => h.exports.CodexImageAccountPickerModal({
    accounts, selectedIds: [], imageGenerationModel: 'gpt-image-2', onCancel() {}, onConfirm() {},
  }));
  const rows = () => nodes(h.flush()).filter((node) => node.props.className?.includes('codex-image-account-row') && node.props['aria-label']);
  assert.deepEqual(rows().map((row) => row.props['aria-label']), ['alpha.example', 'Zeta Images']);
  nodes(h.flush()).find((node) => node.type === 'sort')!.props.onChange('account');
  assert.deepEqual(rows().map((row) => row.props['aria-label']), ['Zeta Images', 'alpha.example']);
  const search = () => nodes(h.flush()).find((node) => node.type === 'input' && node.props.type === 'text');
  search()!.props.onChange({ target: { value: 'alpha.example' } });
  assert.deepEqual(rows().map((row) => row.props['aria-label']), ['alpha.example']);
  search()!.props.onChange({ target: { value: 'Zeta' } });
  assert.deepEqual(rows().map((row) => row.props['aria-label']), ['Zeta Images']);
  h.unmount();
});

test('request attempts use current labels and privacy masking, with readable deleted-account fallback', async () => {
  const h = loadHookModule(new URL('./CodexRequestDetailModal.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t }) },
    'react-dom': { createPortal: (value: unknown) => value },
    '../../hooks/useEscClose': { useEscCloseTopmost() {} },
    '../../hooks/useModalFocusTrap': { useModalFocusTrap() {} },
    '../../hooks/useModalScrollLock': { useModalScrollLock() {} },
    '../SingleSelectDropdown': { SingleSelectDropdown: 'dropdown' },
    '../ModalErrorMessage': { ModalErrorMessage: 'modal-error' },
    './CodexOutputSpeed': { CodexOutputSpeed: 'speed' },
    './CodexCapabilityFallbackTrace': { CodexCapabilityFallbackTrace: 'CodexCapabilityFallbackTrace' },
    './CodexAuthorizationFallbackTrace': { CodexAuthorizationFallbackTrace: 'CodexAuthorizationFallbackTrace' },
    '../../services/codexLocalAccessService': {
      async getCodexLocalAccessRequestDetail() {
        return { attempts: [
          { sequence: 1, accountId: 'current', accountEmail: 'api-key-ffffffff', success: true, transport: 'http', latencyMs: 12 },
          { sequence: 2, accountId: 'codex_apikey_deleted', accountEmail: 'api-key-00000000', success: true, transport: 'http', latencyMs: 10 },
        ], payloads: [] };
      },
    },
  }, { document: { body: {} } });
  h.render(() => h.exports.CodexRequestDetailModal({
    event: { requestId: 'request', latencyMs: 22 },
    accountDisplayNames: new Map([['current', 'alpha.example']]),
    maskAccountText: (name: string) => `masked:${name}`, onClose() {},
  }));
  await settlePromises();
  const labels = nodes(h.flush()).filter((node) => node.type === 'strong').map((node) => node.props.children);
  assert.ok(labels.includes('masked:alpha.example'));
  assert.ok(labels.includes('masked:API Key 账号'));
  assert.equal(labels.some((label) => typeof label === 'string' && label.includes('api-key-')), false);
  h.unmount();
});
