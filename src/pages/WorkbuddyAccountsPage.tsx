import { useEffect, useMemo, useState } from 'react';
import { AlertTriangle, ChevronRight, Crosshair, X } from 'lucide-react';
import { PlatformOverviewTabsHeader, PlatformOverviewTab } from '../components/platform/PlatformOverviewTabsHeader';
import { WorkbuddyInstancesContent } from './WorkbuddyInstancesPage';
import { useWorkbuddyAccountStore } from '../stores/useWorkbuddyAccountStore';
import * as workbuddyService from '../services/workbuddyService';
import { syncWorkbuddyToCodebuddyCn } from '../services/codebuddyCnService';
import {
  WorkbuddyAccount,
  getWorkbuddyAccountDisplayEmail,
  getWorkbuddyPlanBadge,
  getWorkbuddyUsage,
  getWorkbuddyQuotaCategoryGroups,
} from '../types/workbuddy';
import { useProviderAccountsPage } from '../hooks/useProviderAccountsPage';
import { WorkbuddyCheckinModal } from '../components/codebuddy-suite/CodebuddySuiteCheckinModal';
import { CodebuddySessionManager } from '../components/codebuddy/CodebuddySessionManager';
import { CodebuddySuiteAccountsSharedView, type CodebuddySuiteAccountsPlatformConfig } from '../components/codebuddy-suite/CodebuddySuiteAccountsSharedView';
import { compareCurrentAccountFirst } from '../utils/currentAccountSort';
import { WorkbuddyCreditsDialog, WorkbuddyCreditsSummary } from '../components/codebuddy-suite/WorkbuddyCredits';
import { getWorkbuddyCredits } from '../utils/workbuddyCredits';
import {
  dismissWorkbuddyProbeRecord,
  getWorkbuddyProbeRecords,
  subscribeWorkbuddyProbeStatus,
} from '../utils/workbuddyProbeStatus';
import { WorkbuddyModelProbeModal } from '../components/workbuddy/WorkbuddyModelProbeModal';

const WORKBUDDY_FLOW_NOTICE_COLLAPSED_KEY = 'agtools.workbuddy.flow_notice_collapsed';

