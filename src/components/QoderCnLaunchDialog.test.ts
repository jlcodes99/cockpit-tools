import assert from 'node:assert/strict';
import test from 'node:test';
import { Children, isValidElement, type ReactNode } from 'react';
import { loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';
import type { QoderClient } from '../utils/qoderRegion';

interface Control {
  children?: ReactNode;
  className?: string;
  title?: string;
  target?: string;
  recovery?: boolean;
  onClick?: () => void;
  onSaving?: (saving: boolean) => void;
  onSaved?: () => void;
}

function find(node: ReactNode, type: unknown, matches = (_props: Control) => true): Control | undefined {
  if (Array.isArray(node)) {
    for (const child of Children.toArray(node)) {
      const result = find(child, type, matches);
      if (result) return result;
    }
    return;
  }
  if (!isValidElement<Control>(node)) return;
  if (node.type === type && matches(node.props)) return node.props;
  for (const child of Children.toArray(node.props.children)) {
    const result = find(child, type, matches);
    if (result) return result;
  }
}

function loadLaunchDialog() {
  const pathField = () => null;
  const noop = () => {};
  const dialog = loadHookModule(new URL('./QoderCnLaunchDialog.tsx', import.meta.url), {
    'react-dom': { createPortal: (node: ReactNode) => node },
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../hooks/useEscClose': { useEscCloseTopmost: noop },
    '../hooks/useModalFocusTrap': { useModalFocusTrap: noop },
    '../hooks/usePlatformRuntimeSupport': { usePlatformRuntimeSupport: (target: string) => target === 'macos-only' },
    '../hooks/useDurableUiSetting': { useDurableUiSetting: () => undefined },
    '../hooks/useLaunchTerminalOptions': { useLaunchTerminalOptions: () => ({ terminalOptions: [] }) },
    './ClientInitializationStatus': { ClientInitializationStatus: noop },
    '../utils/durableUiSettings': { previewDurableUiSetting: noop },
    './CliLaunchFields': { CliLaunchFields: noop },
    './ModalErrorMessage': { ModalErrorMessage: noop, useModalErrorState: () => ({ clear: noop, report: noop }) },
    './PlatformLaunchPathField': { PlatformLaunchPathField: pathField },
    './icons/QoderIcon': { QoderIcon: noop },
  }, { document: { body: {} } });
  return { dialog, pathField };
}

test('Qoder IDE and CLI show their own paths and resume the selected action after saving', async () => {
  const requests: Array<{ client: QoderClient; launch: boolean }> = [];
  const { dialog, pathField } = loadLaunchDialog();
  let closed = false;
  const view = dialog.render(() => dialog.exports.QoderClientLaunchDialog({
    region: 'global',
    account: { id: 'fixture', plan_type: 'Free' },
    accountLabel: 'Account',
    onClose: () => { closed = true; },
    formatError: String,
    onSubmit: async (client: QoderClient, { launch }: { launch: boolean }) => {
      requests.push({ client, launch });
      if (requests.length === 1) throw 'CLI_PATH_NOT_FOUND:qoder_cli';
    },
  }));
  assert.equal(find(view, pathField)?.target, 'qoder');
  const openCli = find(view, 'button', props => props.title === 'qoder.clientTarget.cli')?.onClick;
  assert.ok(openCli);
  openCli();
  const cliView = dialog.flush();
  assert.equal(find(cliView, pathField)?.target, 'qoder_cli');
  const action = find(cliView, 'button', props => Children.toArray(props.children).includes('antigravityLaunch.switchAndLaunch'));
  assert.ok(action?.onClick);
  action.onClick();
  await settlePromises();
  const recovery = find(dialog.flush(), pathField);
  assert.equal(recovery?.recovery, true);
  recovery?.onSaving?.(true);
  const saving = find(dialog.flush(), pathField);
  saving?.onSaving?.(false);
  saving?.onSaved?.();
  await settlePromises();
  assert.deepEqual(requests, [{ client: 'cli', launch: true }, { client: 'cli', launch: true }]);
  assert.equal(closed, true);
});

test('Qoder keeps one launch action while the backend prepares the client', async () => {
  const { dialog } = loadLaunchDialog();
  let resolveLaunch!: () => void;
  const launch = new Promise<void>(resolve => { resolveLaunch = resolve; });
  let closed = false;
  const view = dialog.render(() => dialog.exports.QoderClientLaunchDialog({
    region: 'global',
    account: { id: 'fixture', plan_type: 'Free' },
    accountLabel: 'Account',
    onClose: () => { closed = true; },
    formatError: String,
    onSubmit: () => launch,
  }));
  const hasLabel = (label: string) => (props: Control) => Children.toArray(props.children).includes(label);
  assert.equal(find(view, 'button', hasLabel('common.open')), undefined);
  assert.equal(find(view, 'button', hasLabel('antigravityLaunch.switchOnly')), undefined);
  const submit = find(view, 'button', hasLabel('antigravityLaunch.switchAndLaunch'));
  assert.ok(submit?.onClick);
  submit.onClick();
  await settlePromises();
  assert.equal(closed, false);
  resolveLaunch();
  await settlePromises();
  assert.equal(closed, true);
});
