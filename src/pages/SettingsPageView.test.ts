import { registerHooks } from 'node:module';

registerHooks({ load(url, context, next) {
  return url.endsWith('.css') ? { format: 'module', source: '', shortCircuit: true } : next(url, context);
} });
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { loadHookModule } from '../../tests/helpers/reactHookHarness.ts';

test('the network page opens report links only when a listener address is available', () => {
  const Section = () => null;
  const h = loadHookModule(new URL('./SettingsPageView.tsx', import.meta.url), {
    '../components/UnlockFireworksOverlay': { UnlockFireworksOverlay: Section },
    '../components/SettingsAccountTransferSection': { SettingsAccountTransferSection: Section },
    '../components/SettingsWebdavSyncSection': { SettingsWebdavSyncSection: Section },
    '../components/SettingsInstanceCleanupSection': { SettingsInstanceCleanupSection: Section },
    './SettingsGeneralPanel': { SettingsGeneralPanel: Section },
  });
  const opened: string[] = [];
  const props = {
    activeTab: 'network', t: (key: string) => key, reportEnabled: true, reportPort: '18081',
    reportRawPreviewUrl: '', reportRenderedPreviewUrl: '', reportActualPort: null as number | null,
    menuBarQuotaPlatformOptions: [], releaseHistoryItems: [], releaseHistorySections: [],
    openLink: (url: string) => opened.push(url),
  };
  const links = (node: any): any[] => !node || typeof node !== 'object' ? []
    : Array.isArray(node) ? node.flatMap(links)
    : [...(node.type === 'a' && node.props?.href?.includes('/report?') ? [node] : []), ...links(node.props?.children)];
  const render = () => h.exports.SettingsPageView(props);
  assert.deepEqual(links(h.render(render)), []);
  props.reportActualPort = 18082;
  props.reportRawPreviewUrl = 'http://127.0.0.1:18082/report?token=test%2Btoken';
  props.reportRenderedPreviewUrl = `${props.reportRawPreviewUrl}&render=true`;
  const available = links(h.render(render));
  assert.deepEqual(available.map((link) => link.props.href), [props.reportRawPreviewUrl, props.reportRenderedPreviewUrl]);
  available[1].props.onClick({ preventDefault() {} });
  assert.deepEqual(opened, [props.reportRenderedPreviewUrl]);
  h.unmount();
});

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

test('the Antigravity menu bar selector keeps exact models and one choice per quota window', () => {
  const Dropdown = () => null;
  const h = loadHookModule(new URL('../components/AntigravityMenuBarQuotaSelect.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key === 'common.quota.weeklyWindow' ? 'Weekly' : key }) },
    '../utils/antigravityQuotaWindows.json': {
      default: JSON.parse(readFileSync(new URL('../utils/antigravityQuotaWindows.json', import.meta.url), 'utf8')),
    },
    './SingleSelectDropdown': { SingleSelectDropdown: Dropdown },
  });
  const changes: string[] = [];
  const view = h.render(() => h.exports.AntigravityMenuBarQuotaSelect({
    accounts: [{ quota: { models: [
      { name: 'gemini-weekly', percentage: 80 },
      { name: 'gemini-3.8-flash', display_name: 'Gemini Flash', percentage: 20 },
    ] } }],
    value: 'unavailable-model',
    onChange: (value: string) => changes.push(value),
  })) as any;
  assert.equal(view.type, Dropdown);
  assert.ok(view.props.options.some(({ value, label }: { value: string; label: string }) =>
    value === 'gemini:weekly' && label === 'Gemini (Weekly)'));
  assert.ok(!view.props.options.some(({ value }: { value: string }) => value === 'gemini-weekly'));
  assert.ok(view.props.options.some(({ value, label }: { value: string; label: string }) =>
    value === 'gemini-3.8-flash' && label === 'Gemini Flash'));
  assert.ok(view.props.options.some(({ value }: { value: string }) => value === 'unavailable-model'));
  view.props.onChange('gemini:weekly');
  assert.deepEqual(changes, ['gemini:weekly']);
  h.unmount();
});
