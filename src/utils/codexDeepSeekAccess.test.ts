import assert from "node:assert/strict";
import test from "node:test";
import { CODEX_PROVIDER_GATEWAY_BIND_PREFIX } from "../types/instance.ts";
import {
  isDeepSeekAccount,
  isDeepSeekCdpAccess,
  isDeepSeekDirectAccess,
  isCodexApiKeyUsageQueryEligible,
  parseCodexBoundAccountId,
  resolveDeepSeekAccessMode,
  resolveDeepSeekBindAccountId,
  resolveDeepSeekStartupModel,
  shouldShowCodexApiKeyUsagePanel,
  shouldUseDeepSeekProviderGateway,
} from "./codexDeepSeekAccess.ts";
import type { CodexAccount } from "../types/codex.ts";

function account(partial: Partial<CodexAccount>): CodexAccount {
  return {
    id: partial.id || "acc-1",
    email: partial.email || "a@example.com",
    tokens: partial.tokens || {
      id_token: "",
      access_token: "",
      refresh_token: "",
    },
    created_at: partial.created_at || Date.now(),
    ...partial,
  } as CodexAccount;
}

test("detects DeepSeek by provider or official host", () => {
  assert.equal(
    isDeepSeekAccount({
      api_provider_id: "deepseek",
      api_base_url: "https://example.com",
    }),
    true,
  );
  assert.equal(
    isDeepSeekAccount({
      api_provider_id: "custom",
      api_base_url: "https://api.deepseek.com",
    }),
    true,
  );
  assert.equal(
    isDeepSeekAccount({
      api_provider_id: "openai",
      api_base_url: "https://api.openai.com/v1",
    }),
    false,
  );
});

test("defaults Responses to gateway and Chat to gateway", () => {
  assert.equal(
    resolveDeepSeekAccessMode({
      api_provider_id: "deepseek",
      api_base_url: "https://api.deepseek.com",
      api_wire_api: "responses",
    }),
    "gateway",
  );
  assert.equal(
    resolveDeepSeekAccessMode({
      api_provider_id: "deepseek",
      api_base_url: "https://api.deepseek.com",
      api_wire_api: "chat_completions",
      api_instance_access_mode: "direct",
    }),
    "gateway",
  );
  assert.equal(
    isDeepSeekDirectAccess({
      api_provider_id: "deepseek",
      api_base_url: "https://api.deepseek.com",
      api_wire_api: "responses",
      api_instance_access_mode: "direct",
    }),
    true,
  );
});

test("binds gateway prefix except official-direct Responses", () => {
  const gatewayAccount = {
    id: "acc-1",
    api_provider_id: "deepseek",
    api_base_url: "https://api.deepseek.com",
    api_wire_api: "responses" as const,
    api_instance_access_mode: "gateway",
  };
  assert.equal(shouldUseDeepSeekProviderGateway(gatewayAccount), true);
  assert.equal(
    resolveDeepSeekBindAccountId(gatewayAccount),
    `${CODEX_PROVIDER_GATEWAY_BIND_PREFIX}acc-1`,
  );

  const directAccount = {
    ...gatewayAccount,
    api_instance_access_mode: "direct",
  };
  assert.equal(shouldUseDeepSeekProviderGateway(directAccount), false);
  assert.equal(resolveDeepSeekBindAccountId(directAccount), "acc-1");

  const cdpAccount = {
    ...gatewayAccount,
    api_instance_access_mode: "cdp",
  };
  assert.equal(resolveDeepSeekAccessMode(cdpAccount), "cdp");
  assert.equal(isDeepSeekCdpAccess(cdpAccount), true);
  assert.equal(shouldUseDeepSeekProviderGateway(cdpAccount), false);
  assert.equal(resolveDeepSeekBindAccountId(cdpAccount), "acc-1");
});

test("keeps last official startup model or falls back to Flash", () => {
  assert.equal(
    resolveDeepSeekStartupModel({ api_startup_model: "deepseek-v4-pro" }),
    "deepseek-v4-pro",
  );
  assert.equal(
    resolveDeepSeekStartupModel({ api_startup_model: "gpt-5.5" }),
    "deepseek-v4-flash",
  );
  assert.equal(
    parseCodexBoundAccountId(`${CODEX_PROVIDER_GATEWAY_BIND_PREFIX}acc-2`),
    "acc-2",
  );
});

test("DeepSeek Chat Completions accounts can query usage", () => {
  assert.equal(
    isCodexApiKeyUsageQueryEligible(account({
      auth_mode: "apikey",
      api_provider_id: "deepseek",
      api_base_url: "https://api.deepseek.com",
      api_wire_api: "chat_completions",
      openai_api_key: "sk-deepseek-test",
    })),
    true,
  );
});

test("DeepSeek Responses accounts can query usage", () => {
  assert.equal(
    isCodexApiKeyUsageQueryEligible(account({
      auth_mode: "apikey",
      api_provider_id: "deepseek",
      api_base_url: "https://api.deepseek.com",
      api_wire_api: "responses",
      openai_api_key: "sk-deepseek-test",
    })),
    true,
  );
});

test("ordinary Chat Completions accounts stay excluded from usage query", () => {
  assert.equal(
    isCodexApiKeyUsageQueryEligible(account({
      auth_mode: "apikey",
      api_provider_id: "custom",
      api_base_url: "https://api.example.com/v1",
      api_wire_api: "chat_completions",
      openai_api_key: "sk-example",
    })),
    false,
  );
});

test("non API Key accounts stay excluded", () => {
  assert.equal(
    isCodexApiKeyUsageQueryEligible(account({
      auth_mode: "oauth",
      openai_api_key: "",
    })),
    false,
  );
});

test("accounts without a saved API key stay excluded", () => {
  assert.equal(
    isCodexApiKeyUsageQueryEligible(account({
      auth_mode: "apikey",
      api_provider_id: "deepseek",
      api_base_url: "https://api.deepseek.com",
      api_wire_api: "responses",
      openai_api_key: "",
    })),
    false,
  );
});

test("New API accounts stay excluded", () => {
  assert.equal(
    isCodexApiKeyUsageQueryEligible(account({
      auth_mode: "apikey",
      api_provider_id: "cockpit_api",
      api_base_url: "https://api.example.com",
      openai_api_key: "sk-new-api",
    })),
    false,
  );
});

test("account overview still shows DeepSeek quota when relay quota is hidden", () => {
  const deepseek = account({
    auth_mode: "apikey",
    api_provider_id: "deepseek",
    api_base_url: "https://api.deepseek.com",
    api_wire_api: "responses",
    openai_api_key: "sk-deepseek-test",
  });
  const relay = account({
    auth_mode: "apikey",
    api_provider_id: "custom",
    api_base_url: "https://api.example.com/v1",
    openai_api_key: "sk-relay",
  });
  assert.equal(shouldShowCodexApiKeyUsagePanel(deepseek, true), true);
  assert.equal(shouldShowCodexApiKeyUsagePanel(deepseek, false), true);
  assert.equal(shouldShowCodexApiKeyUsagePanel(relay, true), false);
  assert.equal(shouldShowCodexApiKeyUsagePanel(relay, false), true);
});
