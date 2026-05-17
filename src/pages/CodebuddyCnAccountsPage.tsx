import { useMemo, useState } from 'react';
import { PlatformOverviewTabsHeader, PlatformOverviewTab } from '../components/platform/PlatformOverviewTabsHeader';
import { CodebuddyCnInstancesContent } from './CodebuddyCnInstancesPage';
import { useCodebuddyCnAccountStore } from '../stores/useCodebuddyCnAccountStore';
import * as codebuddyCnService from '../services/codebuddyCnService';
import {
  CodebuddyAccount,
  getCodebuddyAccountDisplayEmail,
  getCodebuddyOfficialQuotaModel,
  getCodebuddyPlanBadge,
  getCodebuddyUsage,
  getCodebuddyQuotaCategoryGroups,
} from '../types/codebuddy';
import { useProviderAccountsPage } from '../hooks/useProviderAccountsPage';
import { CodebuddySuiteAccountsSharedView, type CodebuddySuiteAccountsPlatformConfig } from '../components/codebuddy-suite/CodebuddySuiteAccountsSharedView';
import { compareCurrentAccountFirst } from '../utils/currentAccountSort';

const CB_FLOW_NOTICE_COLLAPSED_KEY = 'agtools.codebuddycn.flow_notice_collapsed';
const CB_CURRENT_ACCOUNT_ID_KEY = 'agtools.codebuddycn.current_account_id';

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
  getQuotaGroups: (account, t) => getCodebuddyQuotaCategoryGroups(account, t),
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
  const store = useCodebuddyCnAccountStore();

  const page = useProviderAccountsPage<CodebuddyAccount>({
    platformKey: 'CodeBuddy CN',
    oauthLogPrefix: 'CodebuddyCnOAuth',
    flowNoticeCollapsedKey: CB_FLOW_NOTICE_COLLAPSED_KEY,
    currentAccountIdKey: CB_CURRENT_ACCOUNT_ID_KEY,
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

  return (
    <div className={`ghcp-accounts-page ${codebuddyCnPlatformConfig.pageClassName}`}>
      <PlatformOverviewTabsHeader
        platform="codebuddy_cn"
        active={activeTab}
        onTabChange={setActiveTab}
      />
      {activeTab === 'instances' ? (
        <CodebuddyCnInstancesContent accountsForSelect={accountsForInstances} />
      ) : (
        <CodebuddySuiteAccountsSharedView
          accounts={store.accounts}
          loading={store.loading}
          page={page}
          platformConfig={codebuddyCnPlatformConfig}
          onRefreshAccounts={() => { store.fetchAccounts(); }}
        />
      )}
    </div>
  );
}
