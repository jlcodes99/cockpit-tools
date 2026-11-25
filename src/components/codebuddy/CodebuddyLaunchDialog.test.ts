import assert from 'node:assert/strict';
import test from 'node:test';
import { Children, isValidElement, type ReactNode } from 'react';
import { loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';

function find(node: ReactNode, type: unknown, testProps: (props: any) => boolean = () => true): any {
  if (!isValidElement<{ children?: ReactNode }>(node)) return;
  if (node.type === type && testProps(node.props)) return node.props;
  return Children.toArray(node.props.children).map((child) => find(child, type, testProps)).find(Boolean);
}

test('CodeBuddy CLI retries the pending copy in the preview after the path is saved', async () => {
  const calls: Array<[unknown, string]> = [];
  const clipboard: string[] = [];
  const fields = () => null;
  const pathField = () => null;
  const mock = loadHookModule(new URL('./CodebuddyLaunchDialog.tsx', import.meta.url), {
    'react-dom': { createPortal: (view: ReactNode) => view },
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '@tauri-apps/plugin-clipboard-manager': { writeText: async (command: string) => { clipboard.push(command); } },
    '../../hooks/useEscClose': { useEscCloseTopmost: () => {} },
    '../../hooks/useModalFocusTrap': { useModalFocusTrap: () => {} },
    '../../hooks/useLaunchTerminalOptions': {
      useLaunchTerminalOptions: () => ({ terminalOptions: [], selectedTerminal: 'system' }),
    },
    '../../hooks/useDurableUiSetting': { useDurableUiSetting: () => undefined },
    '../../utils/durableUiSettings': {},
    '../../utils/windowsOperationDialog': {},
    '../../services/codebuddyCliService': {
      getCodebuddyCliWorkingDir: async () => '/saved/project',
      getCodebuddyCliLaunchCommand: async (target: unknown, command: string) => {
        calls.push([target, command]);
        if (calls.length === 1) throw 'cliMissing';
        return 'resolved-launch-command';
      },
    },
    '../CliLaunchFields': { CliLaunchFields: fields },
    '../PlatformLaunchPathField': { PlatformLaunchPathField: pathField },
    '../icons/CodebuddyIcon': { CodebuddyIcon: () => null },
    '../ModalErrorMessage': { ModalErrorMessage: () => null },
  }, { document: { body: {} } });
  const account = { region: 'international', accountId: 'account-1' };
  let closed = false;
  mock.render(() => mock.exports.CodebuddyLaunchDialog({
    account, accountLabel: 'Account', plan: 'PRO', planClass: 'pro',
    onClose: () => { closed = true; },
  }));
  await settlePromises();
  const cliFields = find(mock.flush(), fields);
  cliFields.workingDir.onChange('/new/project');
  cliFields.onCustomCommandChange('codebuddy --resume');
  const copy = find(mock.flush(), 'button', (props) => Children.toArray(props.children).includes('common.copy'));
  copy.onClick();
  await settlePromises();

  const recovery = find(mock.flush(), pathField);
  assert.equal(recovery.target, 'codebuddy_cli');
  assert.ok(recovery.recovery);
  recovery.onSaved();
  await settlePromises();

  const request = { kind: 'account', account, workingDir: '/new/project', rememberWorkingDir: true };
  assert.deepEqual(JSON.parse(JSON.stringify(calls)), [[request, 'codebuddy --resume'], [request, 'codebuddy --resume']]);
  assert.deepEqual(clipboard, ['resolved-launch-command']);
  assert.equal(closed, false);
});
