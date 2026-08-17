import assert from "node:assert/strict";
import test from "node:test";

import {
  DEEPSEEK_API_BASE_URL,
  OPENCODE_GO_API_BASE_URL,
  OPENCODE_GO_API_PROVIDER_ID,
  findCodexApiProviderPresetByBaseUrl,
  findCodexApiProviderPresetById,
} from "./codexProviderPresets.ts";
import { resolveCodexProviderCapabilityProfile } from "./codexProviderGateway.ts";

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
