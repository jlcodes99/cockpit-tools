import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../../tests/helpers/reactHookHarness';

function nodes(tree: any): any[] {
  if (Array.isArray(tree)) return tree.flatMap(nodes);
  return tree && typeof tree === 'object' ? [tree, ...nodes(tree.props?.children)] : [];
}

function harness() {
  return loadHookModule(new URL('./CodexApiKeyLaunchSection.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../../stores/useCodexAccountStore': { useCodexAccountStore: (select: any) => select({ accounts: [] }) },
    './CodexImageAccountPickerModal': { CodexImageAccountPickerModal: 'image-picker' },
  });
}

test('API-key form drafts restore direct access and expose generic provider choices', () => {
  const h = harness();
  const account = { api_provider_id: 'custom', api_wire_api: 'responses', api_instance_access_mode: 'direct',
    api_image_generation_account_ids: ['stale-image-fixture'] };
  let options = h.exports.createCodexApiKeyLaunchOptions(account);
  assert.equal(options.accessMode, 'direct');
  assert.equal(options.imageGenEnabled, false);
  assert.equal(options.imageGenAccountIds.length, 0);
  const tree = h.render(() => h.exports.CodexApiKeyLaunchSection({ account, options,
    onChange: (next: any) => { options = next; } }));
  const buttons = nodes(tree).filter((node) => node.type === 'button' && node.props.className?.includes('codex-api-key-launch-mode '));
  assert.equal(buttons.length, 3);
  assert.equal(buttons.find((node) => node.props.className.includes('active')).key, 'direct');
  buttons.find((node) => node.key === 'gateway').props.onClick();
  assert.equal(options.accessMode, 'gateway');
  options = { ...options, imageGenEnabled: true, imageGenAccountIds: ['image-fixture'] };
  const next = h.flush();
  nodes(next).find((node) => node.type === 'button' && node.key === 'direct').props.onClick();
  assert.equal(options.imageGenEnabled, false);
  assert.equal(options.imageGenAccountIds.length, 0);
  h.unmount();
});

test('local-gateway display honors saved direct and CDP modes', () => {
  const h = harness();
  const account = { auth_mode: 'apikey', api_provider_id: 'custom', api_wire_api: 'responses',
    api_provider_mode: 'custom', api_sync_model_catalog_to_codex: true };
  assert.equal(h.exports.accountUsesInstanceLocalGateway(account), true);
  for (const mode of ['direct', 'cdp']) {
    assert.equal(h.exports.accountUsesInstanceLocalGateway({ ...account, api_instance_access_mode: mode }), false);
  }
  assert.equal(h.exports.accountUsesInstanceLocalGateway({ ...account, api_wire_api: 'chat_completions', api_instance_access_mode: 'direct' }), true);
  assert.equal(h.exports.accountUsesInstanceLocalGateway({ ...account, upstream_grok_account_id: 'grok-fixture', api_instance_access_mode: 'direct' }), true);
});
