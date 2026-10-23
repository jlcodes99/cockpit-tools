import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  chatGptPlanErrorKey, listChatGptPlanAccounts, withChatGptPlanMembership,
  type ChatGptPlanAccount,
} from '../services/chatgptPlanService';
import { chatGptPlanOverviewId, reconcileChatGptPlanDisplaySources } from '../utils/chatgptPlanOverview';
import { subscribeTauriEvent } from '../utils/tauriEventListener';

/** Load independent grant metadata alongside ordinary accounts, never before them. */
export function useChatGptPlanOverview(memberAccountIds?: readonly string[], officialAccountBindings?: Readonly<Record<string, string>>) {
  const [snapshot, setSnapshot] = useState(() => ({ accounts: [] as ChatGptPlanAccount[], ...reconcileChatGptPlanDisplaySources([]) }));
  const accounts = snapshot.accounts;
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [dialogId, setDialogId] = useState<string | null>(null);
  const active = useRef(false);
  const running = useRef(false);
  const queued = useRef(false);
  const validationReads = useRef(0);
  const reload = useCallback(async () => {
    if (running.current) { queued.current = true; return; }
    running.current = true;
    setLoading(true);
    setError('');
    try {
      const next = await listChatGptPlanAccounts();
      if (active.current) setSnapshot((previous) => ({ accounts: next, ...reconcileChatGptPlanDisplaySources(next, previous.confirmed) }));
    } catch (failure) {
      if (active.current) setError(chatGptPlanErrorKey(failure));
    } finally {
      running.current = false;
      if (active.current) {
        setLoading(false);
        if (queued.current) { queued.current = false; void reload(); }
      }
    }
  }, []);
  useEffect(() => {
    active.current = true;
    void reload();
    const updated = () => { void reload(); };
    window.addEventListener('chatgpt-plan-accounts-changed', updated);
    window.addEventListener('codex-local-access-state-updated', updated);
    const unsubscribe = subscribeTauriEvent('codex-local-access-state-updated', updated);
    return () => {
      active.current = false;
      window.removeEventListener('chatgpt-plan-accounts-changed', updated);
      window.removeEventListener('codex-local-access-state-updated', updated);
      unsubscribe();
    };
  }, [reload]);
  // Validation can finish before the native event listener is installed. Keep
  // the first metadata read nonblocking, then reconcile pending rows without
  // relying on that one event or treating a pending association as authorized.
  useEffect(() => {
    const pending = accounts.some((account) => account.source_account_id && account.source_binding_valid !== true && !account.source_binding_error);
    if (!pending) { validationReads.current = 0; return; }
    if (loading || error || !active.current) return;
    if (validationReads.current >= 4) {
      if (validationReads.current++ === 4) setError('chatgptPlan.error.timeout');
      return;
    }
    const timer = setTimeout(() => { validationReads.current++; void reload(); }, 500 * 2 ** validationReads.current);
    return () => clearTimeout(timer);
  }, [accounts, loading, error, reload]);
  const visible = useMemo(() => accounts.map((account) => withChatGptPlanMembership(account, memberAccountIds, undefined, officialAccountBindings)), [accounts, memberAccountIds, officialAccountBindings]);
  const byId = useMemo(() => new Map(visible.map((account) => [chatGptPlanOverviewId(account.id), account])), [visible]);
  return {
    accounts: visible, displaySourceIds: snapshot.displaySourceIds, byId, loading, error, reload,
    dialogAccount: dialogId ? byId.get(dialogId) : undefined,
    open: setDialogId,
    close: () => setDialogId(null),
  };
}
