import { useEffect, useMemo, useState } from 'react';
import { AlertTriangle, Crosshair, X } from 'lucide-react';
import { PlatformOverviewTabsHeader, PlatformOverviewTab } from '../components/platform/PlatformOverviewTabsHeader';
import { CodebuddyCnInstancesContent } from './CodebuddyCnInstancesPage';
import { useCodebuddyCnAccountStore } from '../stores/useCodebuddyCnAccountStore';
import * as codebuddyCnService from '../services/codebuddyCnService';
import {
  CodebuddyAccount,
  type CodebuddyOfficialQuotaResource,
  getCodebuddyAccountDisplayEmail,
  getCodebuddyOfficialQuotaModel,
  getCodebuddyPlanBadge,
  getCodebuddyUsage,
  getCodebuddyQuotaCategoryGroups,
} from '../types/codebuddy';
import { useProviderAccountsPage } from '../hooks/useProviderAccountsPage';
import { CodebuddySuiteAccountsSharedView, type CodebuddySuiteAccountsPlatformConfig } from '../components/codebuddy-suite/CodebuddySuiteAccountsSharedView';
import { CodebuddyQuotaCreditsSummary } from '../components/codebuddy-suite/CodebuddyQuotaCreditsSummary';
import { CodebuddySessionManager } from '../components/codebuddy/CodebuddySessionManager';
import { compareCurrentAccountFirst } from '../utils/currentAccountSort';
import { WorkbuddyModelProbeModal } from '../components/workbuddy/WorkbuddyModelProbeModal';
import {
  dismissWorkbuddyProbeRecord,
  getWorkbuddyProbeRecords,
  subscribeWorkbuddyProbeStatus,
} from '../utils/workbuddyProbeStatus';

const CB_FLOW_NOTICE_COLLAPSED_KEY = 'agtools.codebuddycn.flow_notice_collapsed';

