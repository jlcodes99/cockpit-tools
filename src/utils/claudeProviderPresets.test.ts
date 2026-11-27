import assert from "node:assert/strict";
import test from "node:test";

import { findClaudeApiProviderPresetById, inferClaudeApiKeyField } from "./claudeProviderPresets.ts";
import { findClaudeDesktopGatewayProviderPresetById } from "./claudeDesktopProviderPresets.ts";

test("AIGROW Claude Code preset maps every role to an AIGROW model", () => {
  const preset = findClaudeApiProviderPresetById("aigrow");
  assert.ok(preset);
  assert.deepEqual(preset.baseUrls, ["https://aigrow.gg"]);
  assert.equal(inferClaudeApiKeyField(preset), "ANTHROPIC_AUTH_TOKEN");
  assert.equal(preset.extraEnv?.ANTHROPIC_MODEL, "claude-haiku-5.5");
  assert.deepEqual(preset.modelCatalog, ["claude-haiku-5.5", "claude-sonnet-5.5", "claude-opus-5.5"]);
});

test("AIGROW Claude Desktop preset maps the desktop roles locally", () => {
  const preset = findClaudeDesktopGatewayProviderPresetById("aigrow");
  assert.ok(preset);
  assert.equal(preset.authScheme, "bearer");
  assert.equal(preset.connectionMode, "local_mapping");
  assert.deepEqual(
    preset.modelMappings.map((mapping) => mapping.upstreamModel),
    ["claude-sonnet-5.5", "claude-opus-5.5", "claude-haiku-5.5"],
  );
});

test("LLMTR Claude Code preset maps every role to an LLMTR model", () => {
  const preset = findClaudeApiProviderPresetById("llmtr");
  assert.ok(preset);
  assert.deepEqual(preset.baseUrls, ["https://llmtr.com"]);
  assert.equal(inferClaudeApiKeyField(preset), "ANTHROPIC_AUTH_TOKEN");
  assert.equal(preset.extraEnv?.ANTHROPIC_MODEL, "anthropic/claude-sonnet-5.5");
  assert.deepEqual(preset.modelCatalog, [
    "anthropic/claude-sonnet-5.5",
    "anthropic/claude-haiku-5.5",
    "anthropic/claude-opus-5.5",
  ]);
});

test("LLMTR Claude Desktop preset maps the desktop roles locally", () => {
  const preset = findClaudeDesktopGatewayProviderPresetById("llmtr");
  assert.ok(preset);
  assert.equal(preset.authScheme, "bearer");
  assert.equal(preset.connectionMode, "local_mapping");
  assert.deepEqual(
    preset.modelMappings.map((mapping) => mapping.upstreamModel),
    ["anthropic/claude-sonnet-5.5", "anthropic/claude-opus-5.5", "anthropic/claude-haiku-5.5"],
  );
  assert.ok(preset.modelMappings.every((mapping) => mapping.supports1m === true));
});
