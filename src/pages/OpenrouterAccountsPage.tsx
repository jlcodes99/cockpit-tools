import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  Eye,
  EyeOff,
  Globe,
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
  X,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
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
  maskSensitiveValue,
} from '../utils/privacy';

const OPENROUTER_FLOW_NOTICE_COLLAPSED_KEY = 'agtools.openrouter.flow_notice_collapsed';

type ViewMode = 'grid' | 'list';
type TabType = 'overview' | 'models';

export function OpenrouterAccountsPage() {
  const { t } = useTranslation();
  const accounts = useOpenrouterAccountStore((state) => state.accounts);
  const loading = useOpenrouterAccountStore((state) => state.loading);
  const fetchAccounts = useOpenrouterAccountStore((state) => state.fetchAccounts);
  const deleteAccounts = useOpenrouterAccountStore((state) => state.deleteAccounts);
  const refreshToken = useOpenrouterAccountStore((state) => state.refreshToken);
  const refreshAllTokens = useOpenrouterAccountStore((state) => state.refreshAllTokens);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [showAddModal, setShowAddModal] = useState(false);
  const [addKeyValue, setAddKeyValue] = useState('');
  const [isAddingAccount, setIsAddingAccount] = useState(false);
  const [addAccountError, setAddAccountError] = useState('');
  const [privacyMode, setPrivacyMode] = useState(isPrivacyModeEnabledByDefault);
  const [flowNoticeCollapsed, setFlowNoticeCollapsed] = useState(() => {
    try { return localStorage.getItem(OPENROUTER_FLOW_NOTICE_COLLAPSED_KEY) === '1'; } catch { return false; }
  });
  const [searchQuery, setSearchQuery] = useState('');
  const [activeTab, setActiveTab] = useState<TabType>('overview');
  const [viewMode, setViewMode] = useState<ViewMode>('grid');
  const [models, setModels] = useState<OpenRouterModel[]>([]);
  const [modelsLoading, setModelsLoading] = useState(false);
  const [currentPage, setCurrentPage] = useState(1);
  const [tagEditTarget, setTagEditTarget] = useState<OpenRouterAccount | null>(null);
  const pageSize = 20;
  const addInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (showAddModal && addInputRef.current) {
      addInputRef.current.focus();
    }
  }, [showAddModal]);

  useEffect(() => {
    setModelsLoading(true);
    openrouterService.fetchOpenRouterModels()
      .then(setModels)
      .catch(() => {})
      .finally(() => setModelsLoading(false));
  }, []);

  const toggleSelect = useCallback((id: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id); else next.add(id);
      return next;
    });
  }, []);

  const handleDelete = useCallback(async (accountId: string) => {
    try { await deleteAccounts([accountId]); setSelected((prev) => { const next = new Set(prev); next.delete(accountId); return next; }); } catch {}
  }, [deleteAccounts]);

  const handleBatchDelete = useCallback(async () => {
    const ids = Array.from(selected);
    if (ids.length === 0) return;
    try { await deleteAccounts(ids); setSelected(new Set()); } catch {}
  }, [deleteAccounts, selected]);

  const handleAddAccount = useCallback(async () => {
    const key = addKeyValue.trim();
    if (!key) return;
    setIsAddingAccount(true);
    setAddAccountError('');
    try {
      await openrouterService.addOpenRouterAccount(key);
      setShowAddModal(false);
      setAddKeyValue('');
      await fetchAccounts();
    } catch (error) {
      setAddAccountError(String(error));
    } finally {
      setIsAddingAccount(false);
    }
  }, [addKeyValue, fetchAccounts]);

  const handleSaveTags = useCallback(async (tags: string[]) => {
    if (!tagEditTarget) return;
    try {
      await openrouterService.updateOpenRouterAccountTags(tagEditTarget.id, tags);
      await fetchAccounts();
      setTagEditTarget(null);
    } catch {}
  }, [tagEditTarget, fetchAccounts]);

  const handleRefresh = useCallback(async (accountId: string) => {
    try { await refreshToken(accountId); } catch {}
  }, [refreshToken]);

  const handleRefreshAll = useCallback(async () => {
    try { await refreshAllTokens(); } catch {}
  }, [refreshAllTokens]);

  const filteredAccounts = useMemo(() => {
    let result = [...accounts];
    if (searchQuery.trim()) {
      const q = searchQuery.trim().toLowerCase();
      result = result.filter(
        (a) =>
          getOpenRouterAccountDisplayEmail(a).toLowerCase().includes(q) ||
          (a.label || '').toLowerCase().includes(q),
      );
    }
    return result;
  }, [accounts, searchQuery]);

  const totalPages = Math.max(1, Math.ceil(filteredAccounts.length / pageSize));
  const paginatedItems = filteredAccounts.slice((currentPage - 1) * pageSize, currentPage * pageSize);
  const freeModelCount = useMemo(() => models.filter((m) => m.is_free).length, [models]);

  const renderAccountCard = (account: OpenRouterAccount) => {
    const isSelected = selected.has(account.id);
    const isMgmt = isOpenRouterManagementKey(account);
    const creditsInfo = getOpenRouterCreditsInfo(account);
    const displayEmail = getOpenRouterAccountDisplayEmail(account);
    const accountTags = (account.tags || []).map((t) => t.trim()).filter(Boolean);
    const visibleTags = accountTags.slice(0, 2);
    const moreTagCount = Math.max(0, accountTags.length - visibleTags.length);
    const pct = getOpenRouterUsagePercent(account);

    return (
      <div key={account.id} className={`ghcp-account-card ${isSelected ? 'selected' : ''}`} onClick={() => toggleSelect(account.id)}>
        <div className="card-top">
          <div className="card-select">
            <input type="checkbox" checked={isSelected} onChange={(e) => { e.stopPropagation(); toggleSelect(account.id); }} />
          </div>
          <span className="account-email" title={displayEmail}>
            {maskSensitiveValue(displayEmail, privacyMode)}
          </span>
          <span className={`tier-badge ${getOpenRouterPlanBadgeClass(account)}`}>{getOpenRouterPlanBadge(account)}</span>
        </div>

        <div className="account-sub-line">
          {account.key_type === 'management' ? t('openrouter.keyTypeManagement', 'Management') :
           account.key_type === 'provisioning' ? t('openrouter.keyTypeProvisioning', 'Provisioning') :
           t('openrouter.keyTypeApi', 'API')}
          {account.label && <>{' · '}{account.label}</>}
        </div>

        {accountTags.length > 0 && (
          <div className="card-tags">
            {visibleTags.map((tag, idx) => (
              <span key={`${account.id}-${tag}-${idx}`} className="tag-pill">{tag}</span>
            ))}
            {moreTagCount > 0 && <span className="tag-pill more">+{moreTagCount}</span>}
          </div>
        )}

        {account.usage != null || account.usage_daily != null || account.usage_weekly != null || account.usage_monthly != null ? (
          <div className="opencode-quota-section">
            {account.usage != null && (
              <div className="quota-item windsurf-credit-item">
                <div className="quota-header">
                  <span className="quota-label">Total Usage</span>
                  <span className={`quota-pct ${pct != null ? (pct >= 90 ? 'critical' : pct >= 70 ? 'warning' : 'high') : ''}`}>
                    ${account.usage.toFixed(4)}
                    {account.limit != null ? ` / $${account.limit.toFixed(2)}` : ''}
                  </span>
                </div>
                {pct != null && (
                  <div className="quota-bar-track">
                    <div className={`quota-bar ${pct >= 90 ? 'critical' : pct >= 70 ? 'warning' : 'high'}`} style={{ width: `${Math.min(pct, 100)}%` }} />
                  </div>
                )}
                {account.limit_remaining != null && (
                  <div className="windsurf-credit-meta-row">
                    <span className="windsurf-credit-left">Remaining: ${account.limit_remaining.toFixed(4)}</span>
                  </div>
                )}
              </div>
            )}
            {account.usage_daily != null && (
              <div className="quota-item windsurf-credit-item">
                <div className="quota-header">
                  <span className="quota-label">Daily</span>
                  <span className="quota-pct">${account.usage_daily.toFixed(4)}</span>
                </div>
              </div>
            )}
            {account.usage_weekly != null && (
              <div className="quota-item windsurf-credit-item">
                <div className="quota-header">
                  <span className="quota-label">Weekly</span>
                  <span className="quota-pct">${account.usage_weekly.toFixed(4)}</span>
                </div>
              </div>
            )}
            {account.usage_monthly != null && (
              <div className="quota-item windsurf-credit-item">
                <div className="quota-header">
                  <span className="quota-label">Monthly</span>
                  <span className="quota-pct">${account.usage_monthly.toFixed(4)}</span>
                </div>
              </div>
            )}
          </div>
        ) : (
          <div className="opencode-quota-section">
            <div className="quota-item windsurf-credit-item">
              <div className="quota-empty" style={{ textAlign: 'center', padding: '8px 0', fontSize: '12px', color: 'var(--text-muted)' }}>
                {t('openrouter.usage.noData', 'No usage data')}
              </div>
            </div>
          </div>
        )}

        {isMgmt && (
          <div className="opencode-quota-section" style={{ marginTop: 0 }}>
            <div className="quota-item windsurf-credit-item" style={{ border: 'none', padding: '4px 0' }}>
              <span style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
                <button className="card-action-btn" onClick={async (e) => { e.stopPropagation(); try { await openrouterService.fetchOpenRouterCredits(account.id); await refreshToken(account.id); } catch {} }} style={{ fontSize: '11px', padding: '2px 8px' }}>
                  {t('openrouter.credits.check', 'Check Credits')}
                </button>
                {creditsInfo?.total_credits != null && (
                  <span style={{ fontSize: '12px', color: 'var(--text-secondary)' }}>
                    {t('openrouter.credits.total', 'Credits')}: {formatOpenRouterCredits(creditsInfo.total_credits)}
                  </span>
                )}
              </span>
            </div>
          </div>
        )}

        <div className="card-footer">
          <span className="card-date">{new Date(account.created_at * 1000).toLocaleDateString()}</span>
          <div className="card-actions">
            <button className="card-action-btn success" onClick={async (e) => { e.stopPropagation(); try { await openrouterService.injectOpenRouterAccount(account.id); } catch {} }} title={t('common.shared.inject', '')}>
              <Play size={14} />
            </button>
            <button className="card-action-btn" onClick={(e) => { e.stopPropagation(); setTagEditTarget(account); }} title={t('common.shared.editTags', '')}>
              <Tag size={14} />
            </button>
            <button className="card-action-btn" onClick={async (e) => { e.stopPropagation(); await handleRefresh(account.id); }} title={t('common.shared.refresh', '')}>
              <RefreshCw size={14} />
            </button>
            <button className="card-action-btn danger" onClick={async (e) => { e.stopPropagation(); await handleDelete(account.id); }} title={t('common.shared.delete', '')}>
              <Trash2 size={14} />
            </button>
          </div>
        </div>
      </div>
    );
  };

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
          <button className={`tab-btn ${activeTab === 'overview' ? 'active' : ''}`} onClick={() => setActiveTab('overview')}>Overview</button>
          <button className={`tab-btn ${activeTab === 'models' ? 'active' : ''}`} onClick={() => setActiveTab('models')}>Models</button>
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
              <button className="flow-notice-close" onClick={() => { setFlowNoticeCollapsed(true); try { localStorage.setItem(OPENROUTER_FLOW_NOTICE_COLLAPSED_KEY, '1'); } catch {} }}><X size={14} /></button>
            </div>
          )}

          <div className="accounts-toolbar">
            <div className="toolbar-left">
              <button className="btn btn-primary btn-sm" onClick={() => setShowAddModal(true)}><Plus size={16} /><span>{t('common.shared.addAccount', 'Add Account')}</span></button>
              <button className="btn btn-secondary btn-sm" onClick={handleBatchDelete} disabled={selected.size === 0}><Trash2 size={16} /><span>{t('common.shared.deleteSelected', 'Delete Selected')}</span></button>
              <button className="btn btn-secondary btn-sm" onClick={handleRefreshAll}><RefreshCw size={16} /><span>{t('common.shared.refreshAll', 'Refresh All')}</span></button>

            </div>
            <div className="toolbar-right">
              <div className="search-box">
                <Search size={14} />
                <input type="text" className="input search-input" placeholder={t('common.shared.search', 'Search...')} value={searchQuery} onChange={(e) => setSearchQuery(e.target.value)} />
              </div>
              <button className="btn btn-secondary icon-only" onClick={() => setPrivacyMode(!privacyMode)} title={privacyMode ? t('privacy.showSensitive', '') : t('privacy.hideSensitive', '')}>
                {privacyMode ? <EyeOff size={14} /> : <Eye size={14} />}
              </button>
            </div>
          </div>

          {loading && accounts.length === 0 ? (
            <div className="loading-container"><RefreshCw size={24} className="loading-spinner" /><p>{t('common.loading', 'Loading...')}</p></div>
          ) : accounts.length === 0 ? (
            <div className="empty-state">
              <Globe size={48} />
              <h3>{t('common.shared.empty.title', 'No Accounts')}</h3>
              <p>{t('openrouter.empty.description', 'Add your OpenRouter API key to get started.')}</p>
              <button className="btn btn-primary" onClick={() => setShowAddModal(true)}>
                <Plus size={16} />{t('common.shared.addAccount', 'Add Account')}
              </button>
            </div>
          ) : filteredAccounts.length === 0 ? (
            <div className="empty-state">
              <h3>{t('common.shared.noMatch.title', 'No matches')}</h3>
              <p>{t('common.shared.noMatch.desc', 'Try adjusting your search.')}</p>
            </div>
          ) : viewMode === 'grid' ? (
            <div className="ghcp-accounts-grid">{paginatedItems.map(renderAccountCard)}</div>
          ) : (
            <div className="accounts-list">
              <div className="account-table-container">
                <table className="account-table">
                  <thead>
                    <tr>
                      <th style={{ width: 40 }}><input type="checkbox" checked={selected.size === filteredAccounts.length && filteredAccounts.length > 0} onChange={() => setSelected((prev) => prev.size === filteredAccounts.length ? new Set() : new Set(filteredAccounts.map((a) => a.id)))} /></th>
                      <th>{t('common.shared.account', 'Account')}</th>
                      <th>{t('common.shared.plan', 'Plan')}</th>
                      <th>{t('openrouter.keyTypeLabel', 'Key Type')}</th>
                      <th>{t('common.shared.actions', 'Actions')}</th>
                    </tr>
                  </thead>
                  <tbody>
                    {paginatedItems.map((account) => (
                      <tr key={account.id} className={selected.has(account.id) ? 'account-row-selected' : ''} onClick={() => toggleSelect(account.id)}>
                        <td><input type="checkbox" checked={selected.has(account.id)} onChange={(e) => { e.stopPropagation(); toggleSelect(account.id); }} /></td>
                        <td>{maskSensitiveValue(getOpenRouterAccountDisplayEmail(account), privacyMode)}</td>
                        <td><span className={`plan-badge ${getOpenRouterPlanBadgeClass(account)}`}>{getOpenRouterPlanBadge(account)}</span></td>
                        <td>{account.key_type}</td>
                        <td className="sticky-action-cell">
                          <button className="btn btn-icon" onClick={(e) => { e.stopPropagation(); setTagEditTarget(account); }}><Tag size={14} /></button>
                          <button className="btn btn-icon" onClick={async (e) => { e.stopPropagation(); try { await openrouterService.injectOpenRouterAccount(account.id); } catch {} }}><Play size={14} /></button>
                          <button className="btn btn-icon" onClick={async (e) => { e.stopPropagation(); await handleRefresh(account.id); }}><RefreshCw size={14} /></button>
                          <button className="btn btn-icon btn-icon-danger" onClick={async (e) => { e.stopPropagation(); await handleDelete(account.id); }}><Trash2 size={14} /></button>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </div>
          )}

          {totalPages > 1 && (
            <div className="pagination-controls">
              {Array.from({ length: totalPages }, (_, i) => i + 1).map((p) => (
                <button key={p} className={`btn btn-sm ${p === currentPage ? 'btn-primary' : 'btn-secondary'}`} onClick={() => setCurrentPage(p)}>{p}</button>
              ))}
            </div>
          )}

          {tagEditTarget && (
            <div className="modal-overlay" onClick={() => setTagEditTarget(null)}>
              <div className="modal tag-edit-modal" onClick={(e) => e.stopPropagation()}>
                <h3>{t('common.shared.editTags', 'Edit Tags')}</h3>
                <input
                  className="input"
                  placeholder={t('common.shared.tagsPlaceholder', 'Enter tags separated by commas')}
                  defaultValue={(tagEditTarget.tags ?? []).join(', ')}
                  id="tag-edit-input"
                />
                <div className="modal-actions">
                  <button className="btn btn-secondary" onClick={() => setTagEditTarget(null)}>{t('common.cancel', 'Cancel')}</button>
                  <button className="btn btn-primary" onClick={() => {
                    const input = document.getElementById('tag-edit-input') as HTMLInputElement;
                    const tags = input.value.split(',').map((s) => s.trim()).filter(Boolean);
                    handleSaveTags(tags);
                  }}>{t('common.save', 'Save')}</button>
                </div>
              </div>
            </div>
          )}

          {showAddModal && (
            <div className="modal-overlay" onClick={() => setShowAddModal(false)}>
              <div className="modal add-account-modal" onClick={(e) => e.stopPropagation()}>
                <div className="modal-header">
                  <h3>{t('openrouter.addAccount.title', 'Connect OpenRouter')}</h3>
                  <p className="add-account-desc">
                    {t('openrouter.addAccount.desc', 'Enter your OpenRouter API key to add an account.')}
                  </p>
                </div>

                <div className="modal-body">
                  <div className="single-input-section">
                    <label>{t('openrouter.addAccount.apiKey', 'API Key')}</label>
                    <div className="token-input-wrap">
                      <input
                        ref={addInputRef}
                        type={privacyMode ? 'password' : 'text'}
                        className="input token-input"
                        placeholder="sk-or-v1-..."
                        value={addKeyValue}
                        onChange={(e) => setAddKeyValue(e.target.value)}
                        disabled={isAddingAccount}
                      />
                    </div>
                  </div>
                </div>

                {addAccountError && (
                  <div className="modal-message error">{addAccountError}</div>
                )}

                <div className="modal-actions">
                  <button className="btn btn-secondary" onClick={() => { setShowAddModal(false); setAddKeyValue(''); setAddAccountError(''); }} disabled={isAddingAccount}>
                    {t('common.cancel', 'Cancel')}
                  </button>
                  <button className="btn btn-primary" onClick={handleAddAccount} disabled={isAddingAccount || !addKeyValue.trim()}>
                    {isAddingAccount ? t('common.saving', 'Saving...') : t('common.shared.addAccount', 'Add Account')}
                  </button>
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
