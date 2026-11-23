import { useEffect } from 'react';
import { listen } from '@tauri-apps/api/event';
import { ACCOUNTS_CHANGED_EVENT, type AccountSyncEventPayload } from '../utils/accountSyncEvents';
import { CodebuddySuiteAccountsSharedView, type CodebuddySuiteAccountsPlatformConfig } from '../components/codebuddy-suite/CodebuddySuiteAccountsSharedView';
import { WorkbuddyCreditsSummary } from '../components/codebuddy-suite/WorkbuddyCredits';
import { PlatformOverviewTabsHeader } from '../components/platform/PlatformOverviewTabsHeader';
import { useProviderAccountsPage } from '../hooks/useProviderAccountsPage';
import * as service from '../services/workbuddyAiService';
import { useWorkbuddyAiAccountStore } from '../stores/useWorkbuddyAiAccountStore';
import {
  type WorkbuddyAccount,
  getWorkbuddyAccountDisplayEmail,
  getWorkbuddyAiPlanBadge,
  getWorkbuddyQuotaCategoryGroups,
  getWorkbuddyUsage,
} from '../types/workbuddy';
import { getWorkbuddyAiCredits, getWorkbuddyAiPlan } from '../utils/workbuddyCredits';

const config: CodebuddySuiteAccountsPlatformConfig<WorkbuddyAccount> = {
  pageClassName: 'workbuddy-accounts-page',
  accountPlatformKey: 'workbuddy_ai',
  searchPlaceholderKey: 'workbuddyAi.search',
  searchPlaceholderDefault: '',
  flowNotice: {
    titleKey: 'workbuddyAi.flowNotice.title',
    titleDefault: '',
    descKey: 'workbuddyAi.flowNotice.desc',
    descDefault: '',
    permissionKey: 'workbuddyAi.flowNotice.permission',
    permissionDefault: '',
    networkKey: 'workbuddyAi.flowNotice.network',
    networkDefault: '',
  },
  noAccountsKey: 'workbuddyAi.noAccounts',
  noAccountsDefault: '',
  addAccountTitleKey: 'workbuddyAi.addAccount',
  addAccountTitleDefault: '',
  allowOauthRestartOnError: true,
  oauthDescKey: 'workbuddyAi.oauthDesc',
  oauthDescDefault: '',
  oauthFeatureCardClassName: 'workbuddy-oauth-feature-card',
  oauthFeatureTitleKey: 'workbuddyAi.oauthFeature.title',
  oauthFeatureTitleDefault: '',
  oauthFeatureItem1Key: 'workbuddyAi.oauthFeature.signIn',
  oauthFeatureItem1Default: '',
  oauthFeatureItem2Key: 'workbuddyAi.oauthFeature.switch',
  oauthFeatureItem2Default: '',
  oauthFeatureItem3Key: 'workbuddyAi.oauthFeature.credits',
  oauthFeatureItem3Default: '',
  oauthUrlInputPlaceholderKey: 'workbuddy.oauthUrlInputPlaceholder',
  oauthUrlInputPlaceholderDefault: '',
  oauthWaitingKey: 'workbuddy.oauthWaiting',
  oauthWaitingDefault: '',
  tokenDescKey: 'workbuddyAi.tokenDesc',
  tokenDescDefault: '',
  tokenInputSecret: true,
  importLocalDescKey: 'workbuddyAi.importDesc',
  importLocalDescDefault: '',
  importLocalClientKey: 'workbuddyAi.importClient',
  importLocalClientDefault: '',
  getDisplayEmail: getWorkbuddyAccountDisplayEmail,
  getPlanBadge: getWorkbuddyAiPlanBadge,
  getPlanBadgeClass: (_badge, account) => getWorkbuddyAiPlan(account).style,
  getUsage: getWorkbuddyUsage,
  getQuotaGroups: getWorkbuddyQuotaCategoryGroups,
  hasQuotaData: (_account, groups) => groups.some((group) => group.items.length > 0),
  renderQuotaSummary: (account) => {
    const credits = getWorkbuddyAiCredits(account);
    return credits.left == null ? null : (
      <WorkbuddyCreditsSummary credits={credits} account={account} international showBreakdown={credits.hasBreakdown} />
    );
  },
  usagePrefix: 'workbuddy',
  quotaPrefix: 'workbuddy',
  tableUsageClassName: 'workbuddy-table-usage',
};

export function WorkbuddyAiAccountsPage() {
  const store = useWorkbuddyAiAccountStore();
  const { fetchAccounts } = store;
  useEffect(() => {
    const unlisten = listen<AccountSyncEventPayload>(ACCOUNTS_CHANGED_EVENT, ({ payload }) => {
      if (payload.platformId === 'workbuddy_ai' && payload.reason === 'quota') {
        void fetchAccounts();
      }
    });
    return () => { void unlisten.then((stop) => stop()); };
  }, [fetchAccounts]);
  const page = useProviderAccountsPage<WorkbuddyAccount>({
    platformKey: 'workbuddy_ai',
    oauthLogPrefix: 'WorkbuddyAiOAuth',
    flowNoticeCollapsedKey: 'agtools.workbuddy_ai.flow_notice_collapsed',
    exportFilePrefix: 'workbuddy_ai_accounts',
    store,
    oauthService: service,
    dataService: service,
    getDisplayEmail: getWorkbuddyAccountDisplayEmail,
  });

  return (
    <div className="ghcp-accounts-page workbuddy-accounts-page workbuddy-ai-accounts-page">
      <PlatformOverviewTabsHeader platform="workbuddy_ai" active="overview" tabs={['overview']} />
      <CodebuddySuiteAccountsSharedView
        accounts={store.accounts}
        loading={store.loading}
        page={page}
        platformConfig={config}
        onRefreshAccounts={store.fetchAccounts}
      />
    </div>
  );
}
