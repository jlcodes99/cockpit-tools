import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { filterAntigravityModelOptions, getAntigravityModelDisplayName, getPreferredAntigravityWakeupModelId } from './antigravityModels.ts';
import { getDefaultGroups, getModelDisplayName } from './modelNames.ts';

test('new and old model versions remain separate selectable upstream IDs', () => {
  const ids = ['gemini-3-flash', 'gemini-3.8-flash-high', 'gemini-4-argon-medium', 'gemini-4-argon-high', 'claude-sonnet-4-6', 'claude-sonnet-5-5-medium'];
  const result = filterAntigravityModelOptions(ids.map((id) => ({ id, displayName: '' })));
  assert.deepEqual(result.map((model) => model.id), ids);
  assert.notEqual(getAntigravityModelDisplayName('gemini-4-argon-medium'), getAntigravityModelDisplayName('gemini-4-argon-high'));
  assert.match(getModelDisplayName('claude-opus-4-6-thinking'), /Thinking/);
  assert.match(getAntigravityModelDisplayName('Unknown (Custom)'), /\(Custom\)/);
});

test('new grouping defaults add Argon and retain legacy groups', () => {
  const groups = getDefaultGroups();
  assert.ok(groups.some((group) => group.id === 'g3_pro'));
  assert.ok(groups.find((group) => group.id === 'g4_argon')?.desktopModels.includes('gemini-4-argon-high'));
  assert.ok(groups.find((group) => group.id === 'claude_45')?.desktopModels.includes('claude-sonnet-5-5-high'));
  assert.ok(groups.find((group) => group.id === 'g3_flash')?.desktopModels.includes('gemini-3.8-flash-high'));
});

const officialModels = JSON.parse(readFileSync(
  new URL('../../tests/fixtures/antigravity-available-models.json', import.meta.url), 'utf8',
)) as {
  agentModelSorts: Array<{ groups: Array<{ modelIds: string[] }> }>;
  models: Record<string, { displayName?: string; model: string; recommended?: boolean }>;
};
const selectableIds = officialModels.agentModelSorts.flatMap((sort) =>
  sort.groups.flatMap((group) => group.modelIds));

test('all official model effort tiers have matching IDs, constants, names and quota groups', () => {
  const options = filterAntigravityModelOptions(selectableIds.map((id) => ({ id, displayName: '' })));
  assert.deepEqual(options.map((model) => model.id), selectableIds);
  for (const option of options) {
    const meta = officialModels.models[option.id];
    assert.equal(option.modelConstant, meta.model, option.id);
    assert.equal(option.displayName, meta.displayName, option.id);
    assert.equal(getAntigravityModelDisplayName(meta.model), meta.displayName);
    assert.equal(getAntigravityModelDisplayName(meta.model.replace(/^MODEL_/, '')), meta.displayName);
    assert.equal(getModelDisplayName(meta.model), meta.displayName);
    assert.equal(getModelDisplayName(option.id), meta.displayName);
    const group = getDefaultGroups().find((item) => item.desktopModels.includes(option.id));
    assert.ok(group?.pluginModels.includes(meta.model), option.id);
  }
});

test('wakeup follows the official picker and defaults to 3.8 Flash High regardless of list order', () => {
  const options = filterAntigravityModelOptions(Object.entries(officialModels.models).map(([id, meta]) => ({
    id, displayName: meta.displayName || '', modelConstant: meta.model,
    recommended: selectableIds.includes(id),
  })), { includeNonRecommended: false });
  assert.deepEqual(options.map((model) => model.id), selectableIds);
  assert.equal(getPreferredAntigravityWakeupModelId(options), 'gemini-3.8-flash-high');
  assert.equal(getPreferredAntigravityWakeupModelId([...options].reverse()), 'gemini-3.8-flash-high');
  assert.equal(getPreferredAntigravityWakeupModelId([{ id: 'gemini-3-flash', displayName: '' }]), 'gemini-3-flash');
  assert.equal(getPreferredAntigravityWakeupModelId([options[0]]), options[0].id);
  assert.equal(getPreferredAntigravityWakeupModelId([]), '');
});

test('recommended upstream IDs survive catalog aliases and future model additions', () => {
  const inputs = [
    { id: 'gemini-pro-agent', displayName: 'Gemini 3.1 Pro (High)', modelConstant: 'MODEL_PLACEHOLDER_M16', recommended: true },
    { id: 'gemini-next-flash-high', displayName: 'Gemini Next Flash (High)', modelConstant: 'MODEL_PLACEHOLDER_M499', recommended: true },
  ];
  assert.deepEqual(filterAntigravityModelOptions(inputs), inputs);
  assert.deepEqual(filterAntigravityModelOptions([
    { id: 'gemini:weekly', displayName: 'Gemini (Weekly)', recommended: true },
    { id: 'tab-internal', displayName: 'Internal' },
  ]), []);
});
