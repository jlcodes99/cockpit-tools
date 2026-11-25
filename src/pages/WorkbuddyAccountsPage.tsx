import { useEffect, useMemo, useState } from 'react';
import { ChevronRight, Crosshair, Sparkles } from 'lucide-react';
import { PlatformOverviewTabsHeader, PlatformOverviewTab } from '../components/platform/PlatformOverviewTabsHeader';
import { WorkbuddyInstancesContent } from './WorkbuddyInstancesPage';
import { useWorkbuddyAccountStore } from '../stores/useWorkbuddyAccountStore';
import * as codexService from '../services/codexService';
import * as workbuddyService from '../services/workbuddyService';
import { emitAccountsChanged } from '../utils/accountSyncEvents';
import { syncWorkbuddyToCodebuddyCn } from '../services/codebuddyCnService';
import {
  WorkbuddyAccount,
  getWorkbuddyAccountDisplayEmail,
  getWorkbuddyPlanBadge,
  getWorkbuddyUsage,
  getWorkbuddyQuotaCategoryGroups,
} from '../types/workbuddy';
import { useProviderAccountsPage } from '../hooks/useProviderAccountsPage';
import { useWorkbuddyProbeAutoRecheck } from '../hooks/useWorkbuddyProbeAutoRecheck';
import { WorkbuddyCheckinModal } from '../components/codebuddy-suite/CodebuddySuiteCheckinModal';
import { CodebuddySessionManager } from '../components/codebuddy/CodebuddySessionManager';
import { CodebuddySuiteAccountsSharedView, type CodebuddySuiteAccountsPlatformConfig } from '../components/codebuddy-suite/CodebuddySuiteAccountsSharedView';
import { compareCurrentAccountFirst } from '../utils/currentAccountSort';
import { WorkbuddyCreditsDialog, WorkbuddyCreditsSummary } from '../components/codebuddy-suite/WorkbuddyCredits';
import { getWorkbuddyCredits, getWorkbuddyExpiringCredits, hasWorkbuddyExpiringCredits } from '../utils/workbuddyCredits';
import {
  subscribeWorkbuddyProbeStatus,
} from '../utils/workbuddyProbeStatus';
import { WorkbuddyProbeNotice } from '../components/workbuddy/WorkbuddyProbeNotice';
import { WorkbuddyModelProbeModal } from '../components/workbuddy/WorkbuddyModelProbeModal';

const WORKBUDDY_FLOW_NOTICE_COLLAPSED_KEY = 'agtools.workbuddy.flow_notice_collapsed';

/** 受限提示里的短时间格式：MM/DD HH:mm */


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
  const [codexBindingId, setCodexBindingId] = useState<string | null>(null);
  const [probeOpen, setProbeOpen] = useState(false);
  const [probeAccountId, setProbeAccountId] = useState<string | null>(null);
  const [selectedAccountIds, setSelectedAccountIds] = useState<string[]>([]);
  const [probeVersion, setProbeVersion] = useState(0);
  const store = useWorkbuddyAccountStore();

  // 探测结果变化时刷新卡片上的受限标识
  useEffect(() => subscribeWorkbuddyProbeStatus(() => setProbeVersion((v) => v + 1)), []);

  // 恢复时间过后自动复测：确认恢复则在卡片提示后标注「已恢复」
  useWorkbuddyProbeAutoRecheck(store.accounts);

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

  const handleExposeToCodex = async (account: WorkbuddyAccount) => {
    if (codexBindingId) return;
    setCodexBindingId(account.id);
    try {
      await codexService.addCodexAccountFromWorkbuddy(account.id);
      await emitAccountsChanged({ platformId: 'codex', reason: 'import' });
      page.setMessage({
        text: page.t('common.shared.exposeToCodex.success', '已接入 Codex'),
        tone: 'success',
      });
    } catch (error) {
      console.error('[WorkBuddy] expose to Codex failed:', error);
      page.setMessage({
        text: page.t('common.shared.exposeToCodex.failed', '接入 Codex 失败：{{error}}', {
          error: String(error).replace(/^Error:\s*/, ''),
        }),
        tone: 'error',
      });
    } finally {
      setCodexBindingId(null);
    }
  };

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
        <WorkbuddyCreditsSummary
          credits={credits}
          account={account}
          onDetails={() => setCreditAccountId(account.id)}
        />
      );
    },
    renderQuotaAction: (account) => account.enterprise_id ? null : (
      <button type="button" className="btn btn-sm workbuddy-credit-link" onClick={() => setCreditAccountId(account.id)}>
        {page.t('workbuddy.credits.details')} <ChevronRight size={13} aria-hidden="true" />
      </button>
    ),
    // 每张卡片操作栏里的探测入口：点一下直接带上该账号打开探测弹窗
    renderCardActionExtra: (account) => (
      <>
        <button
          type="button"
          className="card-action-btn"
          disabled={codexBindingId !== null}
          aria-busy={codexBindingId === account.id}
          onClick={() => { void handleExposeToCodex(account); }}
          title={page.t('workbuddy.exposeToCodex.title', '仅在 Native Rust 网关中接入，不会复制 WorkBuddy Token')}
        >
          {codexBindingId === account.id
            ? page.t('common.shared.exposeToCodex.inProgress', '接入中…')
            : page.t('common.shared.exposeToCodex.action', '接入 Codex')}
        </button>
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
      </>
    ),
    // 卡片底部「包含即将到期的积分」提示（24h 内已由右上角精确徽章展示时避免重复）
    renderCardFooterHint: (account) => {
      if (
        account.enterprise_id ||
        !hasWorkbuddyExpiringCredits(account) ||
        getWorkbuddyExpiringCredits(account).total > 0
      ) {
        return null;
      }
      return (
        <button
          type="button"
          className="workbuddy-credit-expiring-hint"
          onClick={() => setCreditAccountId(account.id)}
          title={page.t('workbuddy.credits.expiringHintTitle', '查看即将到期的积分明细')}
        >
          <Sparkles size={12} aria-hidden="true" />
          {page.t('workbuddy.credits.expiring', '包含即将到期的积分')}
        </button>
      );
    },
    // 卡片内的模型受限标识（切号前一眼确认）：可点 × 关闭
    renderCardNotice: (account) => <WorkbuddyProbeNotice accountId={account.id} />,
  }), [codexBindingId, page.t, probeVersion]);
  const creditAccount = store.accounts.find((account) => account.id === creditAccountId);

  return (
    <div className={`ghcp-accounts-page ${workbuddyPlatformConfig.pageClassName}`}>
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
        <>
          <div className="page-toolbar workbuddy-probe-toolbar">
            <button
              type="button"
              className="btn btn-secondary btn-sm"
              onClick={() => setProbeOpen(true)}
            >
              <Crosshair size={14} aria-hidden="true" />
              {page.t('workbuddy.modelProbe.open', '探测模型可用性')}
            </button>
          </div>
          <CodebuddySuiteAccountsSharedView
            accounts={store.accounts}
            loading={store.loading}
            page={page}
            platformConfig={platformConfig}
            onSelectionChange={setSelectedAccountIds}
            onRefreshAccounts={() => { store.fetchAccounts(); }}
          />
        </>
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
