import { useMemo, useState } from 'react';
import { PlatformOverviewTabsHeader, PlatformOverviewTab } from '../components/platform/PlatformOverviewTabsHeader';
import { MinimaxCodeInstancesContent } from './MinimaxCodeInstancesPage';
import { useMinimaxCodeAccountStore } from '../stores/useMinimaxCodeAccountStore';
import * as minimaxCodeService from '../services/minimaxCodeService';
import {
  MinimaxCodeAccount,
  getMinimaxCodeAccountDisplayEmail,
  getMinimaxCodePlanBadge,
  getMinimaxCodeUsage,
} from '../types/minimaxCode';
import { getMiniMaxCodeQuotaGroups, hasMiniMaxCodeQuotaData } from '../utils/minimaxCode/parser';
import { useProviderAccountsPage } from '../hooks/useProviderAccountsPage';
import { MinimaxCodeCheckinModal } from '../components/codebuddy-suite/CodebuddySuiteCheckinModal';
import { CodebuddySuiteAccountsSharedView, type CodebuddySuiteAccountsPlatformConfig } from '../components/codebuddy-suite/CodebuddySuiteAccountsSharedView';
import { compareCurrentAccountFirst } from '../utils/currentAccountSort';

const MINIMAX_CODE_FLOW_NOTICE_COLLAPSED_KEY = 'agtools.minimaxcode.flow_notice_collapsed';
const MINIMAX_CODE_CURRENT_ACCOUNT_ID_KEY = 'agtools.minimaxcode.current_account_id';

const minimaxCodePlatformConfig: CodebuddySuiteAccountsPlatformConfig<MinimaxCodeAccount> = {
  pageClassName: 'minimax-code-accounts-page',
  quickSettingsType: 'minimaxcode',
  checkinModalTitleKey: 'minimaxCode.checkin.modalTitle',
  checkinModalTitleDefault: '每日签到',
  searchPlaceholderKey: 'minimaxCode.search',
  searchPlaceholderDefault: '搜索 MiniMax Code 账号...',
  flowNotice: {
    titleKey: 'minimaxCode.flowNotice.title',
    titleDefault: 'MiniMax Code 账号管理说明（点击展开/收起）',
    descKey: 'minimaxCode.flowNotice.desc',
    descDefault: '切换账号会读取 MiniMax Code 本地认证存储并写入客户端 auth.json，数据仅在本地处理。',
    permissionKey: 'minimaxCode.flowNotice.permission',
    permissionDefault: '权限范围：读取并写入 MiniMax Code 登录文件（~/.minimax/auth/auth.json），用于本机导入与账号切换。',
    networkKey: 'minimaxCode.flowNotice.network',
    networkDefault: '网络范围：OAuth 授权登录与 Token 刷新需联网请求 MiniMax Code 服务。不上传本地登录文件或凭证原文。',
  },
  noAccountsKey: 'minimaxCode.noAccounts',
  noAccountsDefault: '暂无 MiniMax Code 账号',
  addAccountTitleKey: 'minimaxCode.addAccount',
  addAccountTitleDefault: '添加 MiniMax Code 账号',
  oauthDescKey: 'minimaxCode.oauthDesc',
  oauthDescDefault: '点击下方按钮将在浏览器中打开 MiniMax Code 授权页面。',
  oauthFeatureCardClassName: 'minimax-code-oauth-feature-card',
  oauthFeatureTitleKey: 'minimaxCode.oauthFeature.oauth.title',
  oauthFeatureTitleDefault: '仅授权 IDE 登录信息',
  oauthFeatureItem1Key: 'minimaxCode.oauthFeature.oauth.item1',
  oauthFeatureItem1Default: '在浏览器完成 OAuth 后即可添加账号并用于 IDE 切换。',
  oauthFeatureItem2Key: 'minimaxCode.oauthFeature.oauth.item2',
  oauthFeatureItem2Default: '授权完成后会自动刷新资源包配额数据。',
  oauthFeatureItem3Key: 'minimaxCode.oauthFeature.oauth.item3',
  oauthFeatureItem3Default: '账号卡片将按资源包展示额度、进度和刷新/到期时间。',
  oauthUrlInputPlaceholderKey: 'minimaxCode.oauthUrlInputPlaceholder',
  oauthUrlInputPlaceholderDefault: '可手动输入授权地址',
  oauthWaitingKey: 'minimaxCode.oauthWaiting',
  oauthWaitingDefault: '等待授权完成...',
  tokenDescKey: 'minimaxCode.tokenDesc',
  tokenDescDefault: '粘贴 MiniMax Code 的 access token：',
  importLocalDescKey: 'minimaxCode.import.localDesc',
  importLocalDescDefault: '支持从本机 MiniMax Code 客户端或 JSON 文件导入账号数据。',
  importLocalClientKey: 'minimaxCode.import.localClient',
  importLocalClientDefault: '从本机 MiniMax Code 导入',
  getDisplayEmail: (account) => getMinimaxCodeAccountDisplayEmail(account),
  getPlanBadge: (account) => getMinimaxCodePlanBadge(account),
  getUsage: (account) => ({
    ...getMinimaxCodeUsage(account),
    isNormal: true,
  }),
  getQuotaGroups: (account, t) => getMiniMaxCodeQuotaGroups(account, t),
  hasQuotaData: (account, groups) => hasMiniMaxCodeQuotaData(account, groups),
  usagePrefix: 'minimaxCode',
  quotaPrefix: 'minimaxCode',
  tableUsageClassName: 'minimax-code-table-usage',
  CheckinModal: MinimaxCodeCheckinModal,
};