/** 受限提示里的短时间格式：MM/DD HH:mm */
function formatProbeTime(timestamp: number): string {
  const date = new Date(timestamp);
  const pad = (value: number) => String(value).padStart(2, '0');
  return `${pad(date.getMonth() + 1)}/${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}


function compareQuotaItems(a: CodebuddyOfficialQuotaResource, b: CodebuddyOfficialQuotaResource): number {
  const aAvailable = !!a.unlimited || a.remain > 0;
  const bAvailable = !!b.unlimited || b.remain > 0;
  if (aAvailable !== bAvailable) return aAvailable ? -1 : 1;

  const aExpires = a.expireAt ?? Number.POSITIVE_INFINITY;
  const bExpires = b.expireAt ?? Number.POSITIVE_INFINITY;
  if (aExpires !== bExpires) return aExpires - bExpires;

  return b.remain - a.remain;
}

const codebuddyCnPlatformConfig: CodebuddySuiteAccountsPlatformConfig<CodebuddyAccount> = {
  pageClassName: 'codebuddy-accounts-page',
  quickSettingsType: 'codebuddy_cn',
  searchPlaceholderKey: 'codebuddyCn.search',
  searchPlaceholderDefault: '搜索 CodeBuddy CN 账号...',
  flowNotice: {
    titleKey: 'codebuddyCn.flowNotice.title',
    titleDefault: 'CodeBuddy CN 账号管理说明（点击展开/收起）',
    descKey: 'codebuddyCn.flowNotice.desc',
    descDefault: '切换账号需读取 CodeBuddy CN 本地认证存储并调用系统凭据服务进行加解密，数据仅在本地处理。',
    permissionKey: 'codebuddyCn.flowNotice.permission',
    permissionDefault: '权限范围：读取 CodeBuddy CN 认证数据库 (state.vscdb)，调用系统凭据能力（macOS Keychain / Windows DPAPI / Linux Secret Service）进行解密/回写。',
    networkKey: 'codebuddyCn.flowNotice.network',
    networkDefault: '网络范围：OAuth 授权登录与 Token 刷新需联网请求 codebuddy.cn 与 copilot.tencent.com；资源包配额刷新需调用计费 API。不上传本地密钥或凭证。',
  },
  noAccountsKey: 'codebuddyCn.noAccounts',
  noAccountsDefault: '暂无 CodeBuddy CN 账号',
  addAccountTitleKey: 'codebuddyCn.addAccount',
  addAccountTitleDefault: '添加 CodeBuddy CN 账号',
  oauthDescKey: 'codebuddyCn.oauthDesc',
  oauthDescDefault: '点击下方按钮将在浏览器中打开 CodeBuddy CN 授权页面。',
  oauthFeatureCardClassName: 'codebuddy-oauth-feature-card',
  oauthFeatureTitleKey: 'codebuddy.oauthFeature.oauth.title',
  oauthFeatureTitleDefault: '仅授权 IDE 登录信息',
  oauthFeatureItem1Key: 'codebuddy.oauthFeature.oauth.item1',
  oauthFeatureItem1Default: '在浏览器完成 OAuth 后即可添加账号并用于 IDE 切换。',
  oauthFeatureItem2Key: 'codebuddy.oauthFeature.oauth.item2',
  oauthFeatureItem2Default: '授权完成后会自动刷新资源包配额数据。',
  oauthFeatureItem3Key: 'codebuddy.oauthFeature.oauth.item3',
  oauthFeatureItem3Default: '账号卡片将按资源包展示额度、进度和刷新/到期时间。',
  oauthUrlInputPlaceholderKey: 'codebuddy.oauthUrlInputPlaceholder',
  oauthUrlInputPlaceholderDefault: '可手动输入授权地址',
  oauthWaitingKey: 'codebuddy.oauthWaiting',
  oauthWaitingDefault: '等待授权完成...',
  tokenDescKey: 'codebuddyCn.tokenDesc',
  tokenDescDefault: '粘贴 CodeBuddy CN 的 access token：',
  importLocalDescKey: 'codebuddyCn.import.localDesc',
  importLocalDescDefault: '支持从本机 CodeBuddy CN 客户端或 JSON 文件导入账号数据。',
  importLocalClientKey: 'codebuddyCn.import.localClient',
  importLocalClientDefault: '从本机 CodeBuddy CN 导入',
  syncButtonTitle: (t) => t('codebuddyCn.syncToWorkbuddy', '同步到 WorkBuddy'),
  syncSuccessMessage: (t, count) => t('codebuddyCn.syncSuccess', '成功同步 {{count}} 个账号到 WorkBuddy', { count }),
  syncFailedMessage: (t, error) => t('codebuddyCn.syncFailed', '同步失败: {{error}}', { error }),
  runSync: () => codebuddyCnService.syncCodebuddyCnToWorkbuddy(),
  getDisplayEmail: (account) => getCodebuddyAccountDisplayEmail(account),
  getPlanBadge: (account) => getCodebuddyPlanBadge(account),
  getUsage: (account) => getCodebuddyUsage(account),
  getQuotaGroups: (account, t) => getCodebuddyQuotaCategoryGroups(account, t).map((group) => ({
    ...group,
    items: [...group.items].sort(compareQuotaItems),
  })),
  hasQuotaData: (account) => {
    const model = getCodebuddyOfficialQuotaModel(account);
    return model.resources.length > 0 || model.extra.total > 0 || model.extra.remain > 0 || model.extra.used > 0;
  },
  usagePrefix: 'codebuddy',
  quotaPrefix: 'codebuddy',
  tableUsageClassName: 'codebuddy-table-usage',
};

export function CodebuddyCnAccountsPage() {
  const [activeTab, setActiveTab] = useState<PlatformOverviewTab>('overview');
  const [probeOpen, setProbeOpen] = useState(false);
  const [probeAccountId, setProbeAccountId] = useState<string | null>(null);
  const [selectedAccountIds, setSelectedAccountIds] = useState<string[]>([]);
  const [probeVersion, setProbeVersion] = useState(0);
  const store = useCodebuddyCnAccountStore();

  // 探测结果变化时刷新卡片上的受限标识
  useEffect(() => subscribeWorkbuddyProbeStatus(() => setProbeVersion((v) => v + 1)), []);

  // 恢复时间过后自动复测：确认恢复则在卡片提示后标注「已恢复」（CN 与 WorkBuddy 额度共享、同一套逻辑）

  const page = useProviderAccountsPage<CodebuddyAccount>({
    platformKey: 'CodeBuddy CN',
    oauthLogPrefix: 'CodebuddyCnOAuth',
    flowNoticeCollapsedKey: CB_FLOW_NOTICE_COLLAPSED_KEY,
    exportFilePrefix: 'codebuddy_cn_accounts',
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
      startLogin: codebuddyCnService.startCodebuddyCnOAuthLogin,
      completeLogin: codebuddyCnService.completeCodebuddyCnOAuthLogin,
      cancelLogin: codebuddyCnService.cancelCodebuddyCnOAuthLogin,
    },
    dataService: {
      importFromJson: codebuddyCnService.importCodebuddyCnFromJson,
      importFromLocal: codebuddyCnService.importCodebuddyCnFromLocal,
      addWithToken: codebuddyCnService.addCodebuddyCnAccountWithToken,
      exportAccounts: codebuddyCnService.exportCodebuddyCnAccounts,
      injectToVSCode: codebuddyCnService.injectCodebuddyCnToVSCode,
    },
    getDisplayEmail: (account) => getCodebuddyAccountDisplayEmail(account),
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

  // 与 WorkBuddy 一致，用「总剩余额度 + 分组明细」的新版概览替换平铺的资源包列表；
  // 无配额数据时返回 null，让共享视图回退到原有的空状态提示。
  const platformConfig = useMemo<
    CodebuddySuiteAccountsPlatformConfig<CodebuddyAccount>
  >(
    () => ({
      ...codebuddyCnPlatformConfig,
      renderCardActionExtra: (account) => (
        <button
          type="button"
          className="card-action-btn"
          onClick={() => {
            setProbeAccountId(account.id);
            setProbeOpen(true);
          }}
          title={page.t('workbuddy.modelProbe.title', '探测模型')}
        >
          <Crosshair size={14} />
        </button>
      ),
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
                    {record.resetAt
                        ? page.t('workbuddy.probeNotice.until', '{{model}} 不可用至 {{time}}', {
                            model: record.model,
                            time: formatProbeTime(record.resetAt),
                          })
                        : page.t('workbuddy.probeNotice.unavailable', '{{model}} 暂不可用', {
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
                    title={page.t('common.close')}
                  >
                    <X size={12} aria-hidden="true" />
                  </button>
                </div>
              );
            })}
          </div>
        );
      },

      renderQuotaSummary: (account) => {
        const groups = getCodebuddyQuotaCategoryGroups(
          account,
          page.t as (key: string, defaultValue?: string) => string,
        );
        return groups.some((group) => group.visible) ? (
          <CodebuddyQuotaCreditsSummary groups={groups} />
        ) : null;
      },
    }),
    [page.t, probeVersion],
  );

  return (
    <div className={`ghcp-accounts-page ${codebuddyCnPlatformConfig.pageClassName}`}>
      <div className="page-toolbar workbuddy-probe-toolbar">
        <button
          type="button"
          className="btn btn-secondary btn-sm"
          onClick={() => {
            setProbeAccountId(null);
            setProbeOpen(true);
          }}
        >
          <Crosshair size={14} aria-hidden="true" />
          {page.t('workbuddy.modelProbe.open', '批量探测模型')}
        </button>
      </div>
      <PlatformOverviewTabsHeader
        platform="codebuddy_cn"
        active={activeTab}
        onTabChange={setActiveTab}
        tabs={['overview', 'sessions', 'instances']}
      />
      {activeTab === 'instances' ? (
        <CodebuddyCnInstancesContent accountsForSelect={accountsForInstances} />
      ) : activeTab === 'sessions' ? (
        <CodebuddySessionManager platform="cn" accounts={store.accounts as any} />
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
      {probeOpen && (
        <WorkbuddyModelProbeModal
          accounts={store.accounts}
          currentAccountId={probeAccountId ?? store.currentAccountId}
          selectedAccountIds={selectedAccountIds}
          initialScope={probeAccountId ? 'current' : undefined}
          lockedAccountId={probeAccountId}
          platform="codebuddy_cn"
          displayEmail={getCodebuddyAccountDisplayEmail}
          onClose={() => {
            setProbeOpen(false);
            setProbeAccountId(null);
          }}
        />
      )}
    </div>
  );
}
