import { registerHooks } from 'node:module';

registerHooks({ load(url, context, next) {
  return url.endsWith('.css') ? { format: 'module', source: '', shortCircuit: true } : next(url, context);
} });
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../tests/helpers/reactHookHarness.ts';

test('data settings mount only on first visit and retain their component tree across tabs', () => {
  const Section = () => null;
  const h = loadHookModule(new URL('./SettingsPageView.tsx', import.meta.url), {
    '../components/UnlockFireworksOverlay': { UnlockFireworksOverlay: Section },
    '../components/SettingsAccountTransferSection': { SettingsAccountTransferSection: Section },
    '../components/SettingsWebdavSyncSection': { SettingsWebdavSyncSection: Section },
    '../components/SettingsInstanceCleanupSection': { SettingsInstanceCleanupSection: Section },
    './SettingsGeneralPanel': { SettingsGeneralPanel: Section },
  });
  let activeTab = 'general';
  const props = { t: (key: string) => key, menuBarQuotaPlatformOptions: [], releaseHistoryItems: [], releaseHistorySections: [] };
  const sections = (node: any): any[] => !node || typeof node !== 'object' ? []
    : Array.isArray(node) ? node.flatMap(sections)
    : [...(node.type === 'div' && typeof node.props?.hidden === 'boolean' ? [node] : []), ...sections(node.props?.children)];
  const render = () => h.exports.SettingsPageView({ ...props, activeTab });
  assert.equal(sections(h.render(render)).length, 0);
  activeTab = 'data';
  let group = sections(h.render(render))[0];
  assert.equal(group.props.hidden, false);
  assert.equal(group.props.children.length, 3);
  activeTab = 'general';
  group = sections(h.render(render))[0];
  assert.equal(group.props.hidden, true);
  assert.equal(group.props.children.length, 3);
  activeTab = 'data';
  assert.equal(sections(h.render(render))[0].props.hidden, false);
  h.unmount();
});
