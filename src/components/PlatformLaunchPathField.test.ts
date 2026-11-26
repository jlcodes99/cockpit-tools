import assert from 'node:assert/strict';
import test from 'node:test';
import { Children, isValidElement, type ReactNode } from 'react';
import { loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';

function controls(node: ReactNode, type: 'input' | 'button'): any[] {
  if (!isValidElement<{ children?: ReactNode }>(node)) return [];
  return [
    ...(node.type === type ? [node.props] : []),
    ...Children.toArray(node.props.children).flatMap((child) => controls(child, type)),
  ];
}

test('scanning a launch path previews the result until Save is pressed', async () => {
  const writes: Array<[string, string]> = [];
  const paths = ['/installed/CodeBuddy CN.app'];
  const component = loadHookModule(new URL('./PlatformLaunchPathField.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    './ModalErrorMessage': {
      ModalErrorMessage: () => null,
      useModalErrorState: () => ({ clear: () => {}, report: assert.fail }),
    },
    '../utils/platformLaunchPath': {
      PLATFORM_LAUNCH_PATHS: { codebuddy_cn: { label: 'CodeBuddy CN', guide: '' } },
      readPlatformLaunchPath: async () => ({ configuredPath: '/old/CodeBuddy CN.app', resolvedPath: null, usesConfiguredPath: false }),
      scanPlatformLaunchPaths: async () => paths,
      savePlatformLaunchPath: async (app: string, path: string) => { writes.push([app, path]); },
    },
  }, { window: new EventTarget() });
  component.render(() => component.exports.PlatformLaunchPathField({ target: 'codebuddy_cn' }));
  await settlePromises();

  const button = (translation: string) => controls(component.flush(), 'button')
    .find(({ children }) => Children.toArray(children).includes(translation));
  assert.equal(controls(component.flush(), 'input')[0].value, '/old/CodeBuddy CN.app');
  assert.equal(button('common.save').disabled, true);
  button('appPath.missing.autoDetect').onClick();
  await settlePromises();
  assert.equal(controls(component.flush(), 'input')[0].value, paths[0]);
  assert.deepEqual(writes, []);

  button('common.save').onClick();
  await settlePromises();
  assert.deepEqual(writes, [['codebuddy_cn', paths[0]]]);
  button('common.clear').onClick();
  assert.equal(controls(component.flush(), 'input')[0].value, '');
  assert.equal(writes.length, 1);
  button('common.save').onClick();
  await settlePromises();
  assert.deepEqual(writes[1], ['codebuddy_cn', '']);
  component.unmount();
});
