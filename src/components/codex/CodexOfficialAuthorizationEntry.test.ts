import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../../tests/helpers/reactHookHarness';

test('official authorization is a plain manual entry regardless of legacy reminder preferences', () => {
  let opened = 0;
  const hook = loadHookModule(new URL('./CodexOfficialAuthorizationEntry.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../../utils/codexAccountServiceNavigation': { requestCodexOfficialAuthorizationManagement: () => { opened++; } },
  }, { window: { localStorage: { getItem() { throw new Error('retired reminder storage must not be read'); } } } });
  const entry = hook.render(() => hook.exports.CodexOfficialAuthorizationEntry({}));
  assert.equal(entry.type, 'button');
  assert.equal(opened, 0);
  assert.equal(entry.props.children.length, 2, 'no NEW or automatic authorization reminder badge');
  entry.props.onClick();
  assert.equal(opened, 1);
  hook.unmount();
});
