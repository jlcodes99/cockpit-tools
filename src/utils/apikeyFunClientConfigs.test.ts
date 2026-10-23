import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { APIKEY_FUN_FAMILIES, buildCodexConfigToml, resolveDefaultModel } from './apikeyFunClientConfigs';
import { APIKEY_FUN_DEFAULT_MODEL_CATALOG } from './apikeyFunLinks';

const family = (id: string) => {
  const preset = APIKEY_FUN_FAMILIES.find((entry) => entry.id === id);
  assert.ok(preset, `missing family ${id}`);
  return preset;
};

test('codex config derives the auto compact limit as 90% of the context window', () => {
  const catalog = JSON.parse(readFileSync(new URL('../../sidecars/cockpit-cliproxy/third_party/CLIProxyAPI/internal/registry/models/codex_client_models.json', import.meta.url), 'utf8'));
  const codexTemplate = catalog.models.find((model: { slug: string }) => model.slug === family('codex').fallbackModel);
  assert.ok(codexTemplate, 'Codex family model template');
  for (const [id, contextWindow] of [
    ['codex', codexTemplate.context_window],
    ['glm', 1000000],
    ['kimi', 1000000],
    ['deepseek', 1000000],
  ] as const) {
    const toml = buildCodexConfigToml({
      family: family(id),
      client: 'codex_cli',
      model: family(id).fallbackModel,
    });
    const expected = Math.floor((contextWindow * 90) / 100);
    assert.ok(toml.includes(`model_context_window = ${contextWindow}`), id);
    assert.ok(toml.includes(`model_auto_compact_token_limit = ${expected}`), id);
    // 压缩阈值必须严格小于上下文窗口，否则永远不会触发压缩。
    assert.ok(expected < contextWindow, id);
  }
});

test('built-in Codex templates prefer current models while preserving explicit upstream catalogs', () => {
  const preset = family('codex');
  assert.equal(resolveDefaultModel(preset, []), 'gpt-5.6-sol');
  assert.equal(resolveDefaultModel(preset, ['gpt-5.5', 'gpt-5.6-sol']), 'gpt-5.6-sol');
  assert.equal(resolveDefaultModel(preset, ['custom-gpt', 'gpt-5.5']), 'gpt-5.5');
  assert.equal(resolveDefaultModel(preset, ['gpt-5.4', 'gpt-5.5']), 'gpt-5.4');
  assert.ok(!preset.preferredModels.includes('gpt-5.5'));
  assert.ok(!APIKEY_FUN_DEFAULT_MODEL_CATALOG.some((model) => model === ('gpt-5.5' as string)));
  assert.ok(buildCodexConfigToml({ family: preset, client: 'codex_app', model: 'gpt-5.5' })
    .includes('model = "gpt-5.5"'));
});

test('Grok template prefers 4.7 only when the upstream catalog offers it', () => {
  const preset = family('grok');
  assert.equal(resolveDefaultModel(preset, []), 'grok-4.7');
  assert.equal(resolveDefaultModel(preset, ['grok-4.6', 'grok-4.7']), 'grok-4.7');
  assert.equal(resolveDefaultModel(preset, ['grok-4.6']), 'grok-4.6');
  assert.equal(resolveDefaultModel(preset, ['custom-grok']), 'custom-grok');
  assert.ok(!preset.preferredModels.includes('grok-4.7-build-fast'));
  assert.ok(buildCodexConfigToml({ family: preset, client: 'codex_cli', model: 'grok-4.6' })
    .includes('model = "grok-4.6"'));
});
