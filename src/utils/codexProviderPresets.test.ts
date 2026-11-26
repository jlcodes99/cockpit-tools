import assert from "node:assert/strict";
import test from "node:test";

import {
  DEEPSEEK_API_BASE_URL,
  MINIMAX_API_PROVIDER_ID,
  MINIMAX_EN_API_PROVIDER_ID,
  OLLAMA_LOCAL_API_PROVIDER_ID,
  OPENCODE_GO_API_BASE_URL,
  OPENCODE_GO_API_PROVIDER_ID,
  codexApiProviderPresetVisionSupport,
  findCodexApiProviderPresetByBaseUrl,
  findCodexApiProviderPresetById,
  resolveOllamaLocalPlaceholderKey,
} from "./codexProviderPresets.ts";
import { canConfigureCodexProviderVision, resolveCodexProviderCapabilityProfile } from "./codexProviderGateway.ts";
import { codexProviderNeedsProtocolGateway } from "./codexLocalAccessAccounts.ts";

test("local Ollama defaults to the chat bridge and supplies a key only for its loopback endpoints", () => {
  const baseUrls = findCodexApiProviderPresetById(OLLAMA_LOCAL_API_PROVIDER_ID)!.baseUrls;
  const placeholders = baseUrls.map((baseUrl) => {
    assert.equal(findCodexApiProviderPresetByBaseUrl(baseUrl)?.id, OLLAMA_LOCAL_API_PROVIDER_ID);
    assert.equal(resolveCodexProviderCapabilityProfile({ baseUrl }).gatewayStrategy, "responses_to_chat_completions");
    assert.equal(resolveCodexProviderCapabilityProfile({ baseUrl, wireApi: "responses" }).wireApi, "responses");
    const placeholder = resolveOllamaLocalPlaceholderKey(baseUrl);
    assert.ok(placeholder);
    assert.equal(resolveOllamaLocalPlaceholderKey(`${baseUrl}/`), placeholder);
    return placeholder;
  });
  assert.equal(new Set(placeholders).size, baseUrls.length);
  for (const baseUrl of ["https://ollama.com/v1", "http://192.168.1.2:11434/v1", "http://127.0.0.1:8000/v1"]) {
    assert.equal(resolveOllamaLocalPlaceholderKey(baseUrl), "");
  }
});

test("Ollama Cloud uses its hosted chat endpoint and requires a supplied key", () => {
  const baseUrl = "https://ollama.com/v1";
  const preset = findCodexApiProviderPresetByBaseUrl(`${baseUrl}/`);
  assert.equal(preset?.id, "ollama_cloud");
  assert.equal(preset?.apiKeyUrl, "https://ollama.com/settings/keys");
  assert.equal(resolveOllamaLocalPlaceholderKey(baseUrl), "");
  assert.equal(resolveCodexProviderCapabilityProfile({ baseUrl }).gatewayStrategy, "responses_to_chat_completions");
  assert.equal(resolveCodexProviderCapabilityProfile({ baseUrl, wireApi: "responses" }).wireApi, "responses");
});

test("Anthropic uses the instance gateway and preserves its protocol", () => {
  const profile = resolveCodexProviderCapabilityProfile({ baseUrl: "https://example.invalid", wireApi: "anthropic" });
  assert.equal(profile.wireApi, "anthropic");
  assert.equal(profile.requiresGateway, true);
  assert.equal(profile.supportsDirect, false);
  assert.equal(profile.gatewayStrategy, "responses_to_anthropic");
});

test("third-party Responses providers expose preserved vision settings", () => {
  for (const presetId of ["custom", "packycode", "openrouter"]) {
    assert.equal(canConfigureCodexProviderVision({ presetId, wireApi: "responses" }), true);
  }
  for (const presetId of ["openai_official", "deepseek"]) {
    assert.equal(canConfigureCodexProviderVision({ presetId, wireApi: "responses" }), false);
  }
  assert.equal(canConfigureCodexProviderVision({ presetId: "deepseek", wireApi: "chat_completions" }), true);
});

test("OpenRouter preset includes the current Luna Pro model id", () => {
  const preset = findCodexApiProviderPresetByBaseUrl(
    "https://openrouter.ai/api/v1/",
  );

  assert.ok(preset);
  assert.deepEqual(preset.modelCatalog, ["openai/gpt-5.6-luna-pro"]);
});

