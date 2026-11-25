import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';

test('account linking reads and writes the registered T3 Code commands with their argument names', async () => {
  const calls: Array<{ command: string; args: unknown }> = [];
  const workspace = { t3InstanceId: '__default__', providerInstances: [{ instanceId: 'google-work' }] };
  const switched = { workspace, appliedLive: true, restartRequired: false, message: 'Switch successful' };
  const runtime = [{ instanceId: 'google-work', modelCount: 2, authStatus: 'authenticated' }];
  const gatewayQuota = [{ id: '5h', label: '5h', remainingPercent: 199 }];
  const { exports: service } = loadHookModule(new URL('./t3codeService.ts', import.meta.url), {
    '@tauri-apps/api/core': {
      async invoke(command: string, args: unknown) {
        calls.push({ command, args });
        if (command === 'get_t3code_provider_runtime_status') return runtime;
        if (command === 'get_t3code_api_service_quota') return gatewayQuota;
        if (command === 'switch_t3code_provider_instance_account'
          || command === 'promote_t3code_provider_account'
          || command === 'remove_t3code_provider_instance') return switched;
        return workspace;
      },
    },
  });
  assert.equal(await service.getT3codeProviderWorkspace(null), workspace);
  assert.equal(await service.getT3codeApiServiceQuota(), gatewayQuota);
  assert.equal(await service.bindT3codeProviderInstanceAccount({
    t3InstanceId: 'work', providerInstanceId: 'google-work', driver: 'antigravity', cockpitAccountId: 'google-account',
  }), workspace);
  assert.equal(await service.switchT3codeProviderInstanceAccount({
    t3InstanceId: 'work', providerInstanceId: 'google-work', driver: 'antigravity', cockpitAccountId: 'other-account',
  }), switched);
  assert.equal(await service.createT3codeProviderInstance({
    t3InstanceId: null, instanceId: 'google-work', driver: 'antigravity', displayName: 'Work', bindAccountId: 'google-account',
  }), workspace);
  assert.equal(await service.getT3codeProviderRuntimeStatus('__default__'), runtime);
  assert.equal(await service.updateT3codeProviderInstance({
    t3InstanceId: null, instanceId: 'google-work', driver: 'antigravity', displayName: 'Work 2', enabled: false,
  }), workspace);
  assert.equal(await service.promoteT3codeProviderAccount({
    t3InstanceId: null, providerInstanceId: 'google-work', cockpitAccountId: 'google-account',
  }), switched);
  assert.equal(await service.removeT3codeProviderInstance({
    t3InstanceId: null, instanceId: 'google-work', cockpitAccountId: 'google-account',
  }), switched);
  assert.equal(await service.refreshT3codeProviderAccountQuota({
    t3InstanceId: null, providerInstanceId: 'google-work',
  }), workspace);
  assert.deepEqual(JSON.parse(JSON.stringify(calls)), [
    { command: 'get_t3code_provider_workspace', args: { t3InstanceId: null } },
    { command: 'get_t3code_api_service_quota' },
    { command: 'bind_t3code_provider_instance_account', args: {
      t3InstanceId: 'work', providerInstanceId: 'google-work', driver: 'antigravity', cockpitAccountId: 'google-account',
    } },
    { command: 'switch_t3code_provider_instance_account', args: {
      t3InstanceId: 'work', providerInstanceId: 'google-work', driver: 'antigravity', cockpitAccountId: 'other-account',
    } },
    { command: 'upsert_t3code_provider_instance', args: {
      t3InstanceId: null, instanceId: 'google-work', driver: 'antigravity',
      displayName: 'Work', bindAccountId: 'google-account', enabled: true, accentColor: null,
    } },
    { command: 'get_t3code_provider_runtime_status', args: { t3InstanceId: '__default__' } },
    { command: 'upsert_t3code_provider_instance', args: {
      t3InstanceId: null, instanceId: 'google-work', driver: 'antigravity',
      displayName: 'Work 2', enabled: false, accentColor: null, bindAccountId: null,
    } },
    { command: 'promote_t3code_provider_account', args: {
      t3InstanceId: null, providerInstanceId: 'google-work', cockpitAccountId: 'google-account',
    } },
    { command: 'remove_t3code_provider_instance', args: {
      t3InstanceId: null, instanceId: 'google-work', cockpitAccountId: 'google-account',
    } },
    { command: 'refresh_t3code_provider_account_quota', args: { t3InstanceId: null, providerInstanceId: 'google-work' } },
  ]);
});
