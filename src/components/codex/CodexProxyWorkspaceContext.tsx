import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactElement, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import type { CodexAccount } from '../../types/codex';
import { catalogErrorKey, getProxyCatalog, type ProxyCatalog } from '../../services/codexProxyCatalogService';
import { getCodexUnifiedProxy, unifiedProxyErrorKey, type CodexUnifiedProxyView } from '../../services/codexUnifiedProxyService';
import type { CodexProxyTraffic } from '../../utils/codexProxyTraffic';
import { useCodexProxyTraffic } from './useCodexProxyTraffic';
import { canUseCodexAccountProxy } from '../../utils/codexAccountProxy';

export type CodexProxySectionId = 'overview' | 'accounts' | 'resources' | 'connections' | 'rules' | 'logs' | 'tests' | 'settings';

export type { CodexProxyTraffic, CodexProxyTrafficSample } from '../../utils/codexProxyTraffic';

export interface CodexProxyWorkspaceValue {
  accounts: CodexAccount[];
  selectedId: string;
  entryAccountId?: string | null;
  selectAccount(id: string): void;
  catalog: ProxyCatalog;
  catalogLoading: boolean;
  catalogError: string;
  reloadCatalog(): void;
  acceptCatalog(catalog: ProxyCatalog): void;
  unified: CodexUnifiedProxyView | null;
  unifiedErrorKey: string;
  reloadUnified(): void;
  acceptUnified(view: CodexUnifiedProxyView): void;
  traffic: CodexProxyTraffic;
  section: CodexProxySectionId;
  goSection(id: CodexProxySectionId): void;
}

interface Props {
  accounts: CodexAccount[];
  accountId?: string | null;
  children: ReactNode;
}

const CodexProxyWorkspaceContext = createContext<CodexProxyWorkspaceValue | null>(null);

export function CodexProxyWorkspaceProvider({ accounts, accountId, children }: Props): ReactElement {
  const { t } = useTranslation();
  const eligible = useMemo(() => accounts.filter(canUseCodexAccountProxy), [accounts]);
  const [selectedId, setSelectedId] = useState(accountId ?? eligible[0]?.id ?? '');
  const [section, setSection] = useState<CodexProxySectionId>(accountId ? 'accounts' : 'resources');
  const [catalog, setCatalog] = useState<ProxyCatalog>({ sources: [] });
  const [catalogLoading, setCatalogLoading] = useState(true);
  const [catalogError, setCatalogError] = useState('');
  const [catalogRevision, setCatalogRevision] = useState(0);
  const catalogGeneration = useRef(0);
  const [unified, setUnified] = useState<CodexUnifiedProxyView | null>(null);
  const [unifiedErrorKey, setUnifiedErrorKey] = useState('');
  const [unifiedRevision, setUnifiedRevision] = useState(0);
  const unifiedGeneration = useRef(0);
  const previousSection = useRef<CodexProxySectionId | null>(null);
  // Async callbacks read the latest translator without re-running disk reads on language change.
  const translate = useRef(t);
  translate.current = t;
  const traffic = useCodexProxyTraffic(eligible.length > 0 && ['connections', 'logs'].includes(section));

  /** The host may open the workspace for a specific account. */
  useEffect(() => {
    if (accountId && eligible.some((entry) => entry.id === accountId)) {
      setSelectedId(accountId);
    }
  }, [accountId, eligible]);
  useEffect(() => { if (accountId) setSection('accounts'); }, [accountId]);
  useEffect(() => {
    if (!eligible.some((entry) => entry.id === selectedId)) setSelectedId(eligible[0]?.id ?? '');
  }, [eligible, selectedId]);

  useEffect(() => {
    let live = true;
    const generation = ++catalogGeneration.current;
    setCatalogLoading(true);
    void getProxyCatalog().then((next) => {
      if (!live || generation !== catalogGeneration.current) return;
      setCatalog(next);
      setCatalogError('');
    }).catch((caught) => {
      if (live && generation === catalogGeneration.current) setCatalogError(translate.current(catalogErrorKey(caught)));
    }).finally(() => {
      if (live && generation === catalogGeneration.current) setCatalogLoading(false);
    });
    return () => { live = false; };
  }, [catalogRevision]);

  useEffect(() => {
    let live = true;
    const generation = ++unifiedGeneration.current;
    void getCodexUnifiedProxy().then((next) => {
      if (!live || generation !== unifiedGeneration.current) return;
      setUnified(next);
      setUnifiedErrorKey('');
    }).catch((caught) => {
      if (live && generation === unifiedGeneration.current) setUnifiedErrorKey(unifiedProxyErrorKey(caught));
    });
    return () => { live = false; };
  }, [unifiedRevision]);

  /**
   * Resource edits happen inside the resources section, so the catalog is re-read on the way
   * back; the unified exit is re-read when its own section is opened.
   */
  useEffect(() => {
    const last = previousSection.current;
    previousSection.current = section;
    if (!last || last === section) return;
    if (last === 'resources') setCatalogRevision((value) => value + 1);
    if (section === 'accounts') setUnifiedRevision((value) => value + 1);
  }, [section]);

  const selectAccount = useCallback((id: string) => {
    if (eligible.some((entry) => entry.id === id)) setSelectedId(id);
  }, [eligible]);
  const reloadCatalog = useCallback(() => setCatalogRevision((value) => value + 1), []);
  const acceptCatalog = useCallback((next: ProxyCatalog) => {
    catalogGeneration.current += 1;
    setCatalog(next); setCatalogError(''); setCatalogLoading(false);
  }, []);
  const reloadUnified = useCallback(() => setUnifiedRevision((value) => value + 1), []);
  const acceptUnified = useCallback((view: CodexUnifiedProxyView) => {
    // A completed write wins over an older background read.
    unifiedGeneration.current += 1;
    setUnified(view);
    setUnifiedErrorKey('');
  }, []);
  const goSection = useCallback((id: CodexProxySectionId) => setSection(id === 'overview' ? 'resources' : id === 'rules' ? 'accounts' : id), []);
  const value = useMemo<CodexProxyWorkspaceValue>(() => ({
    accounts,
    entryAccountId: accountId,
    selectedId,
    selectAccount,
    catalog,
    catalogLoading,
    catalogError,
    reloadCatalog,
    acceptCatalog,
    unified,
    unifiedErrorKey,
    reloadUnified,
    acceptUnified,
    traffic,
    section,
    goSection,
  }), [accounts, accountId, selectedId, selectAccount, catalog, catalogLoading, catalogError, reloadCatalog, acceptCatalog, unified, unifiedErrorKey, reloadUnified, acceptUnified, traffic, section, goSection]);

  return <CodexProxyWorkspaceContext.Provider value={value}>{children}</CodexProxyWorkspaceContext.Provider>;
}

export function useCodexProxyWorkspace(): CodexProxyWorkspaceValue {
  const value = useContext(CodexProxyWorkspaceContext);
  if (!value) throw new Error('useCodexProxyWorkspace must be used inside CodexProxyWorkspaceProvider');
  return value;
}
