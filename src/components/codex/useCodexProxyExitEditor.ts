import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { CodexAccount } from '../../types/codex';
import { useCodexAccountStore } from '../../stores/useCodexAccountStore';
import { canUseCodexAccountProxy } from '../../utils/codexAccountProxy';
import { isPrivacyModeEnabledByDefault, maskSensitiveValue, PRIVACY_MODE_CHANGED_EVENT } from '../../utils/privacy';
import {
  bindProxyCatalog, cancelProxyCatalog, catalogErrorKey, probeProxyCatalog,
  type ProxyCatalogSelections, type ProxyCatalogSource,
} from '../../services/codexProxyCatalogService';
import {
  cancelCodexAccountProxy, proxyErrorKey, testCodexAccountProxy,
  type CodexProxyProbeResult,
} from '../../services/codexAccountProxyService';
import { defaultProxySelections } from '../../utils/codexProxySelection';
import { codexProxyAccountName, exitDraftState, restoreExitChoice, type CodexProxyAccountIdentity, type CodexProxyExitChoice } from '../../utils/codexProxyDraft';
import { useCodexProxyWorkspace } from './CodexProxyWorkspaceContext';

export interface CodexProxyExitEditor {
  account: CodexAccount | undefined;
  /** 当前草稿（未保存的选择）；用户没动过时等于已保存绑定 */
  sourceId: string; itemId: string; groupId: string; selections: ProxyCatalogSelections;
  source: ProxyCatalogSource | undefined;
  saved: boolean;              // 草稿与已保存绑定一致
  dirty: boolean;
  bound: boolean;              // 该账号是否有已保存绑定
  result?: CodexProxyProbeResult;   // 最近一次出口检测结果
  testing: boolean;
  busy: 'save' | 'test' | 'unbind' | '';
  error: string;               // 已翻译文案，空串表示无错误
  notice: string;
  select(choice: { sourceId: string; itemId: string; groupId: string; selections: ProxyCatalogSelections }): void;
  test(): void;
  cancelTest(): void;
  save(): void;                // 写入账号绑定
  unbind(): void;              // 解除绑定
  clearError(): void;
  /** 追加字段（不属于冻结契约）：账号列表、运行记录与活动面板需要区分草稿结果与已保存结果。 */
  savedBinding: CodexAccount['egress_proxy'];
  resultScope: 'saved' | 'selection' | '';
  /** 当前草稿是否可写：节点或分组可用且手动分组已选成员。 */
  selectionReady: boolean;
}

type ProbeScope = 'saved' | 'selection';

function dropKey<T>(record: Record<string, T>, key: string): Record<string, T> {
  if (!Object.prototype.hasOwnProperty.call(record, key)) return record;
  const next = { ...record };
  delete next[key];
  return next;
}

function cancelProbe(active: { accountId?: string; requestId: string } | null): void {
  if (!active) return;
  void (active.accountId ? cancelCodexAccountProxy(active.accountId, active.requestId) : cancelProxyCatalog(active.requestId)).catch(() => {});
}

/**
 * One account's exit editor. The workspace shell owns the account list, the catalog and the
 * unified exit; this hook only owns one account's draft, its probe result and its writes.
 */
