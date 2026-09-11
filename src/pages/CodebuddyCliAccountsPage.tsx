import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  WorkbuddyAccount,
  getWorkbuddyAccountDisplayEmail,
  getWorkbuddyPlanBadge,
  getWorkbuddyUsage,
  getWorkbuddyQuotaCategoryGroups,
} from '../types/workbuddy';
import * as workbuddyService from '../services/workbuddyService';
import { injectWorkbuddyToCodebuddyCli, importCodebuddyCliFromLocal } from '../services/codebuddyCliService';
import { useCodebuddyCliAccountStore } from '../stores/useCodebuddyCliAccountStore';
import { useProviderAccountsPage } from '../hooks/useProviderAccountsPage';
import { CodebuddySuiteAccountsSharedView, type CodebuddySuiteAccountsPlatformConfig } from '../components/codebuddy-suite/CodebuddySuiteAccountsSharedView';

const CODEBUDDY_CLI_FLOW_NOTICE_COLLAPSED_KEY = 'agtools.codebuddy_cli.flow_notice_collapsed';
const CODEBUDDY_CLI_CURRENT_ACCOUNT_ID_KEY = 'agtools.codebuddy_cli.current_account_id';

const codebuddyCliPlatformConfig: CodebuddySuiteAccountsPlatformConfig<WorkbuddyAccount> = {
  pageClassName: 'codebuddy-cli-accounts-page',
  quickSettingsType: 'workbuddy',
  searchPlaceholderKey: 'codebuddyCli.search',
  searchPlaceholderDefault: '搜索账号（CodeBuddy CLI）...',
  flowNotice: {
    titleKey: 'codebuddyCli.flowNotice.title',
    titleDefault: 'CodeBuddy CLI 账号管理说明（点击展开/收起）',
    descKey: 'codebuddyCli.flowNotice.desc',
    descDefault: '切换账号将把选中账号的登录态写入 CodeBuddy CLI 官方认证文件（本地明文处理）。',
    permissionKey: 'codebuddyCli.flowNotice.permission',
    permissionDefault: '权限范围：读写 CodeBuddy CLI 认证文件（%LOCALAPPDATA%/CodeBuddyExtension/Data/Public/auth）。',
    networkKey: 'codebuddyCli.flowNotice.network',
    networkDefault: '网络范围：Token 刷新需联网请求 WorkBuddy 服务。不上传本地密钥或凭证。',
  },
  noAccountsKey: 'codebuddyCli.noAccounts',
  noAccountsDefault: '暂无账号，请先在 WorkBuddy 页面添加账号（同一账号体系）',
  addAccountTitleKey: 'codebuddyCli.addAccount',
  addAccountTitleDefault: '添加 CodeBuddy CLI 账号',
  oauthDescKey: 'codebuddyCli.oauthDesc',
  oauthDescDefault: '点击下方按钮将在浏览器中打开 WorkBuddy 授权页面（与 CodeBuddy CLI 同一账号体系）。',
  oauthFeatureCardClassName: 'codebuddy-cli-oauth-feature-card',
  oauthFeatureTitleKey: 'codebuddyCli.oauthFeature.oauth.title',
  oauthFeatureTitleDefault: '仅授权 IDE 登录信息',
  oauthFeatureItem1Key: 'codebuddyCli.oauthFeature.oauth.item1',
  oauthFeatureItem1Default: '在浏览器完成 OAuth 后即可添加账号并用于 CodeBuddy CLI 切换。',
  oauthFeatureItem2Key: 'codebuddyCli.oauthFeature.oauth.item2',
  oauthFeatureItem2Default: '授权完成后会自动刷新资源包配额数据。',
  oauthFeatureItem3Key: 'codebuddyCli.oauthFeature.oauth.item3',
  oauthFeatureItem3Default: '账号卡片将按资源包展示额度、进度和刷新/到期时间。',
  oauthUrlInputPlaceholderKey: 'codebuddyCli.oauthUrlInputPlaceholder',
  oauthUrlInputPlaceholderDefault: '可手动输入授权地址',
  oauthWaitingKey: 'codebuddyCli.oauthWaiting',
  oauthWaitingDefault: '等待授权完成...',
  tokenDescKey: 'codebuddyCli.tokenDesc',
  tokenDescDefault: '粘贴 WorkBuddy 的 access token：',
  importLocalDescKey: 'codebuddyCli.import.localDesc',
  importLocalDescDefault: '支持从本机 WorkBuddy 客户端或 JSON 文件导入账号数据。',
  importLocalClientKey: 'codebuddyCli.import.localClient',
  importLocalClientDefault: '从本机 WorkBuddy 导入',
  getDisplayEmail: (account) => getWorkbuddyAccountDisplayEmail(account),
  getPlanBadge: (account) => getWorkbuddyPlanBadge(account),
  getUsage: (account) => getWorkbuddyUsage(account),
  getQuotaGroups: (account, t) => getWorkbuddyQuotaCategoryGroups(account, t),
  hasQuotaData: (_account, groups) => groups.some((g) => g.items.length > 0),
  usagePrefix: 'workbuddy',
  quotaPrefix: 'workbuddy',
  tableUsageClassName: 'workbuddy-table-usage',
};

export function CodebuddyCliAccountsPage() {
  const { t } = useTranslation();
  const store = useCodebuddyCliAccountStore();
  const [restartHint, setRestartHint] = useState<string | null>(null);

  const page = useProviderAccountsPage<WorkbuddyAccount>({
    platformKey: 'CodeBuddy CLI',
    oauthLogPrefix: 'CodebuddyCliOAuth',
    flowNoticeCollapsedKey: CODEBUDDY_CLI_FLOW_NOTICE_COLLAPSED_KEY,
    currentAccountIdKey: CODEBUDDY_CLI_CURRENT_ACCOUNT_ID_KEY,
    exportFilePrefix: 'codebuddy_cli_accounts',
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
      importFromLocal: importCodebuddyCliFromLocal,
      addWithToken: workbuddyService.addWorkbuddyAccountWithToken,
      exportAccounts: workbuddyService.exportWorkbuddyAccounts,
      injectToVSCode: injectWorkbuddyToCodebuddyCli,
    },
    getDisplayEmail: (account) => getWorkbuddyAccountDisplayEmail(account),
    onInjectSuccess: () => {
      // CLI 只在启动/取 token 时读取认证文件，不监听变更，提示重启后生效。
      setRestartHint(
        t(
          'codebuddyCli.switchRestartHint',
          '切换完成。运行中的 CodeBuddy CLI 需重启后生效（新开的终端窗口会自动生效）。',
        ),
      );
    },
  });

  return (
    <div className={`ghcp-accounts-page ${codebuddyCliPlatformConfig.pageClassName}`}>
      {restartHint && (
        <div className="codebuddy-cli-restart-hint">
          <span>{restartHint}</span>
          <button type="button" onClick={() => setRestartHint(null)}>
            {t('common.close', '关闭')}
          </button>
        </div>
      )}
      <CodebuddySuiteAccountsSharedView
        accounts={store.accounts}
        loading={store.loading}
        page={page}
        platformConfig={codebuddyCliPlatformConfig}
        onRefreshAccounts={() => { store.fetchAccounts(); }}
      />
    </div>
  );
}
