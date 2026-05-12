import { useCallback, useEffect, useMemo, useState } from 'react';
import {
  Database,
  EyeOff,
  KeyRound,
  LayoutGrid,
  List,
  Play,
  Plus,
  RefreshCw,
  RotateCw,
  Search,
  Tag,
  Trash2,
  Upload,
  X,
} from 'lucide-react';
import { confirm as confirmDialog } from '@tauri-apps/plugin-dialog';
import { useTranslation } from 'react-i18next';
import { ModalErrorMessage } from '../components/ModalErrorMessage';
import { useOpenrouterAccountStore } from '../stores/useOpenrouterAccountStore';
import * as openrouterService from '../services/openrouterService';
import {
  OpenRouterAccount,
  getOpenRouterAccountDisplayEmail,
  getOpenRouterPlanBadge,
  getOpenRouterPlanBadgeClass,
  getOpenRouterUsagePercent,
  isOpenRouterManagementKey,
  formatOpenRouterCredits,
  getOpenRouterCreditsInfo,
  OpenRouterModel,
} from '../types/openrouter';
import {
  isPrivacyModeEnabledByDefault,
  persistPrivacyModeEnabled,
} from '../utils/privacy';

const OPENROUTER_FLOW_NOTICE_COLLAPSED_KEY = 'agtools.openrouter.flow_notice_collapsed';
const ADD_ACCOUNT_INPUT_ID = 'openrouter-add-key-input';

type ViewMode = 'grid' | 'list';
type OpenRouterTab = 'accounts' | 'models';

function isOpenRouterFlowNoticeCollapsed(): boolean {
  try {
    return localStorage.getItem(OPENROUTER_FLOW_NOTICE_COLLAPSED_KEY) === '1';
  } catch {
    return false;
  }
}

function persistOpenRouterFlowNoticeCollapsed(collapsed: boolean) {
  try {
    if (collapsed) {
      localStorage.setItem(OPENROUTER_FLOW_NOTICE_COLLAPSED_KEY, '1');
    } else {
      localStorage.removeItem(OPENROUTER_FLOW_NOTICE_COLLAPSED_KEY);
    }
  } catch {}
}

