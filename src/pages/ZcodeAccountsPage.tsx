import { useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  ChevronDown,
  ChevronUp,
  CircleAlert,
  Import,
  RefreshCw,
  RotateCw,
  Search,
  Trash2,
  Upload,
  X,
} from 'lucide-react';
import { ModalErrorMessage } from '../components/ModalErrorMessage';
import {
  PlatformOverviewTab,
  PlatformOverviewTabsHeader,
} from '../components/platform/PlatformOverviewTabsHeader';
import { useProviderAccountsPage } from '../hooks/useProviderAccountsPage';
import { useZcodeAccountStore } from '../stores/useZcodeAccountStore';
import * as zcodeService from '../services/zcodeService';
import {
  ZcodeAccount,
  ZcodeQuotaItem,
  formatZcodeQuotaCount,
  getZcodeAccountDisplayName,
  getZcodePlanBadge,
} from '../types/zcode';

const ZCODE_FLOW_NOTICE_COLLAPSED_KEY = 'agtools.zcode.flow_notice_collapsed';

function getZcodePlanTone(planTier?: string | null): string {
  const normalized = (planTier || '').trim().toLowerCase();
  if (!normalized) return 'unknown';
  if (normalized.includes('max')) return 'enterprise';
  if (normalized.includes('pro')) return 'pro';
  if (normalized.includes('trial') || normalized.includes('体验')) return 'trial';
  return 'unknown';
}

function getQuotaTone(percentUsed: number): 'high' | 'medium' | 'low' {
  const remaining = Math.max(0, Math.min(100, 100 - percentUsed));
  if (remaining >= 50) return 'high';
  if (remaining >= 20) return 'medium';
  return 'low';
}