export function useCodexProxyExitEditor(accountId: string): CodexProxyExitEditor {
  const { t } = useTranslation();
  const { accounts, catalog } = useCodexProxyWorkspace();
  const updateAccountEgressProxy = useCodexAccountStore((state) => state.updateAccountEgressProxy);
  const account = useMemo(
    () => accounts.find((entry) => entry.id === accountId && canUseCodexAccountProxy(entry)),
    [accounts, accountId],
  );
  const [overrides, setOverrides] = useState<Record<string, CodexAccount['egress_proxy']>>({});
  const [draft, setDraft] = useState<{ accountId: string; choice: CodexProxyExitChoice } | null>(null);
  const [results, setResults] = useState<Record<string, { value: CodexProxyProbeResult; scope: ProbeScope }>>({});
  const [busy, setBusy] = useState<'save' | 'test' | 'unbind' | ''>('');
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const generation = useRef(0);
  const operation = useRef(false);
  const testingId = useRef<{ accountId?: string; requestId: string } | null>(null);

  // A write returns the authoritative snapshot; it wins over the list until the store catches up.
  const savedBinding = Object.prototype.hasOwnProperty.call(overrides, accountId)
    ? overrides[accountId] ?? null
    : account?.egress_proxy ?? null;
  const restored = useMemo(() => restoreExitChoice(catalog, savedBinding), [catalog, savedBinding]);
  const currentDraft = draft?.accountId === accountId ? draft.choice : null;
  const choice = currentDraft ?? restored;
  const { bound, saved, dirty } = exitDraftState(savedBinding, restored, choice);
  const source = catalog.sources.find((entry) => entry.id === choice.sourceId);
  const selected = source?.nodes.find((entry) => entry.id === choice.itemId) ?? source?.groups.find((entry) => entry.id === choice.itemId);
  const selections = useMemo(
    () => source ? defaultProxySelections(source, choice.itemId, choice.selections) : null,
    [source, choice.itemId, choice.selections],
  );
  const selectionReady = Boolean(selected?.supported) && selections !== null;
  const probe = results[accountId];

  useEffect(() => {
    // Each account owns a separate operation lifetime. A late completion from
    // the old account must neither keep this account busy nor unlock its work.
    generation.current += 1;
    const active = testingId.current;
    testingId.current = null;
    operation.current = false;
    cancelProbe(active);
    setBusy('');
    setDraft(null);
    setError('');
    setNotice('');
  }, [accountId]);

  useEffect(() => () => {
    generation.current += 1;
    const active = testingId.current;
    testingId.current = null;
    cancelProbe(active);
  }, []);

  const select = useCallback((next: CodexProxyExitChoice) => {
    if (operation.current) return;
    setDraft({ accountId, choice: next });
    setResults((old) => dropKey(old, accountId));
    setError('');
    setNotice('');
  }, [accountId]);

  const cancelTest = useCallback(() => { cancelProbe(testingId.current); }, []);

  const test = useCallback(async () => {
    if (operation.current || !account) return;
    // Only a changed draft is attributed to the draft; otherwise the saved binding is checked.
    const scope: ProbeScope = selectionReady && dirty ? 'selection' : 'saved';
    if (scope === 'saved' && !bound) return;
    operation.current = true;
    setBusy('test'); setError(''); setNotice('');
    const id = account.id;
    const current = generation.current;
    const requestId = crypto.randomUUID();
    testingId.current = { accountId: scope === 'saved' ? id : undefined, requestId };
    try {
      const checked = scope === 'selection'
        ? await probeProxyCatalog(requestId, choice.sourceId, choice.itemId, selections ?? {})
        : await testCodexAccountProxy(id, requestId, null);
      if (generation.current === current) setResults((old) => ({ ...old, [id]: { value: checked, scope } }));
    } catch (caught) {
      if (generation.current === current) {
        setError(t(scope === 'selection' ? catalogErrorKey(caught) : proxyErrorKey(caught, 'probeFailed')));
      }
    } finally {
      if (generation.current === current) {
        testingId.current = null;
        operation.current = false;
        setBusy('');
      }
    }
  }, [account, bound, choice.itemId, choice.sourceId, dirty, selections, selectionReady, t]);

  const save = useCallback(async () => {
    if (operation.current || !account || saved || !selectionReady) return;
    operation.current = true;
    setBusy('save'); setError(''); setNotice('');
    const id = account.id;
    const current = generation.current;
    try {
      const updated = await bindProxyCatalog(id, choice.sourceId, choice.itemId, selections ?? {}, choice.groupId);
      useCodexAccountStore.getState().applyAccountSnapshot(updated);
      if (generation.current !== current) return;
      setOverrides((old) => ({ ...old, [id]: updated.egress_proxy ?? null }));
      setDraft(null);
      setResults((old) => dropKey(old, id));
      setNotice(t('codex.proxy.saved'));
    } catch (caught) {
      if (generation.current === current) setError(t(catalogErrorKey(caught)));
    } finally {
      if (generation.current === current) {
        operation.current = false;
        setBusy('');
      }
    }
  }, [account, choice.groupId, choice.itemId, choice.sourceId, saved, selections, selectionReady, t]);

  const unbind = useCallback(async () => {
    if (operation.current || !account || !bound) return;
    operation.current = true;
    setBusy('unbind'); setError(''); setNotice('');
    const id = account.id;
    const current = generation.current;
    try {
      const updated = await updateAccountEgressProxy(id, null);
      if (generation.current !== current) return;
      setOverrides((old) => ({ ...old, [id]: updated.egress_proxy ?? null }));
      setDraft(null);
      setResults((old) => dropKey(old, id));
      setNotice(t('codex.proxy.unboundHint'));
    } catch (caught) {
      if (generation.current === current) setError(t(proxyErrorKey(caught)));
    } finally {
      if (generation.current === current) {
        operation.current = false;
        setBusy('');
      }
    }
  }, [account, bound, t, updateAccountEgressProxy]);

  const clearError = useCallback(() => setError(''), []);

  return {
    account,
    sourceId: choice.sourceId,
    itemId: choice.itemId,
    groupId: choice.groupId,
    selections: choice.selections,
    source,
    saved,
    dirty,
    bound,
    result: probe?.value,
    testing: busy === 'test',
    busy,
    error,
    notice,
    select,
    test: () => { void test(); },
    cancelTest,
    save: () => { void save(); },
    unbind: () => { void unbind(); },
    clearError,
    savedBinding,
    resultScope: probe?.scope ?? '',
    selectionReady,
  };
}

/** Privacy-mode aware account label shared by the list, the rules table and the dialogs. */
export function useCodexProxyAccountName(): (account?: CodexProxyAccountIdentity | null) => string {
  const [privacy, setPrivacy] = useState(isPrivacyModeEnabledByDefault);
  useEffect(() => {
    const sync = () => setPrivacy(isPrivacyModeEnabledByDefault());
    window.addEventListener(PRIVACY_MODE_CHANGED_EVENT, sync);
    return () => window.removeEventListener(PRIVACY_MODE_CHANGED_EVENT, sync);
  }, []);
  return useCallback(
    (account?: CodexProxyAccountIdentity | null) => account ? maskSensitiveValue(codexProxyAccountName(account), privacy) : '',
    [privacy],
  );
}
