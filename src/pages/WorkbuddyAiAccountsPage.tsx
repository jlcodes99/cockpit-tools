import { PlatformOverviewTabsHeader } from '../components/platform/PlatformOverviewTabsHeader';
import { useWorkbuddyAiAccountStore } from '../stores/useWorkbuddyAiAccountStore';
import * as workbuddyAiService from '../services/workbuddyAiService';
import {
  WorkbuddyAccount,
  getWorkbuddyAccountDisplayEmail,
  getWorkbuddyPlanBadge,
  getWorkbuddyUsage,
  getWorkbuddyQuotaCategoryGroups,
} from '../types/workbuddy';
import { useProviderAccountsPage } from '../hooks/useProviderAccountsPage';
import {
  CodebuddySuiteAccountsSharedView,
  type CodebuddySuiteAccountsPlatformConfig,
} from '../components/codebuddy-suite/CodebuddySuiteAccountsSharedView';

const WORKBUDDY_AI_FLOW_NOTICE_COLLAPSED_KEY = 'agtools.workbuddy_ai.flow_notice_collapsed';
const WORKBUDDY_AI_CURRENT_ACCOUNT_ID_KEY = 'agtools.workbuddy_ai.current_account_id';

const workbuddyAiPlatformConfig: CodebuddySuiteAccountsPlatformConfig<WorkbuddyAccount> = {
  pageClassName: 'workbuddy-ai-accounts-page',
  searchPlaceholderKey: 'workbuddyAi.search',
  searchPlaceholderDefault: 'Search WorkBuddy AI accounts...',
  flowNotice: {
    titleKey: 'workbuddyAi.flowNotice.title',
    titleDefault: 'WorkBuddy AI account management',
    descKey: 'workbuddyAi.flowNotice.desc',
    descDefault: 'WorkBuddy AI accounts are stored separately from the domestic WorkBuddy account pool.',
    permissionKey: 'workbuddyAi.flowNotice.permission',
    permissionDefault: 'Reads and writes only the WorkBuddy AI authentication file.',
    networkKey: 'workbuddyAi.flowNotice.network',
    networkDefault: 'OAuth login and token refresh connect to workbuddy.ai.',
  },
  noAccountsKey: 'workbuddyAi.noAccounts',
  noAccountsDefault: 'No WorkBuddy AI accounts',
  addAccountTitleKey: 'workbuddyAi.addAccount',
  addAccountTitleDefault: 'Add WorkBuddy AI account',
  oauthDescKey: 'workbuddyAi.oauthDesc',
  oauthDescDefault: '每次在独立的 Chrome / Edge 未登录窗口中授权，不影响日常浏览器的登录状态。授权完成或取消后，该窗口将自动关闭。',
  oauthFeatureCardClassName: 'workbuddy-ai-oauth-feature-card',
  oauthFeatureTitleKey: 'workbuddyAi.oauthFeature.oauth.title',
  oauthFeatureTitleDefault: 'WorkBuddy AI OAuth',
  oauthFeatureItem1Key: 'workbuddyAi.oauthFeature.oauth.item1',
  oauthFeatureItem1Default: 'Complete OAuth in the browser to add an account.',
  oauthFeatureItem2Key: 'workbuddyAi.oauthFeature.oauth.item2',
  oauthFeatureItem2Default: 'Token and quota data are refreshed after authorization.',
  oauthFeatureItem3Key: 'workbuddyAi.oauthFeature.oauth.item3',
  oauthFeatureItem3Default: 'Switching writes the WorkBuddy AI client auth file only.',
  oauthUrlInputPlaceholderKey: 'workbuddyAi.oauthUrlInputPlaceholder',
  oauthUrlInputPlaceholderDefault: 'Enter authorization URL manually',
  oauthWaitingKey: 'workbuddyAi.oauthWaiting',
  oauthWaitingDefault: 'Waiting for authorization...',
  tokenDescKey: 'workbuddyAi.tokenDesc',
  tokenDescDefault: 'Paste the WorkBuddy AI access token:',
  importLocalDescKey: 'workbuddyAi.import.localDesc',
  importLocalDescDefault: 'Import account data from the local WorkBuddy AI client or a JSON file.',
  importLocalClientKey: 'workbuddyAi.import.localClient',
  importLocalClientDefault: 'Import from local WorkBuddy AI',
  getDisplayEmail: getWorkbuddyAccountDisplayEmail,
  getPlanBadge: getWorkbuddyPlanBadge,
  getUsage: getWorkbuddyUsage,
  getQuotaGroups: (account, t) => getWorkbuddyQuotaCategoryGroups(account, t),
  hasQuotaData: (_account, groups) => groups.some((group) => group.items.length > 0),
  usagePrefix: 'workbuddy',
  quotaPrefix: 'workbuddy',
  tableUsageClassName: 'workbuddy-table-usage',
};

export function WorkbuddyAiAccountsPage() {
  const store = useWorkbuddyAiAccountStore();
  const page = useProviderAccountsPage<WorkbuddyAccount>({
    platformKey: 'WorkBuddy AI',
    oauthLogPrefix: 'WorkbuddyAiOAuth',
    flowNoticeCollapsedKey: WORKBUDDY_AI_FLOW_NOTICE_COLLAPSED_KEY,
    currentAccountIdKey: WORKBUDDY_AI_CURRENT_ACCOUNT_ID_KEY,
    exportFilePrefix: 'workbuddy_ai_accounts',
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
      startLogin: workbuddyAiService.startWorkbuddyAiOAuthLogin,
      completeLogin: workbuddyAiService.completeWorkbuddyAiOAuthLogin,
      cancelLogin: workbuddyAiService.cancelWorkbuddyAiOAuthLogin,
      openAuthUrl: workbuddyAiService.openWorkbuddyAiOAuthFreshBrowser,
    },
    dataService: {
      importFromJson: workbuddyAiService.importWorkbuddyAiFromJson,
      importFromLocal: workbuddyAiService.importWorkbuddyAiFromLocal,
      addWithToken: workbuddyAiService.addWorkbuddyAiAccountWithToken,
      exportAccounts: workbuddyAiService.exportWorkbuddyAiAccounts,
      injectToVSCode: workbuddyAiService.injectWorkbuddyAiToVSCode,
    },
    getDisplayEmail: getWorkbuddyAccountDisplayEmail,
  });

  return (
    <div className="ghcp-accounts-page workbuddy-ai-accounts-page">
      <PlatformOverviewTabsHeader platform="workbuddy_ai" active="overview" tabs={['overview']} />
      <CodebuddySuiteAccountsSharedView
        accounts={store.accounts}
        loading={store.loading}
        page={page}
        platformConfig={workbuddyAiPlatformConfig}
        onRefreshAccounts={() => { store.fetchAccounts(); }}
      />
    </div>
  );
}
