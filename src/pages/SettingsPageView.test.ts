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

test('menu bar left-click setting exposes the same choices and disabled state in both layouts', () => {
  const Dropdown = () => null;
  const h = loadHookModule(new URL('../components/MenuBarLeftClickSetting.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    './SingleSelectDropdown': { SingleSelectDropdown: Dropdown },
  });
  const findDropdown = (node: any): any => {
    if (Array.isArray(node)) return node.map(findDropdown).find(Boolean);
    return node?.type === Dropdown
      ? node
      : node?.props?.children && findDropdown(node.props.children);
  };
  const changes: boolean[] = [];
  for (const variant of ['general', 'quick-settings']) {
    const render = (opensMainWindow: boolean, disabled: boolean) => findDropdown(h.render(() =>
      h.exports.MenuBarLeftClickSetting({
        variant,
        opensMainWindow,
        disabled,
        onChange: (next: boolean) => changes.push(next),
      }),
    )).props;
    const defaultSelect = render(false, false);
    assert.equal(defaultSelect.value, 'quota');
    assert.deepEqual(Array.from(defaultSelect.options, (option: { value: string }) => option.value), ['quota', 'window']);
    defaultSelect.onChange('window');
    const savedChoice = render(true, true);
    assert.equal(savedChoice.value, 'window');
    assert.equal(savedChoice.disabled, true);
  }
  assert.deepEqual(changes, [true, true]);
  h.unmount();
});

test('quota layout selector edits the draft and leaves confirmation to the dialog', () => {
  const Section = () => null;
  const Dropdown = () => null;
  const h = loadHookModule(new URL('./SettingsPageView.tsx', import.meta.url), {
    '../components/UnlockFireworksOverlay': { UnlockFireworksOverlay: Section },
    '../components/SettingsAccountTransferSection': { SettingsAccountTransferSection: Section },
    '../components/SettingsWebdavSyncSection': { SettingsWebdavSyncSection: Section },
    '../components/SettingsInstanceCleanupSection': { SettingsInstanceCleanupSection: Section },
    '../components/SingleSelectDropdown': { SingleSelectDropdown: Dropdown },
    './SettingsGeneralPanel': { SettingsGeneralPanel: Section },
  });
  const changes: string[] = [];
  let saved = false;
  let cancelled = false;
  const props = {
    activeTab: 'general', t: (key: string) => key,
    menuBarQuotaPlatformOptions: [], releaseHistoryItems: [], releaseHistorySections: [],
    menuBarQuotaModalOpen: true, menuBarQuotaModalMode: 'edit',
    menuBarQuotaDraftLayout: 'horizontal',
    setMenuBarQuotaDraftLayout: (value: string) => changes.push(value),
    handleConfirmMenuBarQuotaModal: () => { saved = true; },
    handleCloseMenuBarQuotaModal: () => { cancelled = true; },
  };
  const nodes = (node: any): any[] => !node || typeof node !== 'object' ? []
    : Array.isArray(node) ? node.flatMap(nodes) : [node, ...nodes(node.props?.children)];
  const tree = nodes(h.render(() => h.exports.SettingsPageView(props)));
  const selector = tree.find((node) => node.type === Dropdown && node.props.ariaLabel === 'settings.general.menuBarQuotaLayout');
  assert.equal(selector.props.value, 'horizontal');
  assert.deepEqual(Array.from(selector.props.options, (option: { value: string }) => option.value), ['horizontal', 'stacked']);
  selector.props.onChange('stacked');
  assert.deepEqual(changes, ['stacked']);
  assert.equal(saved, false);
  tree.find((node) => node.type === 'button' && node.props.children === 'common.cancel').props.onClick();
  assert.equal(cancelled, true);
  assert.equal(saved, false);
  props.menuBarQuotaDraftLayout = 'stacked';
  const edited = nodes(h.render(() => h.exports.SettingsPageView(props)));
  assert.equal(edited.find((node) => node.type === Dropdown && node.props.ariaLabel === 'settings.general.menuBarQuotaLayout').props.value, 'stacked');
  const previews = edited.filter((node) => node.type === 'label' && node.props.className?.includes('settings-menu-bar-quota-preview'));
  assert.equal(previews.length, 2);
  assert.equal(previews[0].props.className.includes('is-selected'), false);
  assert.equal(previews[1].props.className.includes('is-selected'), true);
  const radios = previews.map((preview) => nodes(preview).find((node) => node.type === 'input'));
  assert.deepEqual(radios.map((radio) => radio.props['aria-label']), [
    'settings.general.menuBarQuotaLayoutHorizontal', 'settings.general.menuBarQuotaLayoutStacked',
  ]);
  assert.deepEqual(radios.map((radio) => radio.props.checked), [false, true]);
  assert.equal(edited.some((node) => node.type === 'figcaption'), false);
  radios[0].props.onChange();
  assert.deepEqual(changes, ['stacked', 'horizontal']);
  props.menuBarQuotaDraftLayout = 'horizontal';
  const fromPreview = nodes(h.render(() => h.exports.SettingsPageView(props)));
  assert.equal(fromPreview.find((node) => node.type === Dropdown && node.props.ariaLabel === 'settings.general.menuBarQuotaLayout').props.value, 'horizontal');
  edited.find((node) => node.type === 'button' && node.props.children === 'common.save').props.onClick();
  assert.equal(saved, true);
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
      { name: 'gemini-argon-wtu-weekly', percentage: 90 },
      { name: 'argon:weekly', percentage: 90 },
      { name: 'gemini-3.8-flash', display_name: 'Gemini Flash', percentage: 20 },
    ] } }],
    value: 'unavailable-model',
    onChange: (value: string) => changes.push(value),
  })) as any;
  assert.equal(view.type, Dropdown);
  assert.ok(view.props.options.some(({ value, label }: { value: string; label: string }) =>
    value === 'gemini:weekly' && label === 'Gemini (Weekly)'));
  assert.ok(!view.props.options.some(({ value }: { value: string }) => value === 'gemini-weekly'));
  assert.ok(!view.props.options.some(({ value }: { value: string }) =>
    value === 'argon:weekly' || value === 'gemini-argon-wtu-weekly'));
  assert.ok(view.props.options.some(({ value, label }: { value: string; label: string }) =>
    value === 'gemini-3.8-flash' && label === 'Gemini Flash'));
  assert.ok(view.props.options.some(({ value }: { value: string }) => value === 'unavailable-model'));
  view.props.onChange('gemini:weekly');
  assert.deepEqual(changes, ['gemini:weekly']);
  h.unmount();
});
