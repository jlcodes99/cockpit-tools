import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { APIKEY_FUN_FAMILIES, APIKEY_FUN_GOOGLE_GEMINI_BASE_URL, buildApiKeyFunClientPreview, buildCodexConfigToml, buildGeminiCliEnv, buildGeminiCliSettingsJson, resolveApiKeyFunGeminiModel, resolveApiKeyFunGrokModel, resolveDefaultModel } from './apikeyFunClientConfigs';
import { APIKEY_FUN_DEFAULT_MODEL_CATALOG } from './apikeyFunLinks';

const family = (id: string) => {
  const preset = APIKEY_FUN_FAMILIES.find((entry) => entry.id === id);
  assert.ok(preset, `missing family ${id}`);
  return preset;
};

test('codex config derives the auto compact limit as 90% of the context window', () => {
  const catalog = JSON.parse(readFileSync(new URL('../../resources/codex/codex_client_models.json', import.meta.url), 'utf8'));
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

test('Grok account prefill selects a model actually available to the saved key', () => {
  assert.equal(resolveApiKeyFunGrokModel(['gpt-5', 'grok-4.6', 'grok-4.7']), 'grok-4.7');
  assert.equal(resolveApiKeyFunGrokModel(['gpt-5', 'relay/Grok-4.6']), 'relay/Grok-4.6');
  assert.equal(resolveApiKeyFunGrokModel(['gemini-3', 'gpt-5']), undefined);
});

test('Gemini relay key only exposes the Gemini CLI client, not Codex / Claude Code', () => {
  const gemini = family('gemini');
  // Issue #3104: 同一 baseURL 无法推断密钥支持 Responses / Messages 协议，
  // 因此不再把 Gemini 分组开放给 Codex / Claude Code，只保留官方文档的 Gemini CLI。
  assert.deepEqual(gemini.clients, ['gemini_cli']);

  const preview = buildApiKeyFunClientPreview({
    family: gemini,
    client: 'gemini_cli',
    apiKey: 'sk-test',
    model: 'gemini-3-pro-preview',
    platform: 'macos',
  });
  assert.equal(preview.applyKind, 'gemini_cli');
  const envBlock = preview.blocks.find((block) => block.label.includes('.env'));
  assert.ok(envBlock, 'Gemini CLI preview must include the .env block');
  assert.ok(envBlock.text.includes(`GOOGLE_GEMINI_BASE_URL="${APIKEY_FUN_GOOGLE_GEMINI_BASE_URL}"`));
  assert.ok(envBlock.text.includes('GEMINI_API_KEY="sk-test"'));
  assert.ok(envBlock.text.includes('GEMINI_MODEL="gemini-3-pro-preview"'));
  const settingsBlock = preview.blocks.find((block) => block.label.includes('settings.json'));
  assert.ok(settingsBlock, 'Gemini CLI preview must include the settings.json block');
  assert.ok(settingsBlock.text.includes('"selectedType": "gemini-api-key"'));
});

test('Gemini CLI env and settings match the values actually written to disk', () => {
  const env = buildGeminiCliEnv({ apiKey: 'sk-relay', model: 'gemini-3-pro-preview' });
  assert.equal(
    env,
    `GOOGLE_GEMINI_BASE_URL="${APIKEY_FUN_GOOGLE_GEMINI_BASE_URL}"\nGEMINI_API_KEY="sk-relay"\nGEMINI_MODEL="gemini-3-pro-preview"`,
  );
  // 与 Rust 侧 build_env 输出一致；settings.json 只切认证方式，不携带明文密钥。
  const settings = JSON.parse(buildGeminiCliSettingsJson());
  assert.equal(settings.security.auth.selectedType, 'gemini-api-key');
  assert.ok(!buildGeminiCliSettingsJson().includes('sk-relay'));
});

test('Gemini account prefill selects a model actually available to the saved key', () => {
  assert.equal(resolveApiKeyFunGeminiModel(['gpt-5', 'gemini-3-pro-preview']), 'gemini-3-pro-preview');
  assert.equal(resolveApiKeyFunGeminiModel(['gpt-5', 'relay/gemini-3-flash']), 'relay/gemini-3-flash');
  assert.equal(resolveApiKeyFunGeminiModel(['gpt-5', 'grok-4.7']), undefined);
});
