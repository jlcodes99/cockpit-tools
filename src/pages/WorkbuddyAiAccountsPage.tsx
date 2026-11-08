import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Play } from 'lucide-react';
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
  const { t } = useTranslation();
  const store = useWorkbuddyAiAccountStore();
  const [startingClient, setStartingClient] = useState(false);
  const [startClientMessage, setStartClientMessage] = useState<{ text: string; tone: 'ok' | 'error' } | null>(null);

  const handleStartClient = async () => {
    if (startingClient) return;
    setStartingClient(true);
    setStartClientMessage(null);
    try {
      await workbuddyAiService.startWorkbuddyAiClient();
      setStartClientMessage({
        text: t('workbuddyAi.startClient.success', '客户端已启动，若未弹出请检查任务栏'),
        tone: 'ok',
      });
    } catch (error) {
      setStartClientMessage({
        text: String(error) || t('workbuddyAi.startClient.failed', '启动失败'),
        tone: 'error',
      });
    } finally {
      setStartingClient(false);
    }
  };
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
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 12,
          padding: '10px 20px 0',
        }}
      >
        <button
          type="button"
          onClick={handleStartClient}
          disabled={startingClient}
          style={{
            display: 'inline-flex',
            alignItems: 'center',
            gap: 6,
            padding: '6px 14px',
            borderRadius: 8,
            border: '1px solid rgba(127,127,127,0.35)',
            background: 'transparent',
            color: 'inherit',
            cursor: startingClient ? 'wait' : 'pointer',
            fontSize: 13,
          }}
        >
          <Play size={14} />
          {startingClient
            ? t('workbuddyAi.startClient.starting', '正在启动…')
            : t('workbuddyAi.startClient.label', '启动客户端')}
        </button>
        {startClientMessage && (
          <span
            style={{
              fontSize: 12,
              color: startClientMessage.tone === 'error' ? '#e5534b' : 'inherit',
              opacity: startClientMessage.tone === 'error' ? 1 : 0.75,
            }}
          >
            {startClientMessage.text}
          </span>
        )}
      </div>
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
