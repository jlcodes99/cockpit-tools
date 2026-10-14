import assert from 'node:assert/strict';
import test from 'node:test';
import type { CodexQuickConfig } from '../types/codex';
import type { InstanceProfile } from '../types/instance';
import { codexLaunchPreviewInstanceConfigKey, codexLaunchPreviewQuickConfigKey } from './codexLaunchPreviewConfig';

const instance: InstanceProfile = {
  id: 'profile-a', name: 'A', userDataDir: '/profiles/a', extraArgs: '',
  bindAccountId: 'account-a', createdAt: 1, running: false,
};
const config: CodexQuickConfig = {
  config_dir: '/profiles/a', context_config_key: 'context', model_catalog_config_key: 'catalog',
  context_window_1m: false, auto_compact_token_limit: 200000,
  experimental_model_catalog_enabled: true, experimental_model_catalog_available: true,
  experimental_model_catalog_models: [{ model_id: 'gpt-test', display_name: 'Test' }],
  experimental_model_catalog_reset_models: [], context_management_experimental_mode: false,
};

test('runtime refreshes and unrelated speed/name changes do not invalidate preview drafts', () => {
  assert.equal(codexLaunchPreviewInstanceConfigKey(instance), codexLaunchPreviewInstanceConfigKey({
    ...instance, running: true, lastPid: 123, lastLaunchedAt: 1234, name: 'renamed', appSpeed: 'fast',
  }));
});

test('instance scope, binding and routing changes invalidate stale preview writes', () => {
  for (const change of [
    { userDataDir: '/profiles/new' }, { bindAccountId: 'account-b' },
    { followLocalAccount: true }, { modelRouting: { enabled: true, version: 1, routes: [] } },
  ]) {
    assert.notEqual(codexLaunchPreviewInstanceConfigKey(instance), codexLaunchPreviewInstanceConfigKey({ ...instance, ...change }));
  }
});

test('configuration fingerprints use persistent fields and ignore display-only metadata', () => {
  assert.equal(codexLaunchPreviewQuickConfigKey(config), codexLaunchPreviewQuickConfigKey({
    ...config, experimental_model_catalog_default_model_id: null,
    experimental_model_catalog_reset_models: [{ model_id: 'reset-only', display_name: 'Reset' }],
    auto_compact_token_limit: 300000,
    experimental_model_catalog_models: [{ model_id: 'pool-model', display_name: 'Updated pool' }],
  }));
});

test('context and model changes must be detected before saving a draft', () => {
  for (const change of [
    { context_config_key: 'changed-context' },
    { model_catalog_config_key: 'changed-catalog' },
  ]) {
    assert.notEqual(codexLaunchPreviewQuickConfigKey(config), codexLaunchPreviewQuickConfigKey({ ...config, ...change }));
  }
});
