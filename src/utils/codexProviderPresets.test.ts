import assert from "node:assert/strict";
import test from "node:test";

import {
  DEEPSEEK_API_BASE_URL,
  MINIMAX_API_PROVIDER_ID,
  MINIMAX_EN_API_PROVIDER_ID,
  OPENCODE_GO_API_BASE_URL,
  OPENCODE_GO_API_PROVIDER_ID,
  codexApiProviderPresetVisionSupport,
  findCodexApiProviderPresetByBaseUrl,
  findCodexApiProviderPresetById,
} from "./codexProviderPresets.ts";
import { canConfigureCodexProviderVision, resolveCodexProviderCapabilityProfile } from "./codexProviderGateway.ts";

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
  assert.ok(preset.modelCatalog?.includes("qwen3.7-plus"));

  const profile = resolveCodexProviderCapabilityProfile({
    presetId: OPENCODE_GO_API_PROVIDER_ID,
    baseUrl: OPENCODE_GO_API_BASE_URL,
  });
  assert.equal(profile.wireApi, "chat_completions");
  assert.equal(profile.requiresGateway, true);
});

test("DeepSeek keeps its native Responses default", () => {
  const profile = resolveCodexProviderCapabilityProfile({
    presetId: "deepseek",
    baseUrl: DEEPSEEK_API_BASE_URL,
  });

  assert.equal(profile.wireApi, "responses");
});

test("both MiniMax presets declare image input only for the vision-capable model", () => {
  [MINIMAX_API_PROVIDER_ID, MINIMAX_EN_API_PROVIDER_ID].forEach((presetId) => {
    const preset = findCodexApiProviderPresetById(presetId);

    assert.ok(preset);
    assert.deepEqual(preset.modelCatalog, ["MiniMax-M3", "MiniMax-M2.7"]);
    assert.deepEqual(preset.visionModelCatalog, ["MiniMax-M3"]);

    const support = codexApiProviderPresetVisionSupport(preset);
    assert.equal(support["minimax-m3"], true);
    assert.equal(support["minimax-m2.7"], undefined);
  });
});

test("presets without a declared vision catalog report no image support", () => {
  const preset = findCodexApiProviderPresetById(OPENCODE_GO_API_PROVIDER_ID);

  assert.ok(preset);
  assert.deepEqual(codexApiProviderPresetVisionSupport(preset), {});
  assert.deepEqual(codexApiProviderPresetVisionSupport(null), {});
});