export function OpenrouterAccountsPage() {
  const { t } = useTranslation();
  const accounts = useOpenrouterAccountStore((state) => state.accounts);
  const loading = useOpenrouterAccountStore((state) => state.loading);
  const fetchAccounts = useOpenrouterAccountStore((state) => state.fetchAccounts);
  const deleteAccounts = useOpenrouterAccountStore((state) => state.deleteAccounts);
  const refreshToken = useOpenrouterAccountStore((state) => state.refreshToken);
  const refreshAllTokens = useOpenrouterAccountStore((state) => state.refreshAllTokens);
  const importFromJson = useOpenrouterAccountStore((state) => state.importFromJson);
  const exportAccounts = useOpenrouterAccountStore((state) => state.exportAccounts);

  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [showAddForm, setShowAddForm] = useState(false);
  const [addKeyValue, setAddKeyValue] = useState('');
  const [isAddingAccount, setIsAddingAccount] = useState(false);
  const [addAccountError, setAddAccountError] = useState('');
  const [privacyMode, setPrivacyMode] = useState(isPrivacyModeEnabledByDefault);
  const [flowNoticeCollapsed, setFlowNoticeCollapsed] = useState(isOpenRouterFlowNoticeCollapsed());
  const [tagEditTarget, setTagEditTarget] = useState<OpenRouterAccount | null>(null);
  const [editTagId, setEditTagId] = useState<string | null>(null);
  const [editTags, setEditTags] = useState<string[]>([]);
  const [searchQuery, setSearchQuery] = useState('');
  const [activeTab, setActiveTab] = useState<OpenRouterTab>('accounts');
  const [viewMode, setViewMode] = useState<ViewMode>('grid');
  const [models, setModels] = useState<OpenRouterModel[]>([]);
  const [modelsLoading, setModelsLoading] = useState(false);
  const [addFormKeyType, setAddFormKeyType] = useState<'api' | 'management' | 'provisioning'>('api');
  const [currentPage, setCurrentPage] = useState(1);
  const pageSize = 20;

  useEffect(() => {
    setModelsLoading(true);
    openrouterService
      .fetchOpenRouterModels()
      .then((fetched) => setModels(fetched))
      .catch(() => {})
      .finally(() => setModelsLoading(false));
  }, []);

  const handleToggleAccount = useCallback((accountId: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(accountId)) next.delete(accountId);
      else next.add(accountId);
      return next;
    });
  }, []);

  const handleDeleteSingle = useCallback(
    async (accountId: string) => {
      const confirmed = await confirmDialog(
        t('common.shared.delete.confirmMessage', '确定要删除此账号吗？'),
        { title: t('common.shared.delete.title', '删除账号'), kind: 'warning' },
      );
      if (!confirmed) return;
      try {
        await deleteAccounts([accountId]);
        setSelected((prev) => { const next = new Set(prev); next.delete(accountId); return next; });
      } catch (error) {
        console.error('Failed to delete account:', error);
      }
    },
    [deleteAccounts, t],
  );

  const handleDeleteSelected = useCallback(async () => {
    const ids = Array.from(selected);
    if (ids.length === 0) return;
    const confirmed = await confirmDialog(
      t('common.shared.deleteMultiple.confirmMessage', {
        defaultValue: '确定要删除选中的 {{count}} 个账号吗？',
        count: ids.length,
      }),
      { title: t('common.shared.delete.title', '删除账号'), kind: 'warning' },
    );
    if (!confirmed) return;
    try {
      await deleteAccounts(ids);
      setSelected(new Set());
    } catch (error) {
      console.error('Failed to delete accounts:', error);
    }
  }, [deleteAccounts, selected, t]);

  const handleAddAccount = useCallback(async () => {
    const key = addKeyValue.trim();
    if (!key) return;
    setIsAddingAccount(true);
    setAddAccountError('');
    try {
      await openrouterService.addOpenRouterAccount(key);
      setAddKeyValue('');
      setShowAddForm(false);
      setAddFormKeyType('api');
      await fetchAccounts();
    } catch (error) {
      setAddAccountError(String(error));
    } finally {
      setIsAddingAccount(false);
    }
  }, [addKeyValue, fetchAccounts]);

  const handleInject = useCallback(async (accountId: string) => {
    try { await openrouterService.injectOpenRouterAccount(accountId); } catch (error) { console.error(error); }
  }, []);

  const handleExport = useCallback(async () => {
    try {
      const ids = Array.from(selected);
      const json = ids.length > 0 ? await exportAccounts(ids) : await exportAccounts(accounts.map(a => a.id));
      const blob = new Blob([json], { type: 'application/json' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url; a.download = 'openrouter_accounts.json'; a.click();
      URL.revokeObjectURL(url);
    } catch {}
  }, [accounts, exportAccounts, selected]);

  const handleImport = useCallback(async () => {
    const input = document.createElement('input');
    input.type = 'file'; input.accept = '.json';
    input.onchange = async () => {
      const file = input.files?.[0];
      if (file) {
        const text = await file.text();
        const result = await importFromJson(text);
        if (result.length > 0) await fetchAccounts();
      }
    };
    input.click();
  }, [fetchAccounts, importFromJson]);

  const handleSaveTags = useCallback(async (tags: string[]) => {
    if (!editTagId) return;
    try {
      await openrouterService.updateOpenRouterAccountTags(editTagId, tags);
      await fetchAccounts();
      setEditTagId(null);
      setTagEditTarget(null);
    } catch {}
  }, [editTagId, fetchAccounts]);

  const filteredAccounts = useMemo(() => {
    let result = [...accounts];
    if (searchQuery.trim()) {
      const q = searchQuery.trim().toLowerCase();
      result = result.filter(
        (a) =>
          a.id.toLowerCase().includes(q) ||
          getOpenRouterAccountDisplayEmail(a).toLowerCase().includes(q) ||
          (a.label || '').toLowerCase().includes(q),
      );
    }
    return result;
  }, [accounts, searchQuery]);

  const totalPages = Math.max(1, Math.ceil(filteredAccounts.length / pageSize));
  const paginatedItems = filteredAccounts.slice((currentPage - 1) * pageSize, currentPage * pageSize);

  const renderUsageBar = (account: OpenRouterAccount) => {
    const pct = getOpenRouterUsagePercent(account);
    if (pct == null) {
      return <div className="usage-bar-container"><span className="text-muted">{t('openrouter.usage.noData', 'No data')}</span></div>;
    }
    const colorClass = pct >= 90 ? 'usage-bar-danger' : pct >= 70 ? 'usage-bar-warning' : 'usage-bar-ok';
    return (
      <div className="usage-bar-container">
        <div className="usage-bar"><div className={`usage-bar-fill ${colorClass}`} style={{ width: `${pct}%` }} /></div>
        <span className="usage-bar-label">{pct.toFixed(1)}%</span>
      </div>
    );
  };

  const renderAccountCard = (account: OpenRouterAccount) => {
    const isSelected = selected.has(account.id);
    const isMgmt = isOpenRouterManagementKey(account);
    const creditsInfo = getOpenRouterCreditsInfo(account);

    return (
      <div key={account.id} className={`account-card ${isSelected ? 'account-card-selected' : ''}`} onClick={() => handleToggleAccount(account.id)}>
        <div className="account-card-header">
          <div className="account-card-checkbox"><input type="checkbox" checked={isSelected} onChange={() => handleToggleAccount(account.id)} /></div>
          <div className="account-card-info">
            <div className="account-card-email">{getOpenRouterAccountDisplayEmail(account)}</div>
            {account.label && <div className="account-card-label">{account.label}</div>}
          </div>
          <div className="account-card-actions">
            <span className={`plan-badge ${getOpenRouterPlanBadgeClass(account)}`}>{getOpenRouterPlanBadge(account)}</span>
            <button className="btn btn-icon" onClick={(e) => { e.stopPropagation(); setTagEditTarget(account); setEditTagId(account.id); setEditTags(account.tags ?? []); }} title={t('common.shared.editTags', '')}><Tag size={14} /></button>
            <button className="btn btn-icon" onClick={async (e) => { e.stopPropagation(); await handleInject(account.id); }} title={t('common.shared.inject', '')}><Play size={14} /></button>
            <button className="btn btn-icon" onClick={async (e) => { e.stopPropagation(); try { await refreshToken(account.id); } catch {} }} title={t('common.shared.refresh', '')}><RefreshCw size={14} /></button>
            <button className="btn btn-icon btn-icon-danger" onClick={async (e) => { e.stopPropagation(); await handleDeleteSingle(account.id); }} title={t('common.shared.delete', '')}><Trash2 size={14} /></button>
          </div>
        </div>
        <div className="account-card-details">
          <div className="account-card-detail-row"><span className="detail-label">{t('openrouter.keyType', 'Key Type')}</span><span className="detail-value">{account.key_type}</span></div>
          <div className="account-card-detail-row"><span className="detail-label">{t('openrouter.usage.used', 'Usage')}</span><span className="detail-value">{account.usage != null ? `$${account.usage.toFixed(4)}` : '—'}</span></div>
          <div className="account-card-detail-row"><span className="detail-label">{t('openrouter.limit', 'Limit')}</span><span className="detail-value">{account.limit != null ? `$${account.limit.toFixed(2)}` : '—'}</span></div>
          {renderUsageBar(account)}
        </div>
        {(account.usage_daily != null || account.usage_weekly != null || account.usage_monthly != null) && (
          <div className="account-card-usage-breakdown">
            {account.usage_daily != null && <span className="usage-breakdown-item">D: ${account.usage_daily.toFixed(4)}</span>}
            {account.usage_weekly != null && <span className="usage-breakdown-item">W: ${account.usage_weekly.toFixed(4)}</span>}
            {account.usage_monthly != null && <span className="usage-breakdown-item">M: ${account.usage_monthly.toFixed(4)}</span>}
          </div>
        )}
        {isMgmt && creditsInfo && (
          <div className="account-card-credits">
            <button className="btn btn-secondary btn-sm" onClick={async (e) => { e.stopPropagation(); try { await openrouterService.fetchOpenRouterCredits(account.id); await refreshToken(account.id); } catch {} }}>
              {t('openrouter.credits.check', 'Check Credits')}
            </button>
            {creditsInfo.total_credits != null && <span className="credits-label">{t('openrouter.credits.total', 'Credits')}: {formatOpenRouterCredits(creditsInfo.total_credits)}</span>}
          </div>
        )}
        {account.tags && account.tags.length > 0 && (
          <div className="account-card-tags">{account.tags.map((tag) => <span key={tag} className="tag-badge">{tag}</span>)}</div>
        )}
      </div>
    );
  };

  const freeModelCount = useMemo(() => models.filter((m) => m.is_free).length, [models]);

  return (
    <div className="provider-accounts-page openrouter-page" data-page="openrouter">
      <div className="provider-page-header">
        <div className="provider-page-title-row">
          <h2>{t('nav.openrouter', 'OpenRouter')}</h2>
          <div className="header-actions">
            <button className="btn btn-secondary btn-sm" onClick={() => setViewMode(viewMode === 'grid' ? 'list' : 'grid')}>
              {viewMode === 'grid' ? <List size={16} /> : <LayoutGrid size={16} />}
            </button>
            <button className="btn btn-secondary btn-sm" onClick={async () => { setModelsLoading(true); try { setModels(await openrouterService.fetchOpenRouterModels()); } catch {} finally { setModelsLoading(false); } }}>
              <RotateCw size={16} />
            </button>
          </div>
        </div>
        <div className="tab-bar">
          <button className={`tab-btn ${activeTab === 'accounts' ? 'active' : ''}`} onClick={() => setActiveTab('accounts')}>{t('openrouter.tabs.accounts', 'Accounts')}</button>
          <button className={`tab-btn ${activeTab === 'models' ? 'active' : ''}`} onClick={() => setActiveTab('models')}>{t('openrouter.tabs.models', 'Models')}</button>
        </div>
      </div>

      {activeTab === 'models' ? (
        <div className="models-tab">
          <div className="models-header">
            <h3>{t('openrouter.models.title', 'Available Models')}</h3>
            <span className="models-count">{modelsLoading ? t('common.loading', 'Loading...') : t('openrouter.models.count', { defaultValue: '{{total}} models ({{free}} free)', total: models.length, free: freeModelCount })}</span>
          </div>
          {models.length > 0 && (
            <div className="models-grid">
              {models.slice(0, 50).map((model) => (
                <div key={model.id} className="model-card">
                  <div className="model-card-header">
                    <span className="model-card-id">{model.id}</span>
                    {model.is_free && <span className="plan-badge badge-free">FREE</span>}
                  </div>
                  <div className="model-card-details">
                    <span className="model-card-context">{model.context_length > 0 ? `${(model.context_length / 1000).toFixed(0)}K ctx` : '—'}</span>
                    <span className="model-card-pricing">{model.pricing.prompt !== '0' ? `$${model.pricing.prompt}/1K prompt` : 'Free'}</span>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      ) : (
        <>
          {!flowNoticeCollapsed && (
            <div className="flow-notice">
              <div className="flow-notice-content"><KeyRound size={16} /><span>{t('openrouter.flowNotice', 'Enter your OpenRouter API key to add an account.')}</span></div>
              <button className="flow-notice-close" onClick={() => { setFlowNoticeCollapsed(true); persistOpenRouterFlowNoticeCollapsed(true); }}><X size={14} /></button>
            </div>
          )}

          {showAddForm && (
            <div className="add-account-panel">
              <div className="add-account-form">
                <div className="add-account-row">
                  <input id={ADD_ACCOUNT_INPUT_ID} type={privacyMode ? 'password' : 'text'} className="input" placeholder={t('openrouter.addKeyPlaceholder', 'sk-or-v1-...')} value={addKeyValue} onChange={(e) => setAddKeyValue(e.target.value)} disabled={isAddingAccount} />
                  <select className="input" value={addFormKeyType} onChange={(e) => setAddFormKeyType(e.target.value as 'api' | 'management' | 'provisioning')} disabled={isAddingAccount}>
                    <option value="api">API</option>
                    <option value="management">Management</option>
                    <option value="provisioning">Provisioning</option>
                  </select>
                  <button className="btn btn-primary" onClick={handleAddAccount} disabled={isAddingAccount || !addKeyValue.trim()}>
                    {isAddingAccount ? t('common.saving', 'Saving...') : t('common.shared.addAccount', 'Add Account')}
                  </button>
                  <button className="btn btn-secondary" onClick={() => { setShowAddForm(false); setAddKeyValue(''); setAddAccountError(''); }} disabled={isAddingAccount}>
                    {t('common.cancel', 'Cancel')}
                  </button>
                </div>
                <div className="add-account-meta">
                  <label className="privacy-toggle">
                    <input type="checkbox" checked={privacyMode} onChange={(e) => { setPrivacyMode(e.target.checked); persistPrivacyModeEnabled(e.target.checked); }} />
                    <EyeOff size={14} /><span>{t('common.shared.hideKey', 'Hide key')}</span>
                  </label>
                </div>
                {addAccountError && <ModalErrorMessage message={addAccountError} />}
              </div>
            </div>
          )}

          <div className="accounts-toolbar">
            <div className="toolbar-left">
              <button className="btn btn-primary btn-sm" onClick={() => setShowAddForm((prev) => !prev)}><Plus size={16} /><span>{t('common.shared.addAccount', 'Add Account')}</span></button>
              <button className="btn btn-secondary btn-sm" onClick={handleDeleteSelected} disabled={selected.size === 0}><Trash2 size={16} /><span>{t('common.shared.deleteSelected', 'Delete Selected')}</span></button>
              <button className="btn btn-secondary btn-sm" onClick={refreshAllTokens}><RefreshCw size={16} /><span>{t('common.shared.refreshAll', 'Refresh All')}</span></button>
              <button className="btn btn-secondary btn-sm" onClick={handleExport}><Upload size={16} /><span>{t('common.shared.export', 'Export')}</span></button>
              <button className="btn btn-secondary btn-sm" onClick={handleImport}><Trash2 size={16} /><span>{t('common.shared.import', 'Import')}</span></button>
            </div>
            <div className="toolbar-right">
              <div className="search-box">
                <Search size={14} />
                <input type="text" className="input search-input" placeholder={t('common.shared.search', 'Search...')} value={searchQuery} onChange={(e) => setSearchQuery(e.target.value)} />
              </div>
            </div>
          </div>

          <div className="accounts-section">
            {filteredAccounts.length === 0 && !loading && (
              <div className="empty-state">
                <Database size={32} />
                <p>{t('common.shared.empty.title', 'No Accounts')}</p>
                {!showAddForm && <button className="btn btn-primary" onClick={() => setShowAddForm(true)}>{t('common.shared.addAccount', 'Add Account')}</button>}
              </div>
            )}
            {loading && filteredAccounts.length === 0 && <div className="loading-state">{t('common.loading', 'Loading...')}</div>}
            {filteredAccounts.length > 0 && (
              <div className={viewMode === 'grid' ? 'accounts-grid' : ''}>
                {paginatedItems.map((account) => viewMode === 'grid' ? renderAccountCard(account) : null)}
              </div>
            )}
            {totalPages > 1 && (
              <div className="pagination-controls">
                {Array.from({ length: totalPages }, (_, i) => i + 1).map((p) => (
                  <button key={p} className={`btn btn-sm ${p === currentPage ? 'btn-primary' : 'btn-secondary'}`} onClick={() => setCurrentPage(p)}>{p}</button>
                ))}
              </div>
            )}
          </div>

          {tagEditTarget && editTagId && (
            <div className="tag-edit-modal-overlay" onClick={() => { setTagEditTarget(null); setEditTagId(null); }}>
              <div className="tag-edit-modal" onClick={(e) => e.stopPropagation()}>
                <h3>{t('common.shared.editTags', 'Edit Tags')}</h3>
                <input
                  className="input"
                  placeholder={t('common.shared.tagsPlaceholder', 'Enter tags separated by commas')}
                  value={editTags.join(', ')}
                  onChange={(e) => setEditTags(e.target.value.split(',').map((s) => s.trim()).filter(Boolean))}
                />
                <div className="modal-footer">
                  <button className="btn btn-secondary" onClick={() => { setTagEditTarget(null); setEditTagId(null); }}>{t('common.cancel', 'Cancel')}</button>
                  <button className="btn btn-primary" onClick={() => handleSaveTags(editTags)}>{t('common.save', 'Save')}</button>
                </div>
              </div>
            </div>
          )}
        </>
      )}
    </div>
  );
}

export default OpenrouterAccountsPage;
