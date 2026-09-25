import assert from "node:assert/strict";
import test from "node:test";

import type { CodexAccount } from "../types/codex.ts";
import { findCodexAccountsReferencingModelProvider } from "./codexModelProviderAccountSync.ts";

function account(overrides: Partial<CodexAccount>): CodexAccount {
  return {
    id: "account-1",
    email: "api@example.com",
    auth_mode: "apikey",
    openai_api_key: "sk-test",
    tokens: { access_token: "", id_token: "" },
    created_at: 1,
    last_used: 1,
    ...overrides,
  };
}

test("finds existing API Key accounts by provider id or normalized base URL", () => {
  const result = findCodexAccountsReferencingModelProvider(
    { id: "provider-1", baseUrl: "https://relay.example.com/v1/" },
    [
      account({ id: "by-id", api_provider_id: "provider-1" }),
      account({
        id: "by-url",
        api_provider_id: "preset-id",
        api_base_url: "https://relay.example.com/v1",
      }),
      account({ id: "other", api_base_url: "https://other.example.com/v1" }),
    ],
  );

  assert.deepEqual(result, ["by-id", "by-url"]);
});

test("excludes OAuth accounts and API Key accounts without a readable key", () => {
  const result = findCodexAccountsReferencingModelProvider(
    { id: "provider-1", baseUrl: "https://relay.example.com/v1" },
    [
      account({ id: "oauth", auth_mode: "oauth", api_provider_id: "provider-1" }),
      account({ id: "missing-key", openai_api_key: "", api_provider_id: "provider-1" }),
    ],
  );

  assert.deepEqual(result, []);
});

test("a shared endpoint does not link an unregistered credential", () => {
  const result = findCodexAccountsReferencingModelProvider(
    { id: "provider-1", baseUrl: "https://relay.example.com/v1",
      apiKeys: [{ apiKey: "key-a" }, { apiKey: "key-b" }] },
    [
      account({ id: "a", openai_api_key: "key-a", api_base_url: "https://relay.example.com/v1" }),
      account({ id: "b", openai_api_key: "key-b", api_base_url: "https://relay.example.com/v1" }),
      account({ id: "unregistered", openai_api_key: "key-c", api_base_url: "https://relay.example.com/v1" }),
    ],
  );
  assert.deepEqual(result, ["a", "b"]);
});
