import { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  PlatformOverviewTabsHeader,
  type PlatformOverviewTab,
} from '../components/platform/PlatformOverviewTabsHeader';
import {
  CodebuddySuiteAccountsSharedView,
  type CodebuddySuiteAccountsPlatformConfig,
} from '../components/codebuddy-suite/CodebuddySuiteAccountsSharedView';
import { useProviderAccountsPage } from '../hooks/useProviderAccountsPage';
import * as zcodeService from '../services/zcodeService';
import { useZcodeAccountStore } from '../stores/useZcodeAccountStore';
import {
  getZcodeAccountDisplayEmail,
  getZcodePlanBadge,
  getZcodeQuotaGroups,
  getZcodeUsage,
  hasZcodeQuotaData,
  type ZcodeAccount,
} from '../types/zcode';
import { compareCurrentAccountFirst } from '../utils/currentAccountSort';
import { ZcodeInstancesContent } from './ZcodeInstancesPage';

const FLOW_NOTICE_KEY = 'agtools.zcode.flow_notice_collapsed';

export function ZcodeAccountsPage() {
  const { t } = useTranslation();
  const [activeTab, setActiveTab] = useState<PlatformOverviewTab>('overview');
  const [oauthProvider, setOauthProvider] = useState<zcodeService.ZcodeOAuthProvider>('zai');
  const [apiKeyProvider, setApiKeyProvider] = useState<zcodeService.ZcodeApiKeyProvider>('zai');
  const store = useZcodeAccountStore();

  const page = useProviderAccountsPage<ZcodeAccount>({
    platformKey: 'zcode',
    oauthLogPrefix: 'ZcodeOAuth',
    flowNoticeCollapsedKey: FLOW_NOTICE_KEY,
    exportFilePrefix: 'zcode_accounts',
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
      startLogin: () => zcodeService.startZcodeOAuthLogin(oauthProvider),
      completeLogin: zcodeService.completeZcodeOAuthLogin,
      cancelLogin: zcodeService.cancelZcodeOAuthLogin,
      submitCallbackUrl: zcodeService.submitZcodeOAuthCallbackUrl,
      openAuthUrl: zcodeService.openZcodeOAuthWindow,
    },
    dataService: {
      importFromJson: zcodeService.importZcodeFromJson,
      importFromLocal: zcodeService.importZcodeFromLocal,
      addWithToken: (apiKey) => zcodeService.importZcodeApiKey(apiKey, apiKeyProvider),
      exportAccounts: zcodeService.exportZcodeAccounts,
      injectToVSCode: zcodeService.injectZcodeAccount,
    },
    getDisplayEmail: getZcodeAccountDisplayEmail,
    resolveOauthSuccessMessage: () => t('zcode.oauth.success', 'ZCode OAuth 登录成功'),
  });

  const previousOauthProviderRef = useRef(oauthProvider);
  useEffect(() => {
    if (previousOauthProviderRef.current === oauthProvider) return;
    previousOauthProviderRef.current = oauthProvider;
    if (page.showAddModal && page.addTab === 'oauth') {
      page.handleRetryOauth();
    }
  }, [oauthProvider, page.addTab, page.handleRetryOauth, page.showAddModal]);

  const accountsForInstances = useMemo(
    () =>
      [...store.accounts].sort((left, right) => {
        const current = compareCurrentAccountFirst(left.id, right.id, store.currentAccountId);
        if (current !== 0) return current;
        const diff = right.created_at - left.created_at;
        return page.sortDirection === 'desc' ? diff : -diff;
      }),
    [page.sortDirection, store.accounts, store.currentAccountId],
  );

  const platformConfig: CodebuddySuiteAccountsPlatformConfig<ZcodeAccount> = {
    pageClassName: 'zcode-accounts-page',
    quickSettingsType: 'zcode',
    searchPlaceholderKey: 'zcode.search',
    searchPlaceholderDefault: '搜索 ZCode 账号...',
    flowNotice: {
      titleKey: 'zcode.flowNotice.title',
      titleDefault: 'ZCode 账号管理说明',
      descKey: 'zcode.flowNotice.desc',
      descDefault: 'Cockpit 按 ZCode 官方格式管理 OAuth 凭据与 API Key，用于真实切号与实例绑定。',
      permissionKey: 'zcode.flowNotice.permission',
      permissionDefault: '本地范围：~/.zcode/v2/credentials.json、config.json、setting.json 与受管实例目录。',
      networkKey: 'zcode.flowNotice.network',
      networkDefault: '网络范围：OAuth、用户信息、订阅与配额接口；不会上传账号到 Cockpit 服务。',
    },
    noAccountsKey: 'zcode.empty',
    noAccountsDefault: '暂无 ZCode 账号',
    addAccountTitleKey: 'zcode.addAccount',
    addAccountTitleDefault: '添加 ZCode 账号',
    oauthDescKey: 'zcode.oauth.desc',
    oauthDescDefault: '关闭 ZCode 后，在 Cockpit 授权窗口完成登录；回调将由 Cockpit 直接接收。',
    oauthFeatureCardClassName: 'zcode-oauth-feature-card',
    oauthFeatureTitleKey: 'zcode.oauth.title',
    oauthFeatureTitleDefault: 'ZCode OAuth',
    oauthFeatureItem1Key: 'zcode.oauth.item1',
    oauthFeatureItem1Default: '支持 Z.ai 与 BigModel 官方登录。',
    oauthFeatureItem2Key: 'zcode.oauth.item2',
    oauthFeatureItem2Default: '授权后自动保存账号并刷新套餐与模型额度。',
    oauthFeatureItem3Key: 'zcode.oauth.item3',
    oauthFeatureItem3Default: '账号可用于默认客户端切号和多开实例绑定。',
    oauthUrlInputPlaceholderKey: 'zcode.oauth.urlPlaceholder',
    oauthUrlInputPlaceholderDefault: 'ZCode OAuth 授权地址',
    oauthWaitingKey: 'zcode.oauth.waiting',
    oauthWaitingDefault: '等待 ZCode OAuth 回调...',
    oauthOpenButtonKey: 'zcode.oauth.openWindow',
    oauthOpenButtonDefault: '打开授权窗口',
    showOauthIncognitoOpenButton: true,
    tokenTabLabelKey: 'zcode.apiKey.tab',
    tokenTabLabelDefault: 'API Key',
    tokenDescKey: 'zcode.apiKey.desc',
    tokenDescDefault: '添加 ZCode 官方支持的 Z.ai 或 BigModel API Key。切号和多开实例会写入各自的 config.json。',
    tokenInputPlaceholderKey: 'zcode.apiKey.placeholder',
    tokenInputPlaceholderDefault: '粘贴 API Key',
    tokenSubmitLabelKey: 'zcode.apiKey.add',
    tokenSubmitLabelDefault: '添加 API Key',
    tokenInputSecret: true,
    importLocalDescKey: 'zcode.import.localDesc',
    importLocalDescDefault: '从本机 ZCode 加密凭据或 JSON 文件导入账号。',
    importLocalClientKey: 'zcode.import.localClient',
    importLocalClientDefault: '从本机 ZCode 导入',
    getDisplayEmail: getZcodeAccountDisplayEmail,
    getPlanBadge: getZcodePlanBadge,
    getSearchText: (account) =>
      [
        getZcodeAccountDisplayEmail(account),
        account.display_name,
        account.user_id,
        account.provider,
        getZcodePlanBadge(account),
      ]
        .filter(Boolean)
        .join(' '),
    getUsage: getZcodeUsage,
    getQuotaGroups: getZcodeQuotaGroups,
    hasQuotaData: (account) => hasZcodeQuotaData(account),
    usagePrefix: 'zcode',
    quotaPrefix: 'zcode',
    tableUsageClassName: 'zcode-table-usage',
    showMfaQuickCode: false,
    tokenControl: (
      <div className="zcode-oauth-provider-control" role="group" aria-label={t('zcode.apiKey.provider', 'API Key 服务')}>
        <button
          type="button"
          className={`btn btn-secondary ${apiKeyProvider === 'zai' ? 'active' : ''}`}
          aria-pressed={apiKeyProvider === 'zai'}
          onClick={() => setApiKeyProvider('zai')}
        >
          Z.ai
        </button>
        <button
          type="button"
          className={`btn btn-secondary ${apiKeyProvider === 'bigmodel' ? 'active' : ''}`}
          aria-pressed={apiKeyProvider === 'bigmodel'}
          onClick={() => setApiKeyProvider('bigmodel')}
        >
          BigModel
        </button>
      </div>
    ),
    oauthProviderControl: (
      <div className="zcode-oauth-provider-control" role="group" aria-label={t('zcode.oauth.provider', '登录服务')}>
        <button
          type="button"
          className={`btn btn-secondary ${oauthProvider === 'zai' ? 'active' : ''}`}
          aria-pressed={oauthProvider === 'zai'}
          onClick={() => setOauthProvider('zai')}
        >
          Z.ai
        </button>
        <button
          type="button"
          className={`btn btn-secondary ${oauthProvider === 'bigmodel' ? 'active' : ''}`}
          aria-pressed={oauthProvider === 'bigmodel'}
          onClick={() => setOauthProvider('bigmodel')}
        >
          BigModel
        </button>
      </div>
    ),
  };

  return (
    <div className="ghcp-accounts-page zcode-accounts-page">
      <PlatformOverviewTabsHeader platform="zcode" active={activeTab} onTabChange={setActiveTab} />
      {activeTab === 'instances' ? (
        <ZcodeInstancesContent accountsForSelect={accountsForInstances} />
      ) : (
        <CodebuddySuiteAccountsSharedView
          accounts={store.accounts}
          loading={store.loading}
          page={page}
          platformConfig={platformConfig}
          onRefreshAccounts={() => void store.fetchAccounts()}
        />
      )}
    </div>
  );
}
