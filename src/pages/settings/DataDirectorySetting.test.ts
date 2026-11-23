import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';

function nodes(value: any): any[] {
  if (!value || typeof value !== 'object') return [];
  if (Array.isArray(value)) return value.flatMap(nodes);
  return [value, ...nodes(value.props?.children)];
}

async function harness(activeCustomPath: string | null = null) {
  const currentPath = activeCustomPath ?? 'C:\\Users\\fixture\\.cockpit_tools';
  const calls: (string | null)[] = [];
  let choice: string | null = null;
  let saveError: string | null = null;
  const app = loadHookModule(new URL('./DataDirectorySetting.tsx', import.meta.url), {
    '@tauri-apps/api/core': {
      async invoke(command: string, payload?: { path: string | null }) {
        if (command === 'get_data_directory_settings') return {
          currentPath, configuredPath: activeCustomPath, environmentOverride: false, restartRequired: false, error: null,
        };
        assert.equal(command, 'set_data_directory');
        calls.push(payload!.path);
        if (saveError) throw new Error(saveError);
        return {
          currentPath, configuredPath: payload!.path, environmentOverride: false,
          restartRequired: activeCustomPath !== payload!.path, error: null,
        };
      },
    },
    '@tauri-apps/plugin-dialog': { open: async () => choice },
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../../services/accountService': { openDataFolder: async () => {} },
  });
  app.render(() => app.exports.DataDirectorySetting());
  await settlePromises();
  return {
    calls, currentPath,
    choose(value: string | null) { choice = value; },
    fail(value: string) { saveError = value; },
    render() { return nodes(app.flush()); },
    async click(label: string) {
      nodes(app.flush()).find(node => node.type === 'button' && node.props.children === label).props.onClick();
      await settlePromises();
    },
  };
}

test('selecting a directory shows a pending path while opening the picker and cancelling saves nothing', async () => {
  const view = await harness();
  await view.click('instances.launchDialog.chooseDirectory');
  assert.deepEqual(view.calls, []);

  const destination = 'D:\\Existing Cockpit Data';
  view.choose(destination);
  await view.click('instances.launchDialog.chooseDirectory');
  assert.deepEqual(view.calls, [destination]);
  const tree = view.render();
  assert.equal(tree.find(node => node.type === 'code').props.children, view.currentPath);
  assert.equal(tree.find(node => node.props.role === 'status').props.children, 'settings.general.dataDirPending');

  await view.click('platformLayout.reset');
  assert.deepEqual(view.calls, [destination, null]);
  assert.equal(view.render().some(node => node.props.role === 'status'), false);
});

test('resetting an active custom directory keeps it until restart and shows the default is pending', async () => {
  const view = await harness('D:\\Active Cockpit Data');
  await view.click('platformLayout.reset');
  const tree = view.render();
  assert.deepEqual(view.calls, [null]);
  assert.equal(tree.find(node => node.type === 'code').props.children, view.currentPath);
  assert.equal(tree.find(node => node.props.role === 'status').props.children, 'settings.general.dataDirPendingDefault');
});

test('a directory save error keeps the active path and shows the actual failure', async () => {
  const view = await harness();
  view.choose('D:\\Unavailable');
  view.fail('directory is unavailable');
  await view.click('instances.launchDialog.chooseDirectory');
  const tree = view.render();
  assert.equal(tree.find(node => node.type === 'code').props.children, view.currentPath);
  assert.match(tree.find(node => node.props.role === 'alert').props.children, /directory is unavailable/);
  assert.equal(tree.some(node => node.props.role === 'status'), false);
});
