import test from "node:test";
import assert from "node:assert/strict";
import { resolveCodexAccountDisplayName, resolveCodexAccountReferenceDisplayName, resolveCodexHealthIssueDisplayName } from "./codexAccountDisplayName.ts";
import { buildCodexAccountPresentation } from '../presentation/platformAccountPresentation';
import type { CodexAccount } from '../types/codex';

const apiAccount: CodexAccount = {
  id: 'codex_apikey_d5008680', email: 'api-key-d5008680', auth_mode: 'apikey',
  api_base_url: 'https://images.example.com/v1', api_provider_name: 'Image Provider',
  tokens: { access_token: '', id_token: '' }, created_at: 0, last_used: 0,
};
const fallback = 'API Key 账号';

test('account cards and shared labels use the custom name, then Base URL host without changing identities', () => {
  const t = () => fallback;
  const original = structuredClone(apiAccount);
  assert.equal(resolveCodexAccountDisplayName(apiAccount, fallback), 'images.example.com');
  assert.equal(buildCodexAccountPresentation(apiAccount, t).displayName, 'images.example.com');
  const named = { ...apiAccount, account_name: '  Production Images  ' };
  assert.equal(resolveCodexAccountDisplayName(named, fallback), 'Production Images');
  assert.equal(buildCodexAccountPresentation(named, t).displayName, 'Production Images');
  assert.deepEqual(apiAccount, original);
});

test('address labels omit credentials, paths and query strings and retain local ports', () => {
  assert.equal(resolveCodexAccountDisplayName({ ...apiAccount,
    api_base_url: 'https://user:secret@images.example.com/private?api_key=secret#token',
  }, fallback), 'images.example.com');
  assert.equal(resolveCodexAccountDisplayName({ ...apiAccount,
    api_base_url: 'http://127.0.0.1:8080/v1',
  }, fallback), '127.0.0.1:8080');
});

test('missing or invalid addresses use provider, official address or localized fallback', () => {
  for (const api_base_url of ['', undefined, 'invalid', 'file:///private/secret']) {
    assert.equal(resolveCodexAccountDisplayName({ ...apiAccount, api_base_url }, fallback), 'Image Provider');
    assert.equal(resolveCodexAccountDisplayName({ ...apiAccount, api_base_url, api_provider_name: '' }, fallback), fallback);
  }
  assert.equal(resolveCodexAccountDisplayName({ ...apiAccount,
    api_base_url: '', api_provider_mode: 'openai_builtin',
  }, fallback), 'api.openai.com');
});

test('legacy summaries and generated saved names do not expose a hash label', () => {
  assert.equal(resolveCodexAccountDisplayName({ id: 'recycled', email: apiAccount.email }, fallback), fallback);
  assert.equal(resolveCodexAccountDisplayName({ id: apiAccount.email }, fallback), fallback);
  assert.equal(resolveCodexAccountDisplayName({ ...apiAccount, account_name: apiAccount.email }, fallback), 'images.example.com');
  assert.equal(resolveCodexHealthIssueDisplayName('', apiAccount.email, '', apiAccount.id, apiAccount, fallback), 'images.example.com');
});

test('historical requests resolve current names by ID or old email and handle deleted accounts', () => {
  const names = new Map([[apiAccount.id, 'Production Images'], [apiAccount.email, 'images.example.com']]);
  assert.equal(resolveCodexAccountReferenceDisplayName(apiAccount.id, apiAccount.email, names, fallback), 'Production Images');
  assert.equal(resolveCodexAccountReferenceDisplayName('old-id', apiAccount.email, names, fallback), 'images.example.com');
  assert.equal(resolveCodexAccountReferenceDisplayName(apiAccount.id, apiAccount.email, new Map(), fallback), fallback);
  assert.equal(resolveCodexAccountReferenceDisplayName('oauth-id', 'person@example.com', new Map(), fallback), 'person@example.com');
});

test('ordinary OAuth card labels keep their existing email', () => {
  const oauth = { ...apiAccount, id: 'oauth', auth_mode: 'oauth', email: 'person@example.com', account_name: 'Note' };
  assert.equal(buildCodexAccountPresentation(oauth, () => fallback).displayName, 'person@example.com');
});

test("prefers a manually assigned API Key account name over the generated email", () => {
  assert.equal(
    resolveCodexHealthIssueDisplayName(
      "Production Key",
      "api-key-d5008680",
      "api-key-d5008680",
      "codex-account-id",
    ),
    "Production Key",
  );
});

test("generated historical names fall back to an understandable account label", () => {
  assert.equal(
    resolveCodexHealthIssueDisplayName(
      "",
      "api-key-d5008680",
      "health@example.com",
      "codex-account-id",
    ),
    "API",
  );
});

test("falls back to health email and then local account ID", () => {
  assert.equal(
    resolveCodexHealthIssueDisplayName(undefined, undefined, "health@example.com", "account-id"),
    "health@example.com",
  );
  assert.equal(
    resolveCodexHealthIssueDisplayName(undefined, undefined, undefined, "account-id"),
    "account-id",
  );
});
