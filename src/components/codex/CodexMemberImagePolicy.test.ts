import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../../tests/helpers/reactHookHarness';
import { resolveCodexMemberImagePolicy } from '../../utils/codexLocalAccessAccounts';
import { discoverImageModels, eligibleImageAccountIds, selectedImageAccountModels } from '../../utils/codexImageAccounts';
import type { CodexAccount } from '../../types/codex';

const plan = { id: 'chatgpt_plan_test', auth_mode: 'apikey', api_provider_id: 'cockpit_chatgpt_plan', api_model_catalog: ['gpt-image-2'] } as CodexAccount;
const api = { ...plan, id: 'normal-api', api_provider_id: 'custom' };

test('plan image policy is disabled even for previously enabled config; ordinary accounts retain their policy', () => {
  for (const value of [undefined, 'enabled', 'inherit', 'disabled'] as const) {
    assert.equal(resolveCodexMemberImagePolicy(plan, value), 'disabled');
  }
  assert.equal(resolveCodexMemberImagePolicy(api, undefined), 'disabled');
  assert.equal(resolveCodexMemberImagePolicy(api, 'enabled'), 'enabled');
  assert.equal(resolveCodexMemberImagePolicy({ ...api, auth_mode: 'oauth' }, undefined), 'inherit');
});

test('plan image dropdown is disabled and cannot publish changes; a normal provider remains editable', () => {
  const changes: string[] = [];
  const hook = loadHookModule(new URL('./CodexMemberImagePolicy.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../SingleSelectDropdown': { SingleSelectDropdown: 'Dropdown' },
  });
  let account = plan;
  const render = () => hook.exports.CodexMemberImagePolicy({ account, value: 'enabled', disabled: false, onChange: (next: string) => changes.push(next) });
  const dropdown = hook.render(render).props.children;
  assert.equal(dropdown.props.disabled, true);
  assert.equal(dropdown.props.value, 'disabled');
  assert.equal(dropdown.props.options[0].label, 'chatgptPlan.imagePolicyLocked');
  dropdown.props.onChange('enabled');
  assert.deepEqual(changes, []);
  account = api;
  const normal = hook.flush().props.children;
  assert.equal(normal.props.disabled, false);
  assert.equal(normal.props.value, 'enabled');
  normal.props.onChange('inherit');
  assert.deepEqual(changes, ['inherit']);
  hook.unmount();
});

test('a plan catalog cannot make it an image source or populate the image catalog', () => {
  assert.deepEqual(discoverImageModels([plan]), []);
  assert.deepEqual(eligibleImageAccountIds([plan.id, api.id], [plan, api], 'gpt-image-2'), [api.id]);
  assert.deepEqual(selectedImageAccountModels([plan.id, api.id], [plan, api], 'gpt-image-2'), { [api.id]: 'gpt-image-2' });
});

test('official mode locks images on an original account without overwriting its local preference', () => {
  const source = { ...api, id: 'original', auth_mode: 'oauth', api_provider_id: undefined } as CodexAccount;
  assert.equal(resolveCodexMemberImagePolicy(source, 'enabled', true), 'disabled');
  assert.equal(resolveCodexMemberImagePolicy(source, 'enabled', false), 'enabled');
  const changes: string[] = [];
  const hook = loadHookModule(new URL('./CodexMemberImagePolicy.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../SingleSelectDropdown': { SingleSelectDropdown: 'Dropdown' },
  });
  let official = true;
  const render = () => hook.exports.CodexMemberImagePolicy({ account: source, official, value: 'enabled', disabled: false,
    onChange: (value: string) => changes.push(value) });
  const dropdown = hook.render(render).props.children;
  assert.equal(dropdown.props.disabled, true);
  assert.equal(dropdown.props.value, 'disabled');
  dropdown.props.onChange('enabled');
  assert.deepEqual(changes, []);
  official = false;
  const local = hook.flush().props.children;
  assert.equal(local.props.disabled, false);
  assert.equal(local.props.value, 'enabled');
  hook.unmount();
});
