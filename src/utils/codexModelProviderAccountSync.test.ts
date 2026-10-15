import assert from "node:assert/strict";
import test from "node:test";

import type { CodexModelProvider } from "../services/codexModelProviderService.ts";
import type { CodexAccount } from "../types/codex.ts";
import { buildCodexModelProviderAccountSnapshot, findCodexAccountsReferencingModelProvider } from "./codexModelProviderAccountSync.ts";

test("Responses provider account snapshots retain vision defaults, overrides and routing", () => {
  const snapshot = buildCodexModelProviderAccountSnapshot({
    id: "cmp_custom",
    name: "Custom Responses",
    baseUrl: "https://relay.example.com/v1",
    wireApi: "responses",
    modelCatalog: ["Qwen-VL", "text-only"],
    supportsVision: true,
    modelCapabilities: { "qwen-vl": { supportsVision: true }, "text-only": { supportsVision: false } },
    visionRoutingModel: "Qwen-VL",
    supportsWebsockets: false,
    apiKeys: [],
    createdAt: 1,
    updatedAt: 1,
  });
  assert.equal(snapshot.apiSupportsVision, true);
  assert.deepEqual(snapshot.apiModelVisionSupport, { "qwen-vl": true, "text-only": false });
  assert.equal(snapshot.apiVisionRoutingModel, "Qwen-VL");
  assert.equal(snapshot.apiWireApi, "responses");
});

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

test('account snapshots resolve by credential and preserve another key and legacy defaults', () => {
  const provider: CodexModelProvider = {
    id: 'provider', name: 'Relay', baseUrl: 'https://relay.example/v1',
    modelCatalog: ['legacy'], modelContextWindows: { legacy: 128000 },
    supportsVision: true, modelCapabilities: { legacy: { supportsVision: true } },
    visionRoutingModel: 'legacy', supportsWebsockets: false,
    apiKeys: [
      { id: 'a', name: 'same-name', apiKey: 'key-a', createdAt: 1, updatedAt: 1,
        modelCatalog: ['a'], modelContextWindows: { a: 256000 },
        modelCapabilities: { a: { supportsVision: true } }, visionRoutingModel: 'a' },
      { id: 'b', name: 'same-name', apiKey: 'key-b', createdAt: 1, updatedAt: 1,
        modelCatalog: ['b'], modelContextWindows: {}, supportsVision: false,
        modelCapabilities: {}, visionRoutingModel: null },
      { id: 'legacy', name: '', apiKey: 'key-legacy', createdAt: 1, updatedAt: 1 },
    ], createdAt: 1, updatedAt: 1,
  };
  const a = buildCodexModelProviderAccountSnapshot(provider, 'same-name', ' key-a ');
  const b = buildCodexModelProviderAccountSnapshot(provider, 'same-name', 'key-b');
  const legacy = buildCodexModelProviderAccountSnapshot(provider, '', 'key-legacy');
  assert.deepEqual(a.apiModelCatalog, ['a']);
  assert.deepEqual(a.apiModelContextWindows, { a: 256000 });
  assert.deepEqual(b.apiModelCatalog, ['b']);
  assert.deepEqual(b.apiModelContextWindows, {});
  assert.deepEqual(b.apiModelVisionSupport, {});
  assert.equal(b.apiSupportsVision, false);
  assert.equal(b.apiVisionRoutingModel, undefined);
  assert.deepEqual(legacy.apiModelCatalog, ['legacy']);
  assert.deepEqual(provider.modelCatalog, ['legacy']);
});


test('key-owned catalogs cannot inherit metadata or vision routing for models outside their permissions', () => {
  const provider: CodexModelProvider = {
    id: 'provider', name: 'Relay', baseUrl: 'https://relay.example/v1',
    modelCatalog: ['allowed', 'other-key-model'],
    modelContextWindows: { allowed: 128000, 'other-key-model': 256000 },
    modelCapabilities: { allowed: { supportsVision: false }, 'other-key-model': { supportsVision: true } },
    visionRoutingModel: 'other-key-model', supportsWebsockets: false,
    apiKeys: [{ id: 'key', name: '', apiKey: 'restricted', modelCatalog: ['allowed'], createdAt: 1, updatedAt: 1 }],
    createdAt: 1, updatedAt: 1,
  };
  const snapshot = buildCodexModelProviderAccountSnapshot(provider, '', 'restricted');
  assert.deepEqual(snapshot.apiModelContextWindows, { allowed: 128000 });
  assert.deepEqual(snapshot.apiModelVisionSupport, { allowed: false });
  assert.equal(snapshot.apiVisionRoutingModel, undefined);
});
