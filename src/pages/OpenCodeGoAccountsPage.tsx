import { useEffect, useMemo, useState } from 'react';
import {
  ArrowDownWideNarrow,
  BadgeCheck,
  Clock3,
  KeyRound,
  LayoutGrid,
  List,
  Pencil,
  Plus,
  RefreshCw,
  Search,
  Trash2,
  X,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { OpenCodeIcon } from '../components/icons/OpenCodeIcon';
import { SingleSelectFilterDropdown } from '../components/SingleSelectFilterDropdown';
import { OpenCodeGoQuotaWindowCards } from '../components/opencode-go/OpenCodeGoQuotaWindowCards';
import { OpenCodeGoAddAccountModal } from '../components/opencode-go/OpenCodeGoAddAccountModal';
import { useOpenCodeGoAccountStore } from '../stores/useOpenCodeGoAccountStore';
import type { OpenCodeGoConnection } from '../types/openCodeGo';
import {
  createOpenCodeGoConnectionSlots,
  filterAndSortOpenCodeGoConnections,
  normalizeOpenCodeGoConnectionName,
  resolveOpenCodeGoConnectionTier,
  type OpenCodeGoConnectionQuery,
  type OpenCodeGoConnectionTier,
} from '../utils/openCodeGoConnections';

type EditorState = { slotIndex: number; connection: OpenCodeGoConnection | null };
type TierFilter = 'all' | OpenCodeGoConnectionTier;
type SortBy = NonNullable<OpenCodeGoConnectionQuery['sortBy']>;
type ProviderTab = 'go' | 'zen';

export function OpenCodeGoAccountsPage() {
  const { t } = useTranslation();
  const quotaErrorText = (kind: string | undefined): string => {
    switch (kind) {
      case 'authentication':
        return t('openCodeGo.errors.authentication', 'Authentication failed. Check this connection key.');
      case 'rate_limit':
        return t('openCodeGo.errors.rateLimit', 'OpenCode Go is rate limiting usage checks. Try again shortly.');
      case 'network':
        return t('openCodeGo.errors.network', 'Unable to reach OpenCode Go.');
      default:
        return t('openCodeGo.errors.unavailable', 'Usage data is unavailable for this connection.');
    }
  };
  const actionErrorText = (error: unknown): string => {
    const code = String(error).toUpperCase();
    if (code.includes('AUTHENTICATION')) return quotaErrorText('authentication');
    if (code.includes('RATE_LIMIT')) return quotaErrorText('rate_limit');
    if (code.includes('NETWORK')) return quotaErrorText('network');
    return quotaErrorText(undefined);
  };
  const store = useOpenCodeGoAccountStore();
  const [editor, setEditor] = useState<EditorState | null>(null);
  const [showAddAccount, setShowAddAccount] = useState(false);
  const [name, setName] = useState('');
  const [email, setEmail] = useState('');
  const [apiKey, setApiKey] = useState('');
  const [saving, setSaving] = useState(false);
  const [actionError, setActionError] = useState('');
  const [viewMode, setViewMode] = useState<'grid' | 'list'>('grid');
  const [searchQuery, setSearchQuery] = useState('');
  const [tierFilter, setTierFilter] = useState<TierFilter>('all');
  const [sortBy, setSortBy] = useState<SortBy>('name');
  const [sortDirection, setSortDirection] = useState<'asc' | 'desc'>('asc');
  const [providerTab, setProviderTab] = useState<ProviderTab>('go');
  const [deleteCandidateId, setDeleteCandidateId] = useState<string | null>(null);
  const [pendingDeleteId, setPendingDeleteId] = useState<string | null>(null);
  const [refreshingConnectionId, setRefreshingConnectionId] = useState<string | null>(null);
  const [testingConnectionId, setTestingConnectionId] = useState<string | null>(null);
  const [testedConnectionId, setTestedConnectionId] = useState<string | null>(null);

  useEffect(() => { void store.fetchAccounts(); }, [store.fetchAccounts]);

  const providerConnections = useMemo(
    () => store.accounts.filter((connection) => (connection.provider ?? 'go') === providerTab),
    [providerTab, store.accounts],
  );
  const visibleConnections = useMemo(
    () => filterAndSortOpenCodeGoConnections(providerConnections, {
      query: searchQuery,
      tier: tierFilter,
      sortBy,
      sortDirection,
    }),
    [providerConnections, searchQuery, sortBy, sortDirection, tierFilter],
  );
  const hasActiveFilter = Boolean(searchQuery.trim()) || tierFilter !== 'all';
  const slots = useMemo(
    () => createOpenCodeGoConnectionSlots(visibleConnections),
    [visibleConnections],
  );

  const openEditor = (slotIndex: number, connection: OpenCodeGoConnection | null) => {
    setEditor({ slotIndex, connection });
    setName(connection?.name ?? '');
    // Raw emails never leave encrypted storage; blank means retain the saved association.
    setEmail('');
    setApiKey('');
    setActionError('');
  };
  const closeEditor = () => {
    if (saving) return;
    setEditor(null);
    setEmail('');
    setApiKey('');
    setActionError('');
  };
  const refreshConnection = async (connectionId: string) => {
    setRefreshingConnectionId(connectionId);
    setActionError('');
    try {
      await store.refreshQuota(connectionId);
    } catch (error) {
      setActionError(actionErrorText(error));
    } finally {
      setRefreshingConnectionId(null);
    }
  };
  const testConnection = async (connectionId: string) => {
    setTestingConnectionId(connectionId);
    setTestedConnectionId(null);
    setActionError('');
    try {
      await store.testConnection(connectionId);
      setTestedConnectionId(connectionId);
    } catch (error) {
      setActionError(actionErrorText(error));
    } finally {
      setTestingConnectionId(null);
    }
  };
  const deleteConnection = async () => {
    if (!deleteCandidateId || pendingDeleteId) return;
    setPendingDeleteId(deleteCandidateId);
    setActionError('');
    try {
      await store.deleteConnection(deleteCandidateId);
    } catch {
      setActionError(t('openCodeGo.errors.delete', 'Unable to delete this connection. Try again.'));
    } finally {
      setPendingDeleteId(null);
      setDeleteCandidateId(null);
    }
  };
  const deleteCandidate = store.accounts.find((connection) => connection.id === deleteCandidateId) ?? null;
  const saveEditor = async () => {
    if (!editor || (!editor.connection && !apiKey.trim())) return;
    setSaving(true);
    setActionError('');
    try {
      if (editor.connection) {
        await store.updateConnection(editor.connection.id, {
          name,
          ...(email.trim() ? { email } : {}),
          ...(apiKey.trim() ? { apiKey } : {}),
        });
      } else {
        await store.createConnection(name, apiKey, email, providerTab);
      }
      setEditor(null);
      setApiKey('');
    } catch (error) {
      setActionError(actionErrorText(error));
    } finally {
      setSaving(false);
    }
  };

  return (
    <main className="main-content opencode-go-accounts-page fade-in">
      <div className="page-tabs-row opencode-go-header">
        <div className="platform-header-title">
          <span className="opencode-go-title-icon"><OpenCodeIcon size={28} /></span>
          <div>
            <h1 className="opencode-go-title">OpenCode Go</h1>
            <p>{t('openCodeGo.subtitle', 'Configured connections and 5-hour, weekly, and monthly quota windows.')}</p>
          </div>
        </div>
        <div className="opencode-go-header-actions">
          <button
            type="button"
            className="header-action-btn"
            onClick={() => setShowAddAccount(true)}
          >
            <Plus size={15} />
            <span>{t('openCodeGo.add.submit', 'Add connection')}</span>
          </button>
          <button type="button" className="header-action-btn" onClick={() => void store.refreshAllQuotas()} disabled={store.loading || store.accounts.length === 0}>
            <RefreshCw size={15} className={store.loading ? 'loading-spinner' : ''} />
            <span>{t('common.refresh', 'Refresh all')}</span>
          </button>
        </div>
      </div>

      <section className="opencode-go-summary" aria-live="polite">
        <div><KeyRound size={18} /><span>{t('openCodeGo.connections', 'Configured connections')}</span><strong>{store.accounts.length}</strong></div>
        <code>{providerTab === 'go' ? 'https://opencode.ai/zen/go/v1' : 'https://opencode.ai/zen/v1'}</code>
      </section>

      <div className="opencode-provider-tabs" role="tablist" aria-label={t('openCodeGo.providerTabs', 'OpenCode providers')}>
        {(['go', 'zen'] as const).map((provider) => (
          <button
            key={provider}
            type="button"
            role="tab"
            aria-selected={providerTab === provider}
            className={providerTab === provider ? 'active' : ''}
            onClick={() => { setProviderTab(provider); setSearchQuery(''); setTierFilter('all'); }}
          >
            {provider === 'go' ? t('openCodeGo.tabs.go', 'Go') : t('openCodeGo.tabs.zen', 'Zen')}
          </button>
        ))}
      </div>

      <div className="toolbar opencode-go-toolbar">
        <div className="toolbar-left">
          <div className="search-box">
            <Search className="search-icon" size={16} />
            <input type="search" value={searchQuery} onChange={(event) => setSearchQuery(event.target.value)} placeholder={t('openCodeGo.searchPlaceholder', 'Search connections...')} aria-label={t('openCodeGo.searchLabel', 'Search connections')} />
          </div>
          <div className="view-switcher" aria-label="View">
            <button type="button" className={`view-btn ${viewMode === 'grid' ? 'active' : ''}`} onClick={() => setViewMode('grid')} title={t('openCodeGo.gridView', 'Grid view')} aria-pressed={viewMode === 'grid'}><LayoutGrid size={16} /></button>
            <button type="button" className={`view-btn ${viewMode === 'list' ? 'active' : ''}`} onClick={() => setViewMode('list')} title={t('openCodeGo.listView', 'List view')} aria-pressed={viewMode === 'list'}><List size={16} /></button>
          </div>
          <SingleSelectFilterDropdown
            value={tierFilter}
            options={[
              { value: 'all', label: t('openCodeGo.filters.allTiers', 'All tiers') },
              { value: 'available', label: t('openCodeGo.filters.available', 'Available') },
              { value: 'exhausted', label: t('openCodeGo.filters.exhausted', 'Exhausted') },
              { value: 'error', label: t('openCodeGo.filters.error', 'Error') },
              { value: 'pending', label: t('openCodeGo.filters.pending', 'Pending') },
            ]}
            ariaLabel={t('openCodeGo.filterLabel', 'Filter by tier')}
            onChange={(value) => setTierFilter(value as TierFilter)}
          />
          <SingleSelectFilterDropdown
            value={sortBy}
            options={[
              { value: 'name', label: t('openCodeGo.sort.name', 'Name') },
              { value: 'created_at', label: t('openCodeGo.sort.created', 'Created') },
              { value: 'remaining', label: t('openCodeGo.sort.remaining', 'Remaining quota') },
            ]}
            icon={<ArrowDownWideNarrow size={14} />}
            ariaLabel={t('openCodeGo.sortLabel', 'Sort connections')}
            onChange={(value) => setSortBy(value as SortBy)}
          />
          <button type="button" className="sort-direction-btn opencode-go-sort-direction" onClick={() => setSortDirection((current) => current === 'asc' ? 'desc' : 'asc')} title={t('openCodeGo.toggleSortDirection', 'Toggle sort direction')} aria-label={t('openCodeGo.toggleSortDirection', 'Toggle sort direction')}>{sortDirection === 'desc' ? '⬇' : '⬆'}</button>
        </div>
        {hasActiveFilter && <button type="button" className="btn btn-secondary" onClick={() => { setSearchQuery(''); setTierFilter('all'); }}>{t('openCodeGo.clearFilters', 'Clear filters')}</button>}
      </div>

      {(store.error || actionError) && <div className="opencode-go-empty" role="alert">{store.error ? quotaErrorText(store.error) : actionError}</div>}
      {!store.loaded && store.loading ? (
        <div className="opencode-go-empty">{t('common.loading', 'Loading...')}</div>
      ) : (
        <div className={`opencode-go-connection-grid ${viewMode === 'list' ? 'list' : ''}`}>
          {slots.map(({ connection }, index) => {
            if (!connection) return null;
            const displayName = normalizeOpenCodeGoConnectionName(connection.name, index);
            const tier = resolveOpenCodeGoConnectionTier(connection);
            return (
              <article className="opencode-go-connection-card" key={connection.id}>
                <header>
                  <div>
                    <span className="opencode-go-connection-icon"><KeyRound size={16} /></span>
                    <div><h2>{displayName}</h2><code>{connection.keyHint}</code>{connection.emailHint && <span className="opencode-go-email-hint">{connection.emailHint}</span>}<span className={`opencode-go-tier ${tier}`}>{tier}</span></div>
                  </div>
                  <div className="opencode-go-card-actions">
                    <button type="button" aria-label={t('openCodeGo.refreshConnection', 'Refresh {{name}}', { name: displayName })} onClick={() => void refreshConnection(connection.id)} disabled={!connection.enabled || refreshingConnectionId === connection.id}><RefreshCw size={14} className={refreshingConnectionId === connection.id ? 'loading-spinner' : ''} /></button>
                    <button type="button" aria-label={t('openCodeGo.editConnection', 'Edit {{name}}', { name: displayName })} onClick={() => openEditor(index, connection)} disabled={pendingDeleteId === connection.id}><Pencil size={14} /></button>
                    <button type="button" aria-label={t('openCodeGo.testConnection', 'Test {{name}}', { name: displayName })} onClick={() => void testConnection(connection.id)} disabled={!connection.enabled || testingConnectionId === connection.id}>{testingConnectionId === connection.id ? <RefreshCw size={14} className="loading-spinner" /> : <BadgeCheck size={14} />}</button>
                    <button type="button" className={connection.enabled ? '' : 'is-disabled'} aria-label={connection.enabled ? t('openCodeGo.disableConnection', 'Disable {{name}}', { name: displayName }) : t('openCodeGo.enableConnection', 'Enable {{name}}', { name: displayName })} onClick={() => void store.setConnectionEnabled(connection.id, !connection.enabled)} disabled={pendingDeleteId === connection.id}>{connection.enabled ? t('openCodeGo.disable', 'Disable') : t('openCodeGo.enable', 'Enable')}</button>
                    <button type="button" aria-label={t('openCodeGo.deleteConnection', 'Delete {{name}}', { name: displayName })} onClick={() => setDeleteCandidateId(connection.id)} disabled={pendingDeleteId === connection.id}>{pendingDeleteId === connection.id ? <RefreshCw size={14} className="loading-spinner" /> : <Trash2 size={14} />}</button>
                  </div>
                </header>
                <div className="opencode-go-quota-panel">
                  <OpenCodeGoQuotaWindowCards
                    quota={connection.quota}
                    errors={connection.quotaError ? {
                      rolling: quotaErrorText(connection.quotaError.kind),
                      weekly: quotaErrorText(connection.quotaError.kind),
                      monthly: quotaErrorText(connection.quotaError.kind),
                    } : undefined}
                  />
                  {!connection.enabled && <p className="opencode-go-connection-disabled">{t('openCodeGo.disabledDescription', 'Disabled connections are retained locally and excluded from usage refreshes.')}</p>}
                  {testedConnectionId === connection.id && <p className="opencode-go-connection-tested" role="status">{t('openCodeGo.testSucceeded', 'Connection verified.')}</p>}
                  {connection.quotaError && <p className="opencode-go-quota-error">{quotaErrorText(connection.quotaError.kind)}</p>}
                  {connection.quota?.queriedAt && <span className="opencode-go-updated"><Clock3 size={13} />{new Date(connection.quota.queriedAt * 1000).toLocaleTimeString()}</span>}
                </div>
              </article>
            );
          })}
          {hasActiveFilter && visibleConnections.length === 0 && <div className="opencode-go-empty">{t('openCodeGo.noFilterResults', 'No connections match these filters.')}</div>}
        </div>
      )}

      <OpenCodeGoAddAccountModal
        open={showAddAccount}
        createConnection={async ({ name, apiKey, email }) => {
          return store.createConnection(name, apiKey, email, providerTab);
        }}
        onClose={() => setShowAddAccount(false)}
      />

      {deleteCandidate && (
        <div className="opencode-go-editor-backdrop" role="presentation" onMouseDown={() => !pendingDeleteId && setDeleteCandidateId(null)}>
          <section className="opencode-go-editor" role="alertdialog" aria-modal="true" aria-labelledby="opencode-go-delete-title" onMouseDown={(event) => event.stopPropagation()}>
            <header><h2 id="opencode-go-delete-title">{t('openCodeGo.delete.title', 'Delete connection?')}</h2><button type="button" onClick={() => setDeleteCandidateId(null)} disabled={Boolean(pendingDeleteId)} aria-label={t('common.close', 'Close')}><X size={18} /></button></header>
            <p>{t('openCodeGo.delete.confirm', 'Delete {{name}}? This only removes its local encrypted connection.', { name: deleteCandidate.name || deleteCandidate.emailHint || deleteCandidate.keyHint })}</p>
            <footer><button type="button" className="btn btn-secondary" onClick={() => setDeleteCandidateId(null)} disabled={Boolean(pendingDeleteId)}>{t('common.cancel', 'Cancel')}</button><button type="button" className="btn btn-danger" onClick={() => void deleteConnection()} disabled={Boolean(pendingDeleteId)}>{pendingDeleteId ? t('common.deleting', 'Deleting...') : t('common.delete', 'Delete')}</button></footer>
          </section>
        </div>
      )}

      {editor && (
        <div className="opencode-go-editor-backdrop" role="presentation" onMouseDown={closeEditor}>
          <section className="opencode-go-editor" role="dialog" aria-modal="true" aria-labelledby="opencode-go-editor-title" onMouseDown={(event) => event.stopPropagation()}>
            <header><h2 id="opencode-go-editor-title">{editor.connection ? t('openCodeGo.edit.title', 'Edit connection') : t('openCodeGo.addSlot', 'Add connection {{index}}', { index: editor.slotIndex + 1 })}</h2><button type="button" onClick={closeEditor} aria-label={t('common.close', 'Close')}><X size={18} /></button></header>
            <label>{t('openCodeGo.add.name', 'Connection name')}<input value={name} onChange={(event) => setName(event.target.value)} placeholder={t('openCodeGo.connectionFallback', 'Connection {{index}}', { index: editor.slotIndex + 1 })} /></label>
            <label>{t('openCodeGo.add.email', 'Email')}<input type="email" autoComplete="email" value={email} onChange={(event) => setEmail(event.target.value)} placeholder={t('openCodeGo.edit.keepEmail', 'Leave blank to keep the saved email')} /></label>
            <label>{t('openCodeGo.add.apiKey', 'API key')}<input type="password" autoComplete="off" value={apiKey} onChange={(event) => setApiKey(event.target.value)} placeholder={editor.connection ? t('openCodeGo.edit.keepKey', 'Leave blank to keep the current key') : t('openCodeGo.add.required', 'Required')} /></label>
            {actionError && <p className="opencode-go-quota-error" role="alert">{actionError}</p>}
            <footer><button type="button" className="btn btn-secondary" onClick={closeEditor}>{t('common.cancel', 'Cancel')}</button><button type="button" className="btn btn-primary" disabled={saving || (!editor.connection && !apiKey.trim())} onClick={() => void saveEditor()}>{saving ? t('common.saving', 'Saving...') : t('openCodeGo.edit.save', 'Save connection')}</button></footer>
          </section>
        </div>
      )}
    </main>
  );
}
