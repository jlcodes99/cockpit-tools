import assert from 'node:assert/strict';
import test from 'node:test';
import { isAinipyBaseUrl } from './ainipy.ts';
import { isCodexApiKeyUsageQueryEligible, shouldShowCodexApiKeyUsagePanel } from './codexDeepSeekAccess.ts';
import type { CodexAccount } from '../types/codex.ts';

test('Ainipy detection trusts only official HTTPS hosts', () => {
  for (const url of ['https://www.ainipy.com/api/desktop/v1', 'https://ainipy.com/', 'https://api.ainipy.com/v1']) {
    assert.equal(isAinipyBaseUrl(url), true);
  }
  for (const url of ['http://ainipy.com', 'https://ainipy.com.evil.test', 'https://ainipy.com:444', 'https://user@ainipy.com', '', 'not a URL']) {
    assert.equal(isAinipyBaseUrl(url), false);
  }
});

test('Ainipy cards support automatic usage and remain visible for either wire API', () => {
  for (const api_wire_api of ['responses', 'chat_completions'] as const) {
    const account = { auth_mode: 'apikey', api_base_url: 'https://www.ainipy.com/api/desktop/v1',
      openai_api_key: 'test-key', api_wire_api } as CodexAccount;
    assert.equal(isCodexApiKeyUsageQueryEligible(account), true);
    assert.equal(shouldShowCodexApiKeyUsagePanel(account, true), true);
    assert.equal(isCodexApiKeyUsageQueryEligible({ ...account, openai_api_key: '' }), false);
  }
});
