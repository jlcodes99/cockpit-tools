import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../../tests/helpers/reactHookHarness';

function nodes(tree: any): any[] {
  if (!tree || typeof tree !== 'object') return [];
  if (Array.isArray(tree)) return tree.flatMap(nodes);
  return [tree, ...nodes(tree.props?.children)];
}

function texts(node: any): string[] {
  if (node == null || typeof node === 'boolean') return [];
  if (typeof node === 'string' || typeof node === 'number') return [String(node)];
  if (Array.isArray(node)) return node.flatMap(texts);
  if (typeof node === 'object') {
    if (typeof node.type === 'function') return texts(node.type(node.props));
    return texts(node.props?.children);
  }
  return [];
}

function setupHistory(entries: any[] = [], legacyRecords: any[] = []) {
  const t = (key: string, args?: any) => (args ? `${key}:${JSON.stringify(args)}` : key);
  const h = loadHookModule(new URL('./CodexResetHistory.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t, i18n: { language: 'en-US' } }) },
    '../ModalErrorMessage': { ModalErrorMessage: 'ModalError' },
    '../../hooks/useCodexResetHistory': {
      useCodexResetHistory: () => ({
        entries,
        legacyRecords,
        cursor: null,
        loading: false,
        error: null,
        errorKey: 0,
        refresh: async () => {},
        more: async () => {},
        retry: async () => {},
      }),
    },
  });
  return { h, t };
}

test('ruleDescriptions formats historical rule thresholds for read-only display', () => {
  const { h, t } = setupHistory();
  assert.deepEqual(
    Array.from(
      h.exports.ruleDescriptions(
        {
          enabled: true,
          expiry_enabled: true,
          before_seconds: 7200,
          quota_enabled: true,
          remaining_percent: 15,
        },
        t,
      ),
    ),
    [
      'codex.resetAutomation.previewExpiry:{"value":2,"unit":"codex.resetAutomation.hours"}',
      'codex.resetAutomation.previewWeekly:{"percent":15}',
    ],
  );
  assert.deepEqual(
    Array.from(
      h.exports.ruleDescriptions(
        {
          enabled: false,
          expiry_enabled: true,
          before_seconds: 172800,
          quota_enabled: true,
          remaining_percent: 5,
          prefer_usage_credits: true,
        },
        t,
      ),
    ),
    [
      'codex.resetAutomation.previewExpiry:{"value":2,"unit":"codex.resetAutomation.days"}',
      'codex.resetAutomation.previewWeeklyAfterCredits:{"percent":5}',
    ],
  );
});

test('CodexResetHistory renders historical automatic records read-only alongside manual history', () => {
  const { h } = setupHistory(
    [
      {
        id: 'auto-1',
        at: 1_700_000_000_000,
        kind: 'reset',
        reason: 'quota',
        requestId: 'auto-1',
        context: {
          rule: {
            enabled: true,
            expiry_enabled: false,
            before_seconds: 300,
            quota_enabled: true,
            remaining_percent: 10,
          },
          before: {
            available_count: 1,
            available_card_ids: ['card-1'],
            hourly: null,
            weekly: { remaining: 8 },
          },
        },
        after: {
          available_count: 0,
          available_card_ids: [],
          hourly: null,
          weekly: { remaining: 100 },
        },
        receipt: { credit_id: 'card-1', windows_reset: 1 },
      },
      {
        id: 'manual-1',
        at: 1_700_001_000_000,
        kind: 'reset',
        reason: 'manual',
        requestId: 'manual-1',
      },
    ],
    [
      {
        id: 'legacy-check-1',
        account_id: 'acc-1',
        at: 1_699_999_000,
        reason: 'check',
        status: 'failed',
        detail: 'offline',
      },
    ],
  );

  h.render(() =>
    h.exports.CodexResetHistory({ accountId: 'acc-1', children: 'available-credits-view' }),
  );
  const initialTree = nodes(h.flush());
  const localTab = initialTree.find(
    (node) => node.type === 'button' && node.props?.children === 'codex.resetHistory.local',
  );
  assert.ok(localTab);
  localTab.props.onClick();

  const localRoot = h.flush();
  const renderedText = texts(localRoot).join('');

  assert.match(renderedText, /codex\.resetHistory\.weekly/);
  assert.match(renderedText, /codex\.resetHistory\.manual/);
  assert.match(renderedText, /codex\.resetAutomation\.previewWeekly:\{"percent":10\}/);
  assert.match(renderedText, /codex\.resetHistory\.redeemedCardId · card-1/);
  assert.match(renderedText, /codex\.resetHistory\.before · codex\.quota\.resetCredits:\{"count":1\} · codex\.quota\.weekly 8%/);
  assert.match(renderedText, /codex\.resetHistory\.after · codex\.quota\.resetCredits:\{"count":0\} · codex\.quota\.weekly 100%/);
  assert.match(renderedText, /codex\.resetHistory\.legacyActivityTitle/);
  assert.match(renderedText, /codex\.resetHistory\.legacyActivityHint/);
  assert.match(renderedText, /codex\.resetHistory\.legacyReasonCheck · common\.failedoffline/);
  assert.doesNotMatch(renderedText, /masterLabel|applyAll|editPublic/);
  h.unmount();
});

test('CodexResetHistory keeps legacy activities clearly separated even with no new local records', () => {
  const { h } = setupHistory([], [
    {
      id: 'legacy-expiry-1',
      account_id: 'acc-1',
      at: 1_699_998_000,
      reason: 'expiry',
      status: 'success',
      detail: null,
    },
  ]);

  h.render(() =>
    h.exports.CodexResetHistory({ accountId: 'acc-1', children: 'available-credits-view' }),
  );
  const localTab = nodes(h.flush()).find(
    (node) => node.type === 'button' && node.props?.children === 'codex.resetHistory.local',
  );
  localTab.props.onClick();

  const renderedText = texts(h.flush()).join('');
  assert.match(renderedText, /codex\.resetHistory\.localEmpty/);
  assert.match(renderedText, /codex\.resetHistory\.legacyActivityTitle/);
  assert.match(renderedText, /codex\.resetHistory\.legacyReasonExpiry · common\.success/);
  assert.doesNotMatch(renderedText, /codex\.resetHistory\.used/);
  h.unmount();
});