export function MinimaxCodeAccountsPage() {
  const [activeTab, setActiveTab] = useState<PlatformOverviewTab>('overview');
  const store = useMinimaxCodeAccountStore();

  const page = useProviderAccountsPage<MinimaxCodeAccount>({
    platformKey: 'MiniMax Code',
    oauthLogPrefix: 'MinimaxCodeOAuth',
    flowNoticeCollapsedKey: MINIMAX_CODE_FLOW_NOTICE_COLLAPSED_KEY,
    currentAccountIdKey: MINIMAX_CODE_CURRENT_ACCOUNT_ID_KEY,
    exportFilePrefix: 'minimax_code_accounts',
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
      startLogin: () => minimaxCodeService.startMinimaxCodeOAuthLogin(),
      completeLogin: minimaxCodeService.completeMinimaxCodeOAuthLogin,
      cancelLogin: minimaxCodeService.cancelMinimaxCodeOAuthLogin,
    },
    dataService: {
      importFromJson: minimaxCodeService.importMinimaxCodeFromJson,
      importFromLocal: minimaxCodeService.importMinimaxCodeFromLocal,
      addWithToken: minimaxCodeService.addMinimaxCodeAccountWithToken,
      exportAccounts: minimaxCodeService.exportMinimaxCodeAccounts,
      injectToVSCode: minimaxCodeService.injectMinimaxCodeToClient,
      openWebview: undefined,
    },
    getDisplayEmail: (account) => getMinimaxCodeAccountDisplayEmail(account),
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
    <div className={`ghcp-accounts-page ${minimaxCodePlatformConfig.pageClassName}`}>
      <PlatformOverviewTabsHeader
        platform="minimaxcode"
        active={activeTab}
        onTabChange={setActiveTab}
        tabs={['overview', 'instances']}
      />
      {activeTab === 'instances' ? (
        <MinimaxCodeInstancesContent accountsForSelect={accountsForInstances} />
      ) : (
        <CodebuddySuiteAccountsSharedView
          accounts={store.accounts}
          loading={store.loading}
          page={page}
          platformConfig={minimaxCodePlatformConfig}
          onRefreshAccounts={() => { store.fetchAccounts(); }}
        />
      )}
    </div>
  );
}