/** 受限提示里的短时间格式：MM/DD HH:mm */
function formatProbeTime(timestamp: number): string {
  const date = new Date(timestamp);
  const pad = (value: number) => String(value).padStart(2, '0');
  return `${pad(date.getMonth() + 1)}/${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

const workbuddyPlatformConfig: CodebuddySuiteAccountsPlatformConfig<WorkbuddyAccount> = {
  pageClassName: 'workbuddy-accounts-page',
  quickSettingsType: 'workbuddy',
  searchPlaceholderKey: 'workbuddy.search',
  searchPlaceholderDefault: '搜索 WorkBuddy 账号...',
  flowNotice: {
    titleKey: 'workbuddy.flowNotice.title',
    titleDefault: 'WorkBuddy 账号管理说明（点击展开/收起）',
    descKey: 'workbuddy.flowNotice.desc',
    descDefault: '切换账号需读取 WorkBuddy 本地认证存储并调用系统凭据服务进行加解密，数据仅在本地处理。',
    permissionKey: 'workbuddy.flowNotice.permission',
    permissionDefault: '权限范围：读取 WorkBuddy 认证数据库，调用系统凭据能力（macOS Keychain / Windows DPAPI / Linux Secret Service）进行解密/回写。',
    networkKey: 'workbuddy.flowNotice.network',
    networkDefault: 'OAuth 登录和 Token 刷新会请求 WorkBuddy 服务。积分余额与明细查询使用账号 Token 请求 WorkBuddy 计费服务。本地登录文件和系统密钥保留在本机。',
  },
  noAccountsKey: 'workbuddy.noAccounts',
  noAccountsDefault: '暂无 WorkBuddy 账号',
  addAccountTitleKey: 'workbuddy.addAccount',
  addAccountTitleDefault: '添加 WorkBuddy 账号',
  oauthDescKey: 'workbuddy.oauthDesc',
  oauthDescDefault: '点击下方按钮将在浏览器中打开 WorkBuddy 授权页面。',
  oauthFeatureCardClassName: 'workbuddy-oauth-feature-card',
  oauthFeatureTitleKey: 'workbuddy.oauthFeature.oauth.title',
  oauthFeatureTitleDefault: '仅授权 IDE 登录信息',
  oauthFeatureItem1Key: 'workbuddy.oauthFeature.oauth.item1',
  oauthFeatureItem1Default: '在浏览器完成 OAuth 后即可添加账号并用于 IDE 切换。',
  oauthFeatureItem2Key: 'workbuddy.oauthFeature.oauth.item2',
  oauthFeatureItem2Default: '授权完成后会自动查询积分余额。',
  oauthFeatureItem3Key: 'workbuddy.oauthFeature.oauth.item3',
  oauthFeatureItem3Default: '账号卡片展示积分概览，可打开「积分明细」查看积分记录和到期时间。',
  oauthUrlInputPlaceholderKey: 'workbuddy.oauthUrlInputPlaceholder',
  oauthUrlInputPlaceholderDefault: '可手动输入授权地址',
  oauthWaitingKey: 'workbuddy.oauthWaiting',
  oauthWaitingDefault: '等待授权完成...',
  tokenDescKey: 'workbuddy.tokenDesc',
  tokenDescDefault: '粘贴 WorkBuddy 的 access token：',
  importLocalDescKey: 'workbuddy.import.localDesc',
  importLocalDescDefault: '支持从本机 WorkBuddy 客户端或 JSON 文件导入账号数据。',
  importLocalClientKey: 'workbuddy.import.localClient',
  importLocalClientDefault: '从本机 WorkBuddy 导入',
  syncButtonTitle: (t) => `${t('common.shared.import.label', '导入')} ${t('nav.codebuddyCn', 'CodeBuddy CN')}`,
  syncSuccessMessage: (t, count) => t('common.shared.token.importSuccessMsg', '成功导入 {{count}} 个账号', { count }),
  syncFailedMessage: (t, error) => t('common.shared.token.importFailedMsg', '导入失败: {{error}}', { error }),
  runSync: () => syncWorkbuddyToCodebuddyCn(),
  getDisplayEmail: (account) => getWorkbuddyAccountDisplayEmail(account),
  getPlanBadge: (account) => getWorkbuddyPlanBadge(account),
  getUsage: (account) => getWorkbuddyUsage(account),
  getQuotaGroups: (account, t) => getWorkbuddyQuotaCategoryGroups(account, t),
  hasQuotaData: (_account, groups) => groups.some((g) => g.items.length > 0),
  usagePrefix: 'workbuddy',
  quotaPrefix: 'workbuddy',
  tableUsageClassName: 'workbuddy-table-usage',
  CheckinModal: WorkbuddyCheckinModal,
};

export function WorkbuddyAccountsPage() {
  const [activeTab, setActiveTab] = useState<PlatformOverviewTab>('overview');
  const [creditAccountId, setCreditAccountId] = useState<string | null>(null);
  const [probeOpen, setProbeOpen] = useState(false);
  const [probeAccountId, setProbeAccountId] = useState<string | null>(null);
  const [selectedAccountIds, setSelectedAccountIds] = useState<string[]>([]);
  const [probeVersion, setProbeVersion] = useState(0);
  const store = useWorkbuddyAccountStore();

  // 探测结果变化时刷新卡片上的受限标识
  useEffect(() => subscribeWorkbuddyProbeStatus(() => setProbeVersion((v) => v + 1)), []);

  // 恢复时间过后自动复测：确认恢复则在卡片提示后标注「已恢复」

  const page = useProviderAccountsPage<WorkbuddyAccount>({
    platformKey: 'WorkBuddy',
    oauthLogPrefix: 'WorkbuddyOAuth',
    flowNoticeCollapsedKey: WORKBUDDY_FLOW_NOTICE_COLLAPSED_KEY,
    exportFilePrefix: 'workbuddy_accounts',
    oauthTabKeys: ['oauth'],
    store: {
      accounts: store.accounts,
      currentAccountId: store.currentAccountId,
      loading: store.loading,
      error: store.error,
      fetchAccounts: store.fetchAccounts,
      fetchCurrentAccountId: store.fetchCurrentAccountId,
      deleteAccounts: store.deleteAccounts,
      refreshToken: store.refreshToken,
      refreshAllTokens: store.refreshAllTokens,
      setCurrentAccountId: store.setCurrentAccountId,
      updateAccountTags: store.updateAccountTags,
    },
    oauthService: {
      startLogin: workbuddyService.startWorkbuddyOAuthLogin,
      completeLogin: workbuddyService.completeWorkbuddyOAuthLogin,
      cancelLogin: workbuddyService.cancelWorkbuddyOAuthLogin,
    },
    dataService: {
      importFromJson: workbuddyService.importWorkbuddyFromJson,
      importFromLocal: workbuddyService.importWorkbuddyFromLocal,
      addWithToken: workbuddyService.addWorkbuddyAccountWithToken,
      exportAccounts: workbuddyService.exportWorkbuddyAccounts,
      injectToVSCode: workbuddyService.injectWorkbuddyToVSCode,
      openWebview: workbuddyService.openWorkbuddyWebview,
    },
    getDisplayEmail: (account) => getWorkbuddyAccountDisplayEmail(account),
  });

  const accountsForInstances = useMemo(
    () =>
      [...store.accounts].sort((a, b) => {
        const currentFirstDiff = compareCurrentAccountFirst(a.id, b.id, store.currentAccountId);
        if (currentFirstDiff !== 0) {
          return currentFirstDiff;
        }
        const diff = b.created_at - a.created_at;
        return page.sortDirection === 'desc' ? diff : -diff;
      }),
    [page.sortDirection, store.accounts, store.currentAccountId],
  );

  const platformConfig = useMemo<CodebuddySuiteAccountsPlatformConfig<WorkbuddyAccount>>(() => ({
    ...workbuddyPlatformConfig,
    renderQuotaSummary: (account) => {
      if (account.enterprise_id) return null;
      const credits = getWorkbuddyCredits(account);
      return credits.left == null ? null : (
        <WorkbuddyCreditsSummary credits={credits} onDetails={() => setCreditAccountId(account.id)} />
      );
    },
    renderQuotaAction: (account) => account.enterprise_id ? null : (
      <button type="button" className="btn btn-sm workbuddy-credit-link" onClick={() => setCreditAccountId(account.id)}>
        {page.t('workbuddy.credits.details')} <ChevronRight size={13} aria-hidden="true" />
      </button>
    ),
    // 每张卡片操作栏里的探测入口：点一下直接带上该账号打开探测弹窗
    renderCardActionExtra: (account) => (
      <button
        type="button"
        className="card-action-btn"
        onClick={() => {
          setProbeAccountId(account.id);
          setProbeOpen(true);
        }}
        title={page.t('workbuddy.modelProbe.title', '探测模型可用性')}
      >
        <Crosshair size={14} />
      </button>
    ),
    // 卡片内的模型受限标识（切号前一眼确认）：可点 × 关闭
    renderCardNotice: (account) => {
      const records = getWorkbuddyProbeRecords(account.id);
      if (records.length === 0) return null;
      return (
        <div className="wb-probe-notice">
          {records.slice(0, 2).map((record) => {
            return (
              <div key={record.model} className={`wb-probe-notice-item ${record.status}`}>
                <AlertTriangle size={12} aria-hidden="true" />
                <span title={record.message}>
                  {record.status === 'rate_limited' && record.resetAt
                      ? page.t('workbuddy.probeNotice.until', '{{model}} 不可用至 {{time}}', {
                          model: record.model,
                          time: formatProbeTime(record.resetAt),
                        })
                      : page.t(record.status === 'error' ? 'workbuddy.probeNotice.failed' : 'workbuddy.probeNotice.unavailable', {
                          model: record.model,
                        })}
                </span>
                <button
                  type="button"
                  className="wb-probe-notice-close"
                  onClick={(event) => {
                    event.stopPropagation();
                    dismissWorkbuddyProbeRecord(account.id, record.model);
                  }}
                  title={page.t('common.close', '关闭')}
                >
                  <X size={12} aria-hidden="true" />
                </button>
              </div>
            );
          })}
        </div>
      );
    },
  }), [page.t, probeVersion]);
  const creditAccount = store.accounts.find((account) => account.id === creditAccountId);

  return (
    <div className={`ghcp-accounts-page ${workbuddyPlatformConfig.pageClassName}`}>
      <div className="page-toolbar workbuddy-probe-toolbar">
        <button
          type="button"
          className="btn btn-secondary btn-sm"
          onClick={() => setProbeOpen(true)}
        >
          <Crosshair size={14} aria-hidden="true" />
          {page.t('workbuddy.modelProbe.open', '批量探测模型')}
        </button>
      </div>
      <PlatformOverviewTabsHeader
        platform="workbuddy"
        active={activeTab}
        onTabChange={setActiveTab}
        tabs={['overview', 'sessions', 'instances']}
      />
      {activeTab === 'sessions' ? (
        <CodebuddySessionManager platform="workbuddy" accounts={store.accounts as any} />
      ) : activeTab === 'instances' ? (
        <WorkbuddyInstancesContent accountsForSelect={accountsForInstances} />
      ) : (
        <CodebuddySuiteAccountsSharedView
          accounts={store.accounts}
          loading={store.loading}
          page={page}
          platformConfig={platformConfig}
          onSelectionChange={setSelectedAccountIds}
          onRefreshAccounts={() => { store.fetchAccounts(); }}
        />
      )}
      {creditAccount && (
        <WorkbuddyCreditsDialog
          key={creditAccount.id}
          account={creditAccount}
          accountLabel={page.maskAccountText(getWorkbuddyAccountDisplayEmail(creditAccount))}
          onClose={() => setCreditAccountId(null)}
        />
      )}
      {probeOpen && (
        <WorkbuddyModelProbeModal
          accounts={store.accounts}
          currentAccountId={probeAccountId ?? store.currentAccountId}
          selectedAccountIds={selectedAccountIds}
          initialScope={probeAccountId ? 'current' : undefined}
          lockedAccountId={probeAccountId}
          onClose={() => {
            setProbeOpen(false);
            setProbeAccountId(null);
          }}
        />
      )}
    </div>
  );
}
