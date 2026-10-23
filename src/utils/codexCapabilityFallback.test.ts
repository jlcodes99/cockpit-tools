import assert from 'node:assert/strict';
import test from 'node:test';
import { CODEX_CAPABILITY_IDS } from '../types/codexLocalAccess';
import { capabilityFallbackErrorKey, capabilityFallbackFingerprint, copyCapabilityFallback, declaredCapability, hasCapabilityFallbackErrors, isCapabilityProtocolUnsupported, validateCapabilityFallback } from './codexCapabilityFallback';
import type { CodexAccount } from '../types/codex';

test('fallback is automatic, copies ordering and never declares unknown tools supported', () => {
  const config = copyCapabilityFallback();
  assert.deepEqual(config, { enabled: true, accountIds: [], overrides: [] });
  assert.equal(CODEX_CAPABILITY_IDS.length, 17);
  assert.equal((CODEX_CAPABILITY_IDS as readonly string[]).includes('unknown_tool'), false);
  assert.equal(declaredCapability(config, 'a', 'model', 'function'), 'unknown');
  const value = { enabled: true, accountIds: ['b', 'a'], overrides: [{ accountId: 'a', model: '*', capabilities: { function: 'supported' as const } }] };
  const copy = copyCapabilityFallback(value);
  copy.accountIds.reverse(); copy.overrides[0].capabilities.function = 'unsupported';
  assert.deepEqual(value.accountIds, ['b', 'a']);
  assert.equal(value.overrides[0].capabilities.function, 'supported');
  assert.notEqual(capabilityFallbackFingerprint(value), capabilityFallbackFingerprint(copy));
  assert.equal(declaredCapability(value, 'a', 'model', 'unknown_tool' as any), 'unknown');
});

test('exact model declarations override one capability while inheriting other wildcard declarations', () => {
  const value = { enabled: true, accountIds: ['a'], overrides: [
    { accountId: 'a', model: '*', capabilities: { function: 'supported' as const, image_input: 'unsupported' as const } },
    { accountId: 'a', model: 'Model-A', capabilities: { image_input: 'supported' as const } },
  ] };
  assert.equal(declaredCapability(value, 'a', 'Model-A', 'image_input'), 'supported');
  assert.equal(declaredCapability(value, 'a', 'Model-A', 'function'), 'supported');
  assert.equal(declaredCapability(value, 'a', 'model-a', 'image_input'), 'unsupported', 'model IDs remain exact');
  assert.equal(declaredCapability(value, 'other', 'Model-A', 'function'), 'unknown');
});

test('automatic fallback accepts an empty preferred pool but rejects foreign accounts, invalid patterns and duplicate declarations', () => {
  const members = new Set(['a', 'b']);
  assert.equal(hasCapabilityFallbackErrors(validateCapabilityFallback(copyCapabilityFallback(), members)), false);
  assert.equal(validateCapabilityFallback({ enabled: false, accountIds: [], overrides: [] }, members).backups, undefined);
  assert.equal(validateCapabilityFallback({ enabled: true, accountIds: ['foreign'], overrides: [] }, members).backups, 'capabilityFallback.errorMembers');
  const overrides = [
    { accountId: 'foreign', model: '*', capabilities: {} },
    { accountId: 'a', model: 'model*', capabilities: {} },
    { accountId: 'b', model: ' Model ', capabilities: {} },
    { accountId: 'b', model: 'Model', capabilities: {} },
  ];
  const errors = validateCapabilityFallback({ enabled: true, accountIds: ['a'], overrides }, members);
  assert.equal(errors.overrides[0].account, 'capabilityFallback.errorMembers');
  assert.equal(errors.overrides[1].model, 'capabilityFallback.errorModel');
  assert.equal(errors.overrides[3].model, 'capabilityFallback.errorDuplicate');
  for (const model of ['x'.repeat(257), '模'.repeat(86), 'model\u0000id', 'model\u0085id']) {
    assert.equal(validateCapabilityFallback({ enabled: false, accountIds: [], overrides: [{ accountId: 'a', model, capabilities: {} }] }, members).overrides[0].model, 'capabilityFallback.errorModel');
  }
  assert.deepEqual(validateCapabilityFallback({ enabled: false, accountIds: [], overrides: [{ accountId: 'a', model: 'x'.repeat(256), capabilities: {} }] }, members).overrides, {});
});

test('settings failures expose only translated allowlisted codes', () => {
  assert.equal(capabilityFallbackErrorKey('CAPABILITY_FALLBACK_NATIVE_REQUIRED'), 'capabilityFallback.nativeOnly');
  assert.equal(capabilityFallbackErrorKey(new Error('CAPABILITY_FALLBACK_INVALID_MODEL')), 'capabilityFallback.errorModel');
  for (const error of ['https://user:secret@host/?token=secret', 'CAPABILITY_FALLBACK_STORAGE: private data']) {
    assert.equal(capabilityFallbackErrorKey(error), 'capabilityFallback.errorGeneric');
  }
});

test('hard protocol exclusions mirror native rules without inferring support or classifying by provider hostname', () => {
  const account = (values: Partial<CodexAccount>) => ({ id: 'a', ...values } as CodexAccount);
  const plan = account({ auth_mode: 'apikey', api_provider_id: 'cockpit_chatgpt_plan', api_wire_api: 'responses' });
  const chat = account({ auth_mode: 'apikey', api_wire_api: 'chat_completions' });
  for (const id of ['tool_search', 'image_generation', 'file_search', 'code_interpreter', 'computer_use', 'mcp', 'programmatic_tool_calling', 'compact'] as const) {
    assert.equal(isCapabilityProtocolUnsupported(plan, id), true, id);
    assert.equal(isCapabilityProtocolUnsupported(chat, id), true, id);
  }
  for (const id of ['audio_input', 'video_input'] as const) {
    assert.equal(isCapabilityProtocolUnsupported(plan, id), true);
    assert.equal(isCapabilityProtocolUnsupported(chat, id), false);
  }
  assert.equal(isCapabilityProtocolUnsupported(plan, 'web_search'), false);
  assert.equal(isCapabilityProtocolUnsupported(chat, 'web_search'), true);
  assert.equal(isCapabilityProtocolUnsupported(account({ api_base_url: 'https://api.deepseek.com' }), 'web_search'), false);
  assert.equal(isCapabilityProtocolUnsupported(account({ auth_mode: 'apikey', api_wire_api: 'chat_completions', upstream_grok_account_id: 'g' }), 'web_search'), false);
  assert.equal(isCapabilityProtocolUnsupported(account({ auth_mode: 'apikey', api_wire_api: 'responses' }), 'web_search'), false);
  assert.equal(isCapabilityProtocolUnsupported(undefined, 'web_search'), false);
});