test("OpenCode Go preset exposes DeepSeek models and its chat-completions transport", () => {
  const preset = findCodexApiProviderPresetById(OPENCODE_GO_API_PROVIDER_ID);

  assert.ok(preset);
  assert.equal(preset.baseUrls[0], OPENCODE_GO_API_BASE_URL);
  assert.ok(preset.modelCatalog?.includes("deepseek-v4-pro"));
  assert.ok(preset.modelCatalog?.includes("deepseek-v4-flash"));
  assert.ok(preset.modelCatalog?.includes("deepseek-v4.1-flash"));
  assert.deepEqual(preset.visionModelCatalog, ["deepseek-v4.1-flash"]);
  assert.ok(preset.modelCatalog?.includes("qwen3.7-plus"));

  const profile = resolveCodexProviderCapabilityProfile({
    presetId: OPENCODE_GO_API_PROVIDER_ID,
    baseUrl: OPENCODE_GO_API_BASE_URL,
  });
  assert.equal(profile.wireApi, "chat_completions");
  assert.equal(profile.requiresGateway, true);
  assert.equal(resolveCodexProviderCapabilityProfile({
    baseUrl: "https://opencode.ai/zen/go/v1/",
  }).wireApi, "chat_completions");
});

test("Requesty preset uses its native Responses endpoint directly", () => {
  const preset = findCodexApiProviderPresetByBaseUrl("https://router.requesty.ai/v1/");
  assert.equal(preset?.id, "requesty");
  assert.ok(preset.modelCatalog?.includes("openai/gpt-6.1-terra"));
  const profile = resolveCodexProviderCapabilityProfile({ baseUrl: preset.baseUrls[0] });
  assert.equal(profile.wireApi, "responses");
  assert.equal(profile.supportsDirect, true);
  assert.equal(profile.requiresGateway, false);
});

test("Atlas Cloud preset uses its native Responses endpoint directly", () => {
  const preset = findCodexApiProviderPresetByBaseUrl("https://api.atlascloud.ai/v1/");
  assert.equal(preset?.id, "atlascloud");
  assert.ok(preset.modelCatalog?.includes("deepseek-ai/deepseek-v4-flash"));
  const profile = resolveCodexProviderCapabilityProfile({ baseUrl: preset.baseUrls[0] });
  assert.equal(profile.wireApi, "responses");
  assert.equal(profile.supportsDirect, true);
  assert.equal(profile.requiresGateway, false);
});

test("AIGROW preset uses its native Responses endpoint directly", () => {
  const preset = findCodexApiProviderPresetByBaseUrl("https://aigrow.gg/v1/");
  assert.equal(preset?.id, "aigrow");
  assert.equal(preset.modelCatalog?.[0], "gpt-6.2-sol");
  const profile = resolveCodexProviderCapabilityProfile({ baseUrl: preset.baseUrls[0] });
  assert.equal(profile.wireApi, "responses");
  assert.equal(profile.supportsDirect, true);
  assert.equal(profile.requiresGateway, false);
});

test("Lazu preset uses its native Responses endpoint directly", () => {
  const preset = findCodexApiProviderPresetByBaseUrl("https://api.lazu.ai/v1/");
  assert.equal(preset?.id, "lazu");
  assert.ok(preset.modelCatalog?.includes("gpt-6.2-astra"));
  const profile = resolveCodexProviderCapabilityProfile({ baseUrl: preset.baseUrls[0] });
  assert.equal(profile.wireApi, "responses");
  assert.equal(profile.supportsDirect, true);
  assert.equal(profile.requiresGateway, false);
});

test("WorldRouter preset uses its native Responses endpoint directly", () => {
  const preset = findCodexApiProviderPresetByBaseUrl("https://inference-api.worldrouter.ai/v1/");
  assert.equal(preset?.id, "worldrouter");
  assert.ok(preset.modelCatalog?.includes("gpt-5.4"));
  const profile = resolveCodexProviderCapabilityProfile({ baseUrl: preset.baseUrls[0] });
  assert.equal(profile.wireApi, "responses");
  assert.equal(profile.supportsDirect, true);
  assert.equal(profile.requiresGateway, false);
});

test("TeamoRouter preset uses its native Responses endpoint directly", () => {
  const preset = findCodexApiProviderPresetByBaseUrl("https://api.teamorouter.com/v1/");
  assert.equal(preset?.id, "teamorouter");
  assert.ok(preset.modelCatalog?.includes("gpt-6.2-astra"));
  const profile = resolveCodexProviderCapabilityProfile({ baseUrl: preset.baseUrls[0] });
  assert.equal(profile.wireApi, "responses");
  assert.equal(profile.supportsDirect, true);
  assert.equal(profile.requiresGateway, false);
});

