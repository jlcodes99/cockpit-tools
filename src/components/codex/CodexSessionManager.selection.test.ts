import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import postcss from 'postcss';
import { loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';
import { filterCodexSessionsByKind } from '../../utils/codexSessionFilters';

function nodes(tree: any): any[] {
  if (Array.isArray(tree)) return tree.flatMap(nodes);
  return tree && typeof tree === 'object' ? [tree, ...nodes(tree.props?.children)] : [];
}
function text(tree: any): string {
  if (Array.isArray(tree)) return tree.map(text).join('');
  return tree && typeof tree === 'object' ? text(tree.props?.children) : String(tree ?? '');
}
const prefix = 'codex.sessionManager.actions.';
async function harness() {
  const sessions = Array.from({ length: 18 }, (_, index) => ({
    sessionId: `chat-${index}`, cwd: index < 10 ? '/project-a' : '/project-b',
    title: `Chat ${index}`, updatedAt: 1, locations: [], sessionKind: 'conversation',
  }));
  const store = { instances: [], listSessionsAcrossInstances: async () => sessions,
    getSessionTokenStatsAcrossInstances: async () => [] };
  const t = (key: string) => key;
  const error = { message: null, scrollKey: 0, set() {} };
  const h = loadHookModule(new URL('./CodexSessionManager.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t, i18n: { resolvedLanguage: 'en' } }) },
    '@tauri-apps/plugin-dialog': {},
    '@tauri-apps/api/event': { listen: async () => () => {} },
    '../../hooks/useEscClose': { useEscClose() {} },
    '../../stores/useCodexInstanceStore': { useCodexInstanceStore: (selector: any) => selector(store) },
    '../../services/codexInstanceService': {},
    '../../utils/codexSessionFilters': { filterCodexSessionsByKind },
    '../ModalErrorMessage': { ModalErrorMessage: 'error', useModalErrorState: () => error },
    '../SingleSelectDropdown': { SingleSelectDropdown: 'dropdown' },
    './CodexSessionVisibilityRepairModal': { CodexSessionVisibilityRepairModal: 'repair-dialog' },
    './CodexSessionUsagePanel': { CodexSessionUsagePanel: 'usage-panel', CodexSessionUsageSummary: 'usage-summary' },
  }, { localStorage: { getItem: () => null }, window: { setTimeout, clearTimeout }, console });
  h.render(() => h.exports.CodexSessionManager());
  await settlePromises(); h.flush(); await settlePromises();
  const rendered = () => nodes(h.flush());
  const button = (key: string) => {
    const matches = rendered().filter((node) => node.type === 'button' && (node.props['aria-label'] ?? text(node)).includes(prefix + key));
    assert.equal(matches.length, 1, `${key} must have exactly one button`);
    return matches[0];
  };
  const groups = () => rendered().filter((node) => node.props?.className === 'codex-session-folder__checkbox');
  const rows = () => rendered().filter((node) => node.props?.className === 'codex-session-row__checkbox');
  const assertCount = (count: number) => {
    for (const action of ['copyToInstance', 'exportSessions', 'moveToTrash']) {
      const node = button(action);
      assert.equal(node.props.disabled, count === 0);
      const counters = nodes(node).filter((item) => item.props?.className === 'codex-session-manager__action-count');
      assert.equal(counters.length, 1);
      assert.equal(text(counters[0]), `(${count})`);
      assert.equal(counters[0].props.style.minWidth, '4ch', 'count width must not depend on selection');
    }
  };
  return { h, rendered, button, groups, rows, assertCount };
}

test('rapid batched group toggles use the latest selection and preserve other groups', async () => {
  const app = await harness();
  try {
    app.assertCount(0);
    const toggle = app.groups()[0].props.onChange;
    toggle(); toggle();
    app.assertCount(0);
    app.groups()[1].props.onChange();
    app.assertCount(8);
    const again = app.groups()[0].props.onChange;
    for (let index = 0; index < 20; index++) again();
    app.assertCount(8);
    assert.equal(app.groups()[0].props.checked, false);
    assert.equal(app.groups()[1].props.checked, true);
    again(); app.assertCount(18);
    again(); app.assertCount(8);
  } finally { app.h.unmount(); }
});

test('single, group and select-all changes keep exactly one batch action and correct counts', async () => {
  const app = await harness();
  try {
    app.assertCount(0);
    for (let index = 0; index < 20; index++) {
      app.button('selectAllSessions').props.onClick(); app.assertCount(18);
      app.button('clearSelectedSessions').props.onClick(); app.assertCount(0);
      if (!app.rows().length) {
        app.rendered().find((node) => node.props?.className === 'codex-session-folder__expand').props.onClick();
      }
      app.rows()[0].props.onChange(); app.assertCount(1);
      app.rows()[0].props.onChange(); app.assertCount(0);
    }
  } finally { app.h.unmount(); }
});

test('direct index repair and visibility entries route to one dialog with the selected chats', async () => {
  const app = await harness();
  try {
    app.groups()[1].props.onChange();
    app.button('repairIndex').props.onClick();
    let dialogs = app.rendered().filter((node) => node.type === 'repair-dialog');
    assert.equal(dialogs.length, 1);
    assert.equal(dialogs[0].props.open, true);
    assert.equal(dialogs[0].props.repairKind, 'history');
    assert.equal(dialogs[0].props.selectedSessionIds.length, 8);
    dialogs[0].props.onClose();
    assert.equal(app.rendered().find((node) => node.type === 'repair-dialog').props.open, false);
    app.button('repairVisibility').props.onClick();
    dialogs = app.rendered().filter((node) => node.type === 'repair-dialog');
    assert.equal(dialogs.length, 1);
    assert.equal(dialogs[0].props.repairKind, 'visibility');
  } finally { app.h.unmount(); }
});

test('selection toolbar overrides inherited fading transitions and reserves numeric width', () => {
  const css = postcss.parse(readFileSync(new URL('../../styles/pages/codex-session-manager.css', import.meta.url), 'utf8'));
  const values = (selector: string) => {
    const found: Record<string, string> = {};
    css.walkRules(selector, (rule) => { rule.walkDecls((decl) => { found[decl.prop] = decl.value; }); });
    return found;
  };
  const scope = '.codex-session-manager:not(.codebuddy-session-manager) .is-selection .codex-session-manager__action-button';
  assert.equal(values(scope).transition, 'none');
  assert.equal(values(`${scope}:disabled`).opacity, '1');
  assert.equal(values(`${scope}:disabled`).color, 'var(--text-muted)');
  assert.equal(values('.codex-session-manager__action-count')['font-variant-numeric'], 'tabular-nums');
  assert.equal(values('.codex-session-manager__action-count')['flex-shrink'], '0');
});
