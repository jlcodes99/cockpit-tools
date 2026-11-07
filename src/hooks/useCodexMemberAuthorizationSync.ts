import { useEffect, useRef, type Dispatch, type SetStateAction } from 'react';
import type { CodexLocalAccessCollection, CodexLocalAccessImageGenerationPolicy } from '../types/codexLocalAccess';
import type { CustomRoutingDraftRule } from '../components/codex/codexLocalAccessModalModel';
import { CODEX_ACCOUNT_SERVICE_APPLIED, rebaseCodexMemberSelection, type CodexAccountServiceApplied } from '../utils/codexAccountServiceNavigation';

/** A completed inner authorization must not be overwritten by the outer member draft. */
export function useCodexMemberAuthorizationSync(isOpen: boolean, collection: CodexLocalAccessCollection | null,
  setSelected: Dispatch<SetStateAction<Set<string>>>,
  setPolicies: Dispatch<SetStateAction<Record<string, CodexLocalAccessImageGenerationPolicy>>>,
  setRouting: Dispatch<SetStateAction<Record<string, CustomRoutingDraftRule>>>) {
  const previous = useRef<readonly string[] | null>(null);
  useEffect(() => {
    if (!isOpen) return;
    const applied = (event: Event) => {
      const detail = (event as CustomEvent<CodexAccountServiceApplied>).detail;
      if (!detail || !Array.isArray(detail.relatedIds) || !Array.isArray(detail.selectedIds)) return;
      setSelected((draft) => {
        const next = new Set(draft);
        for (const id of detail.relatedIds) next.delete(id);
        for (const id of detail.selectedIds) if (detail.relatedIds.includes(id)) next.add(id);
        return next;
      });
    };
    window.addEventListener(CODEX_ACCOUNT_SERVICE_APPLIED, applied);
    return () => window.removeEventListener(CODEX_ACCOUNT_SERVICE_APPLIED, applied);
  }, [isOpen, setSelected]);
  useEffect(() => {
    if (!isOpen) { previous.current = null; return; }
    const ids = collection?.accountIds ?? [];
    const before = previous.current;
    previous.current = ids;
    if (!before || before === ids) return;
    const added = ids.filter((id) => !before.includes(id));
    setSelected((draft) => rebaseCodexMemberSelection(draft, before, ids));
    if (!added.length) return;
    setPolicies((draft) => ({ ...draft, ...Object.fromEntries(added.map((id) => [id, collection?.imageGenerationAccountPolicies?.[id] ?? 'inherit'])) }));
    setRouting((draft) => {
      const next = { ...draft };
      for (const id of added) {
        const rule = collection?.customRoutingRules?.find((item) => item.accountId === id);
        next[id] = { priority: rule?.priority ?? 0, weight: rule?.weight ?? 1, isBackup: Boolean(rule?.isBackup), isPreferred: Boolean(rule?.isPreferred) };
      }
      return next;
    });
  }, [isOpen, collection, setSelected, setPolicies, setRouting]);
}