export function ZcodeAccountsPage() {
  const { t } = useTranslation();
  const store = useZcodeAccountStore();
  const [activeTab, setActiveTab] = useState<PlatformOverviewTab>('overview');
  const [sortDirection, setSortDirection] = useState<'asc' | 'desc'>('desc');
  const jsonImportInputRef = useRef<HTMLInputElement>(null);

  const page = useProviderAccountsPage<ZcodeAccount>({
    platformKey: 'ZCode',
    oauthLogPrefix: 'ZcodeOAuth',
    flowNoticeCollapsedKey: ZCODE_FLOW_NOTICE_COLLAPSED_KEY,
    exportFilePrefix: 'zcode_accounts',
    store: {
      accounts: store.accounts,
      loading: store.loading,
      error: store.error,
      fetchAccounts: store.fetchAccounts,
      deleteAccounts: store.deleteAccounts,
      refreshToken: store.refreshToken,
      refreshAllTokens: store.refreshAllTokens,
      updateAccountTags: store.updateAccountTags,
    },
    dataService: {
      importFromJson: zcodeService.importZcodeFromJson,
      importFromLocal: zcodeService.importZcodeFromLocal,
      exportAccounts: zcodeService.exportZcodeAccounts,
    },
    getDisplayEmail: getZcodeAccountDisplayName,
  });

  const {
    maskAccountText,
    searchQuery,
    setSearchQuery,
    selected,
    toggleSelect,
    refreshing,
    refreshingAll,
    handleRefresh,
    handleRefreshAll,
    handleDelete,
    handleBatchDelete,
    deleteConfirm,
    deleteConfirmError,
    deleteConfirmErrorScrollKey,
    setDeleteConfirm,
    deleting,
    confirmDelete,
    exporting,
    handleExport,
    handleExportByIds,
    formatDate,
  } = page;

  // 从本机 zcode-switch / ZCode 凭证导入（后端快照解析）
  const [importingLocal, setImportingLocal] = useState(false);
  const [importMessage, setImportMessage] = useState<string | null>(null);
  const importFromLocal = async () => {
    if (importingLocal) return;
    setImportingLocal(true);
    setImportMessage(null);
    try {
      const imported = await zcodeService.importZcodeFromLocal();
      await store.fetchAccounts();
      setImportMessage(
        t('common.shared.token.importSuccessMsg', {
          count: imported.length,
          defaultValue: '成功导入 {{count}} 个账号',
        }),
      );
    } catch (error) {
      setImportMessage(
        t('common.shared.import.failedMsg', {
          error: String(error).replace(/^Error:\s*/, ''),
          defaultValue: '导入失败: {{error}}',
        }),
      );
    } finally {
      setImportingLocal(false);
    }
  };

  const accounts = useMemo(() => {
    const list = [...store.accounts];
    list.sort((left, right) => {
      const leftKey = Math.max(left.last_used, left.created_at);
      const rightKey = Math.max(right.last_used, right.created_at);
      return sortDirection === 'desc' ? rightKey - leftKey : leftKey - rightKey;
    });
    const query = searchQuery.trim().toLowerCase();
    if (!query) return list;
    return list.filter((account) =>
      [account.name, account.email, account.display_name, account.plan_tier]
        .filter(Boolean)
        .some((value) => String(value).toLowerCase().includes(query)),
    );
  }, [store.accounts, sortDirection, searchQuery]);

  const filteredIds = useMemo(() => accounts.map((account) => account.id), [accounts]);
  const isAllSelected = filteredIds.length > 0 && filteredIds.every((id) => selected.has(id));

  const renderUsageItem = (item: ZcodeQuotaItem, index: number) => {
    const percentUsed =
      item.percent_used != null && Number.isFinite(item.percent_used)
        ? Math.max(0, Math.min(100, item.percent_used))
        : item.total != null && item.used != null && item.total > 0
          ? Math.max(0, Math.min(100, (item.used / item.total) * 100))
          : null;
    const tone = percentUsed != null ? getQuotaTone(percentUsed) : 'high';
    const usedText = formatZcodeQuotaCount(item.used);
    const totalText = formatZcodeQuotaCount(item.total);
    return (
      <div
        key={`${item.name}-${index}`}
        className="windsurf-official-usage-item zed-usage-metric-item"
      >
        <div className="zed-usage-metric-header">
          <span className="zed-usage-metric-label">{item.name}</span>
          <span className={`zed-usage-metric-value ${tone}`}>
            {percentUsed != null ? `${Math.round(percentUsed)}%` : '--'}
          </span>
        </div>
        <div className="windsurf-credit-meta-row">
          <span className="windsurf-credit-used">{usedText}</span>
          <span className="windsurf-credit-left">{item.period_end || totalText}</span>
        </div>
        <div className="zed-usage-metric-track">
          <div
            className={`zed-usage-metric-bar ${tone}`}
            style={{ width: `${percentUsed ?? 0}%` }}
          />
        </div>
      </div>
    );
  };

  const toggleSelectAllVisible = () => {
    const shouldSelectAll = !isAllSelected;
    filteredIds.forEach((id) => {
      const isSelected = selected.has(id);
      if (shouldSelectAll && !isSelected) {
        toggleSelect(id);
      } else if (!shouldSelectAll && isSelected) {
        toggleSelect(id);
      }
    });
  };

  const handleJsonImportFile = async (event: React.ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    event.target.value = '';
    if (!file) return;
    try {
      const content = await file.text();
      const imported = await zcodeService.importZcodeFromJson(content);
      await store.fetchAccounts();
      setImportMessage(
        t('common.shared.token.importSuccessMsg', {
          count: imported.length,
          defaultValue: '成功导入 {{count}} 个账号',
        }),
      );
    } catch (error) {
      setImportMessage(t('common.shared.import.failedMsg', '导入失败: {{error}}', { error: String(error) }));
    }
  };

  return (
    <div className="ghcp-accounts-page zcode-accounts-page">
      <PlatformOverviewTabsHeader
        platform="zcode"
        active={activeTab}
        onTabChange={setActiveTab}
        tabs={['overview']}
      />

      <details className="ghcp-flow-notice-toggle" open={!page.isFlowNoticeCollapsed}>
        <summary
          className="ghcp-flow-notice-title"
          onClick={(event) => {
            event.preventDefault();
            page.setIsFlowNoticeCollapsed(!page.isFlowNoticeCollapsed);
          }}
        >
          {t('zcode.flowNotice.title', 'ZCode 额度说明（点击展开/收起）')}
        </summary>
        <div className="ghcp-flow-notice-body">
          <div className="ghcp-flow-notice-desc">
            {t(
              'zcode.flowNotice.desc',
              '导入本机已有的 ZCode 账号并展示各账号的 Coding/Start Plan 额度。账号来源：zcode-switch 快照（~/.zcode-switch/accounts）或本机已登录的 ZCode 凭证（~/.zcode/v2/credentials.json）。',
            )}
          </div>
          <ul className="ghcp-flow-notice-list">
            <li>
              {t(
                'zcode.flowNotice.permission',
                '权限范围：只读取本机 ZCode 凭证文件（支持 enc:v1 加密值解密），不写入、不修改 ZCode 配置。',
              )}
            </li>
            <li>
              {t(
                'zcode.flowNotice.network',
                '网络范围：额度查询直连官方接口（open.bigmodel.cn / zcode.z.ai），不上传本地数据。',
              )}
            </li>
            <li>{t('zcode.flowNotice.noSwitch', '当前版本仅支持额度展示，不支持在应用内切换 ZCode 账号。')}</li>
          </ul>
        </div>
      </details>

      <div className="toolbar">
        <div className="toolbar-left">
          <div className="search-box">
            <Search size={16} className="search-icon" />
            <input
              type="text"
              placeholder={t('zcode.search', '搜索 ZCode 账号...')}
              value={searchQuery}
              onChange={(event) => setSearchQuery(event.target.value)}
            />
          </div>
          <button
            type="button"
            className="sort-direction-btn"
            onClick={() => setSortDirection((current) => (current === 'desc' ? 'asc' : 'desc'))}
            title={t('accounts.sort.sortDirection', '排序方向')}
          >
            {sortDirection === 'desc' ? <ChevronDown size={14} /> : <ChevronUp size={14} />}
          </button>
        </div>
        <div className="toolbar-right">
          <button
            type="button"
            className="btn btn-primary icon-only"
            onClick={() => void importFromLocal()}
            disabled={importingLocal}
            title={t('zcode.importLocal', '从本机导入（zcode-switch / ZCode 凭证）')}
          >
            {importingLocal ? <RefreshCw size={14} className="loading-spinner" /> : <Import size={14} />}
            <span>{t('zcode.importLocal', '从本机导入')}</span>
          </button>
          <button
            type="button"
            className="btn btn-secondary icon-only"
            onClick={() => jsonImportInputRef.current?.click()}
            disabled={importingLocal}
            title={t('common.shared.import.label', '导入')}
          >
            <Upload size={14} />
            <span>{t('common.shared.import.label', '导入')}</span>
          </button>
          <button
            type="button"
            className="btn btn-secondary icon-only"
            onClick={() => void handleExport(filteredIds)}
            disabled={exporting}
            title={t('common.shared.export.title', '导出')}
          >
            <Upload size={14} />
            <span>{t('common.shared.export.title', '导出')}</span>
          </button>
          <button
            type="button"
            className="btn btn-secondary icon-only"
            onClick={() => handleRefreshAll()}
            disabled={refreshingAll}
            title={t('common.refreshAll', '全部刷新')}
          >
            <RefreshCw size={14} className={refreshingAll ? 'loading-spinner' : ''} />
            <span>{t('common.refreshAll', '全部刷新')}</span>
          </button>
          <button
            type="button"
            className="btn btn-danger icon-only"
            onClick={() => handleBatchDelete()}
            disabled={selected.size === 0}
            title={t('common.delete', '删除')}
          >
            <Trash2 size={14} />
          </button>
        </div>
      </div>

      {importMessage && (
        <div className="status-pill normal zcode-import-message">
          <span>{importMessage}</span>
          <button type="button" onClick={() => setImportMessage(null)} aria-label={t('common.close', '关闭')}>
            <X size={12} />
          </button>
        </div>
      )}

      {store.loading ? (
        <div className="loading-container">
          <RefreshCw size={24} className="loading-spinner" />
        </div>
      ) : accounts.length === 0 ? (
        <div className="empty-state">
          <p>{t('zcode.noAccounts', '暂无 ZCode 账号')}</p>
          <button type="button" className="btn btn-primary" onClick={() => void importFromLocal()} disabled={importingLocal}>
            {importingLocal ? (
              <RefreshCw size={14} className="loading-spinner" />
            ) : (
              <Import size={14} />
            )}
            <span>{t('zcode.importLocal', '从本机导入')}</span>
          </button>
          <p className="empty-state-hint">
            {t(
              'zcode.emptyHint',
              '需要本机存在 zcode-switch 账号快照（~/.zcode-switch/accounts）或已登录的 ZCode（~/.zcode/v2/credentials.json）。',
            )}
          </p>
        </div>
      ) : (
        <div className="grid-view-container">
          <div className="grid-view-header" style={{ marginBottom: '12px', paddingLeft: '4px' }}>
            <label className="select-all-label">
              <input
                type="checkbox"
                checked={isAllSelected}
                onChange={toggleSelectAllVisible}
              />
              <span>
                {t('accounts.selectedCount', '已选 {{count}} 项', { count: selected.size })}
              </span>
            </label>
          </div>
          <div className="accounts-grid">
            {accounts.map((account) => {
              const displayName = getZcodeAccountDisplayName(account);
              const isSelected = selected.has(account.id);
              const quotaError = account.quota_query_last_error?.trim();
              const items = account.quota_items ?? [];
              return (
                <div key={account.id} className={`ghcp-account-card ${isSelected ? 'selected' : ''}`}>
                  <div className="card-top">
                    <div className="card-select">
                      <input
                        type="checkbox"
                        checked={isSelected}
                        onChange={() => toggleSelect(account.id)}
                      />
                    </div>
                    <span className="account-email" title={maskAccountText(displayName)}>
                      {maskAccountText(displayName)}
                    </span>
                    {quotaError && (
                      <span className="status-pill warning" title={quotaError}>
                        <CircleAlert size={12} />
                        {t('common.shared.quota.queryFailed', '配额查询失败')}
                      </span>
                    )}
                    <span className={`tier-badge ${getZcodePlanTone(account.plan_tier)}`}>
                      {getZcodePlanBadge(account)}
                    </span>
                  </div>

                  {account.email && (
                    <div className="card-tags">
                      <span className="tag-pill">{maskAccountText(account.email)}</span>
                    </div>
                  )}

                  <div className="windsurf-official-usage">
                    {items.length > 0 ? (
                      <div className="windsurf-official-usage-list">{items.map(renderUsageItem)}</div>
                    ) : (
                      <div className="windsurf-official-usage-note">
                        {quotaError
                          ? quotaError
                          : t('common.shared.quota.noData', '暂无配额数据')}
                      </div>
                    )}
                    {account.plan_expire && (
                      <div className="windsurf-official-usage-detail">
                        {t('zcode.planExpire', '套餐有效期')}: {account.plan_expire}
                      </div>
                    )}
                  </div>

                  <div className="card-footer">
                    <span className="card-date">
                      {formatDate(account.usage_updated_at ? Math.floor(account.usage_updated_at / 1000) : account.last_used || account.created_at)}
                    </span>
                    <div className="card-actions">
                      <button
                        type="button"
                        className="card-action-btn"
                        onClick={() => handleRefresh(account.id)}
                        disabled={refreshing === account.id}
                        title={t('common.shared.refreshQuota', '刷新配额')}
                      >
                        <RotateCw size={14} className={refreshing === account.id ? 'loading-spinner' : ''} />
                      </button>
                      <button
                        type="button"
                        className="card-action-btn export-btn"
                        onClick={() => handleExportByIds([account.id], account.name || account.id)}
                        title={t('common.shared.export.title', '导出')}
                      >
                        <Upload size={14} />
                      </button>
                      <button
                        type="button"
                        className="card-action-btn danger"
                        onClick={() => handleDelete(account.id)}
                        title={t('common.delete', '删除')}
                      >
                        <Trash2 size={14} />
                      </button>
                    </div>
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      )}

      <input
        ref={jsonImportInputRef}
        type="file"
        accept=".json,application/json"
        style={{ display: 'none' }}
        onChange={handleJsonImportFile}
      />

      {deleteConfirm && (
        <div className="modal-overlay" onClick={() => !deleting && setDeleteConfirm(null)}>
          <div className="modal" onClick={(event) => event.stopPropagation()}>
            <div className="modal-header">
              <h2>{t('common.confirm', '确认')}</h2>
              <button
                className="modal-close"
                onClick={() => !deleting && setDeleteConfirm(null)}
                aria-label={t('common.close', '关闭')}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              <ModalErrorMessage message={deleteConfirmError} scrollKey={deleteConfirmErrorScrollKey} />
              <p>{deleteConfirm.message}</p>
            </div>
            <div className="modal-footer">
              <button className="btn btn-secondary" onClick={() => setDeleteConfirm(null)} disabled={deleting}>
                {t('common.cancel', '取消')}
              </button>
              <button className="btn btn-danger" onClick={confirmDelete} disabled={deleting}>
                {t('common.confirm', '确认')}
              </button>
            </div>
          </div>
        </div>
      )}

    </div>
  );
}