test("TrustedRouter preset uses its native Responses endpoint directly", () => {
  const preset = findCodexApiProviderPresetByBaseUrl("https://api.trustedrouter.com/v1/");
  assert.equal(preset?.id, "trustedrouter");
  assert.ok(preset.modelCatalog?.includes("deepseek/deepseek-v4-flash"));
  const profile = resolveCodexProviderCapabilityProfile({ baseUrl: preset.baseUrls[0] });
  assert.equal(profile.wireApi, "responses");
  assert.equal(profile.supportsDirect, true);
  assert.equal(profile.requiresGateway, false);
});

test("DeepSeek keeps its native Responses default", () => {
  const profile = resolveCodexProviderCapabilityProfile({
    presetId: "deepseek",
    baseUrl: DEEPSEEK_API_BASE_URL,
  });

  assert.equal(profile.wireApi, "responses");
  assert.equal(resolveCodexProviderCapabilityProfile({ baseUrl: DEEPSEEK_API_BASE_URL, wireApi: "chat_completions" }).wireApi, "responses");
  assert.equal(resolveCodexProviderCapabilityProfile({ baseUrl: "https://relay.example/v1", presetId: "deepseek", wireApi: "chat_completions" }).wireApi, "chat_completions");
});

test("Ark and MiMo presets use models for their own official endpoints", () => {
  const cases = [
    ["https://ark.cn-beijing.volces.com/api/coding/v3", "ark-code-latest"],
    ["https://ark.cn-beijing.volces.com/api/v3", "doubao-seed-2-1-pro-260915"],
    ["https://api.xiaomimimo.com/v1", "mimo-v2.6-pro"],
    ["https://token-plan-cn.xiaomimimo.com/v1", "mimo-v2.6-pro"],
  ];
  for (const [baseUrl, model] of cases) {
    const preset = findCodexApiProviderPresetByBaseUrl(baseUrl);
    assert.equal(preset?.modelCatalog?.[0], model);
    assert.equal(
      preset?.modelCatalog?.includes("glm-5.3"),
      baseUrl === "https://ark.cn-beijing.volces.com/api/coding/v3",
    );
  }
});

test("both MiniMax presets declare image input for their vision-capable models", () => {
  [MINIMAX_API_PROVIDER_ID, MINIMAX_EN_API_PROVIDER_ID].forEach((presetId) => {
    const preset = findCodexApiProviderPresetById(presetId);

    assert.ok(preset);
    assert.deepEqual(preset.modelCatalog, ["MiniMax-M3", "MiniMax-M3.1-Flash", "MiniMax-M2.7"]);
    assert.deepEqual(preset.visionModelCatalog, ["MiniMax-M3", "MiniMax-M3.1-Flash"]);

    const support = codexApiProviderPresetVisionSupport(preset);
    assert.equal(support["minimax-m3"], true);
    assert.equal(support["minimax-m3.1-flash"], true);
    assert.equal(support["minimax-m2.7"], undefined);
  });
});

test("MiniMax API defaults to native Responses and honors an explicit chat protocol", () => {
  for (const [baseUrl, presetId] of [
    ["https://api.minimax.io/v1", MINIMAX_EN_API_PROVIDER_ID],
    ["https://api.minimaxi.com/v1", MINIMAX_API_PROVIDER_ID],
    ["https://api.minimax.cn/v1", MINIMAX_API_PROVIDER_ID],
  ]) {
    assert.equal(findCodexApiProviderPresetByBaseUrl(baseUrl)?.id, presetId);
    assert.equal(resolveCodexProviderCapabilityProfile({ baseUrl }).wireApi, "responses");
    assert.equal(codexProviderNeedsProtocolGateway({ api_base_url: baseUrl }), false);
    assert.equal(resolveCodexProviderCapabilityProfile({ baseUrl, wireApi: "chat_completions" }).requiresGateway, true);
    assert.equal(codexProviderNeedsProtocolGateway({ api_base_url: baseUrl, api_wire_api: "chat_completions" }), true);
  }
});

test("presets without a declared vision catalog report no image support", () => {
  const preset = findCodexApiProviderPresetById("openrouter");

  assert.ok(preset);
  assert.deepEqual(codexApiProviderPresetVisionSupport(preset), {});
  assert.deepEqual(codexApiProviderPresetVisionSupport(null), {});
});
