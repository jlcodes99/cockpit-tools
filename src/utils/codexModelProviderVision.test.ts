import assert from "node:assert/strict";
import test from "node:test";
import {
  buildProviderModelVisionCapabilities,
  expandLegacyProviderVisionCapabilities,
  materializeProviderModelVisionCapabilities,
  providerModelDefaultsToVisionInput,
  resolveProviderModelVisionState,
} from "./codexModelProviderVision.ts";

test("vision switches honor mixed-case explicit values and provider defaults", () => {
  assert.equal(resolveProviderModelVisionState(" MiniMax-M3 ", { "minimax-m3": true }), true);
  assert.equal(resolveProviderModelVisionState("Qwen-VL", { "qwen-vl": false }, true), false);
  assert.equal(resolveProviderModelVisionState("Qwen-VL", {}, true), true);
  assert.equal(resolveProviderModelVisionState("GPT-6-LUNA", {}, false), true);
  assert.equal(resolveProviderModelVisionState("GPT-6-LUNA", { "gpt-6-luna": false }), false);
});

test("saving and reopening retains explicit disabled overrides for inherited vision", () => {
  const saved = buildProviderModelVisionCapabilities("Qwen-VL\nMiniMax-M3", {
    "qwen-vl": false,
    "minimax-m3": true,
    "Other-Model": false,
  });
  assert.deepEqual(saved, {
    "qwen-vl": { supportsVision: false },
    "minimax-m3": { supportsVision: true },
    "other-model": { supportsVision: false },
  });
  const states = Object.fromEntries(Object.entries(saved).map(([model, value]) => [model, value.supportsVision]));
  assert.equal(resolveProviderModelVisionState("Qwen-VL", states, true), false);
  assert.equal(resolveProviderModelVisionState("MiniMax-M3", states, false), true);
  assert.deepEqual(buildProviderModelVisionCapabilities("", {}), {});
});

test("expands legacy provider-level vision into per-model capabilities", () => {
  const provider = {
    baseUrl: "https://relay.example.com/v1",
    supportsVision: true,
    modelCatalog: ["openai/gpt-5.5", "openai/gpt-5.6-luna"],
    modelCapabilities: undefined as
      | Record<string, { supportsVision?: boolean }>
      | undefined,
  };

  assert.equal(expandLegacyProviderVisionCapabilities(provider), true);
  assert.deepEqual(provider.modelCapabilities, {
    "openai/gpt-5.5": { supportsVision: true },
    "openai/gpt-5.6-luna": { supportsVision: true },
  });
});

test("keeps explicit per-model decisions and only fills the gaps", () => {
  const provider = {
    baseUrl: "https://relay.example.com/v1",
    supportsVision: true,
    modelCatalog: ["gpt-5.5", "gpt-5.6-sol"],
    modelCapabilities: {
      "gpt-5.5": { supportsVision: false },
    },
  };

  assert.equal(expandLegacyProviderVisionCapabilities(provider), true);
  assert.deepEqual(provider.modelCapabilities, {
    "gpt-5.5": { supportsVision: false },
    "gpt-5.6-sol": { supportsVision: true },
  });
});

test("materializes every catalog model and removes stale capability entries", () => {
  const provider = {
    baseUrl: "https://relay.example.com/v1",
    supportsVision: false,
    modelCatalog: ["gpt-6-luna", "gpt-4o"],
    modelCapabilities: {
      "gpt-4o": { supportsVision: false },
      "stale-model": { supportsVision: true },
    },
  };

  assert.equal(materializeProviderModelVisionCapabilities(provider), true);
  assert.deepEqual(provider.modelCapabilities, {
    "gpt-6-luna": { supportsVision: true },
    "gpt-4o": { supportsVision: false },
  });
});

test("does not touch providers without the legacy flag or with empty catalog", () => {
  const withoutFlag = {
    baseUrl: "https://relay.example.com/v1",
    supportsVision: false,
    modelCatalog: ["gpt-5.5"],
    modelCapabilities: undefined as
      | Record<string, { supportsVision?: boolean }>
      | undefined,
  };
  assert.equal(expandLegacyProviderVisionCapabilities(withoutFlag), false);
  assert.equal(withoutFlag.modelCapabilities, undefined);

  const emptyCatalog = {
    baseUrl: "https://relay.example.com/v1",
    supportsVision: true,
    modelCatalog: [] as string[],
    modelCapabilities: undefined as
      | Record<string, { supportsVision?: boolean }>
      | undefined,
  };
  assert.equal(expandLegacyProviderVisionCapabilities(emptyCatalog), false);
});

test("skips the official DeepSeek provider", () => {
  const provider = {
    baseUrl: "https://api.deepseek.com",
    supportsVision: true,
    modelCatalog: ["deepseek-v4-pro"],
    modelCapabilities: undefined as
      | Record<string, { supportsVision?: boolean }>
      | undefined,
  };
  assert.equal(expandLegacyProviderVisionCapabilities(provider), false);
  assert.equal(provider.modelCapabilities, undefined);
});

// 与 Rust 侧 codex_account_tests_model_vision 使用同一组模型，防止两处规则漂移。
test("models with catalog image input declarations default to vision input", () => {
  for (const model of [
    "gpt-5.6-luna",
    "gpt-5.6-sol",
    "gpt-5.6-terra",
    "gpt-6.1-sol",
    "gpt-6-astra",
    "gpt-6-sol",
    "gpt-6-luna",
    "codex-auto-review",
    "claude-haiku-4-5",
    "claude-haiku-5-5",
    "claude-opus-5-5",
    "claude-sonnet-5-5",
    "gemini-3.8-flash",
    "gemini-4-argon",
    "grok-4.7",
    "openai/gpt-5.6-sol",
    "openai/gpt-6-astra",
    "openai/gpt-6-sol",
    " OPENAI/GPT-6-LUNA ",
  ]) {
    assert.equal(providerModelDefaultsToVisionInput(model), true, `${model} 在目录中声明图片输入，应默认识图`);
  }
});

test("models absent from the catalog or without image declarations default to false", () => {
  for (const model of [
    "gpt-5.5-pro",
    "gpt-6-luna-custom",
    "gpt-99.0",
    "gpt-5.4",
    "gpt-5.4-mini",
    "gpt-5.3-codex",
    "gpt-5",
    "gpt-4o",
    "gpt-3.5-turbo",
    "gpt-reserve",
    "gpt-image-2.5",
    "deepseek-v4-pro",
    "claude-sonnet-4-5",
    "gemini-2.5-pro",
    "glm-4.6",
  ]) {
    assert.equal(providerModelDefaultsToVisionInput(model), false, `${model} 未找到或未声明图片输入，应默认不支持`);
  }
});
