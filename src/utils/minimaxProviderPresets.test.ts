import assert from 'node:assert/strict';
import test from 'node:test';
import { findClaudeApiProviderPresetById } from './claudeProviderPresets.ts';
import { findClaudeDesktopGatewayProviderPresetById } from './claudeDesktopProviderPresets.ts';
import { findCodexApiProviderPresetById } from './codexProviderPresets.ts';

const models = ['MiniMax-M3.1-Flash', 'MiniMax-M3'];

test('MiniMax Claude presets default to Flash in both regions and expose M3', () => {
  for (const id of ['minimax', 'minimax_global']) {
    const preset = findClaudeApiProviderPresetById(id)!;
    assert.deepEqual(preset.modelCatalog, models);
    for (const key of ['ANTHROPIC_MODEL', 'ANTHROPIC_DEFAULT_SONNET_MODEL', 'ANTHROPIC_DEFAULT_OPUS_MODEL', 'ANTHROPIC_DEFAULT_HAIKU_MODEL']) {
      assert.equal(preset.extraEnv?.[key], models[0]);
    }
    assert.equal(preset.baseUrls[0], id === 'minimax' ? 'https://api.minimaxi.com/anthropic' : 'https://api.minimax.io/anthropic');
    const desktop = findClaudeDesktopGatewayProviderPresetById(id)!;
    assert.deepEqual(desktop.modelMappings.map(route => route.upstreamModel), models);
    assert.ok(desktop.modelMappings.every(route => route.supports1m));
    assert.equal(desktop.baseUrls[0], preset.baseUrls[0]);
  }
});

test('OpenCode Go uses the verified free Flash route and retains non-MiniMax entries', () => {
  const models = findCodexApiProviderPresetById('opencode_go')!.modelCatalog!;
  assert.deepEqual(models.filter(model => model.startsWith('minimax-')), ['minimax-m3.1-flash-free', 'minimax-m3']);
  assert.ok(models.includes('deepseek-v4-pro'));
  assert.ok(models.includes('qwen3.7-plus'));
});

test('SiliconFlow requires an account-supported model instead of guessing a new route', () => {
  for (const id of ['siliconflow', 'siliconflow_global']) {
    assert.equal(findClaudeApiProviderPresetById(id)!.extraEnv?.ANTHROPIC_MODEL, undefined);
    assert.deepEqual(findClaudeDesktopGatewayProviderPresetById(id)!.modelMappings, []);
  }
});
