import assert from "node:assert/strict";
import test from "node:test";
import {
  expandLegacyProviderVisionCapabilities,
  providerModelDefaultsToVisionInput,
} from "./codexModelProviderVision.ts";

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
test("gpt-5.5 and later default to vision input", () => {
  for (const model of [
    "gpt-5.5",
    "gpt-5.5-pro",
    "gpt-5.6-luna",
    "gpt-5.6-sol",
    "gpt-5.6-terra",
    "gpt-6-astra",
    "gpt-6-sol",
    "gpt-6-luna",
    "openai/gpt-5.6-sol",
    "openai/gpt-6-astra",
    "openai/gpt-6-sol",
  ]) {
    assert.equal(providerModelDefaultsToVisionInput(model), true, `${model} 应默认识图`);
  }
});

test("other models keep previous defaults", () => {
  for (const model of [
    "gpt-5.4",
    "gpt-5.4-mini",
    "gpt-5.3-codex",
    "gpt-5",
    "gpt-4o",
    "gpt-3.5-turbo",
    "gpt-reserve",
    "gpt-image-2.5",
    "codex-auto-review",
    "deepseek-v4-pro",
    "claude-sonnet-4-5",
    "gemini-2.5-pro",
    "glm-4.6",
  ]) {
    assert.equal(providerModelDefaultsToVisionInput(model), false, `${model} 不应被默认规则影响`);
  }
});
