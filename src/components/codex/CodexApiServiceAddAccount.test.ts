import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../../tests/helpers/reactHookHarness';

test('the shared API Service add menu preserves account sources without a standalone official authorization entry', () => {
  const requests: unknown[] = [];
  const hook = loadHookModule(new URL('./CodexApiServiceAddAccount.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../SingleSelectDropdown': { SingleSelectDropdown: 'Dropdown' },
    '../../utils/codexAddAccountRequest': { requestCodexOpenAddAccount(value: unknown) { requests.push(value); } },
    '../../utils/chatgptPlanNavigation': { requestChatgptPlanAuthorization() { requests.push('plan'); } },
  });
  const tree = hook.render(() => hook.exports.CodexApiServiceAddAccount({ disabled: false }));
  assert.deepEqual(Array.from(tree.props.options, (option: any) => option.value), ['tempLogin', 'apikey', 'import']);
  tree.props.onChange('chatgpt_plan');
  for (const source of ['tempLogin', 'apikey', 'import']) tree.props.onChange(source);
  tree.props.onChange('unknown');
  assert.equal(JSON.stringify(requests), JSON.stringify([
    { tab: 'tempLogin', autoJoinApiService: true }, { tab: 'apikey', autoJoinApiService: true }, { tab: 'import', autoJoinApiService: true },
  ]));
  hook.unmount();
});
