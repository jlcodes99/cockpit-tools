import { useCallback, useEffect, useMemo, useState } from 'react';
import { confirm as confirmDialog } from '@tauri-apps/plugin-dialog';
import {
  Check,
  CircleAlert,
  Copy,
  LogIn,
  Play,
  Plus,
  RefreshCw,
  Search,
  Trash2,
  UserCheck,
  X,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import {
  SingleSelectDropdown,
  type SingleSelectOption,
} from '../components/SingleSelectDropdown';
import { useLaunchTerminalOptions } from '../hooks/useLaunchTerminalOptions';
import * as claudeService from '../services/claudeService';
import { useClaudeAccountStore } from '../stores/useClaudeAccountStore';
import {
  getClaudeAccountDisplayName,
  getClaudeLoginModeLabel,
  getClaudePlanBadge,
  getClaudePlanBadgeClass,
  type ClaudeAccount,
} from '../types/claude';

type ClaudeCommandMode = 'login' | 'launch';
type ClaudeLoginMode = 'claudeai' | 'console' | 'email' | 'sso' | 'auth_token';

interface CommandModalState {
  accountId: string;
  title: string;
  hint: string;
  command: string;
  copied: boolean;
  executing: boolean;
  executeMessage: string | null;
  executeError: string | null;
  mode: ClaudeCommandMode;
}

const LOGIN_MODE_OPTIONS: SingleSelectOption[] = [
  { value: 'claudeai', label: 'Claude.ai' },
  { value: 'console', label: 'Console' },
  { value: 'email', label: 'Email Hint' },
  { value: 'sso', label: 'SSO' },
  { value: 'auth_token', label: 'Auth Token' },
];

function formatDateTime(value?: number | null): string {
  if (!value || !Number.isFinite(value)) return '--';
  return new Date(value * 1000).toLocaleString();
}

function formatDateTimeCompact(value?: number | null): string {
  if (!value || !Number.isFinite(value)) return '--';
  return new Intl.DateTimeFormat(undefined, {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  }).format(new Date(value * 1000));
}

function buildAccountSearchText(account: ClaudeAccount): string {
  return [
    account.email,
    account.name,
    account.login_hint_email,
    account.auth_method,
    account.org_name,
    account.subscription_type,
    account.login_mode,
    account.anthropic_base_url,
    account.config_dir,
  ]
    .filter(Boolean)
    .join(' ')
    .toLowerCase();
}

export function ClaudeAccountsPage() {
  const { t } = useTranslation();
  const store = useClaudeAccountStore();
  const { terminalOptions, selectedTerminal, setSelectedTerminal } = useLaunchTerminalOptions();

  const [searchQuery, setSearchQuery] = useState('');
  const [commandModal, setCommandModal] = useState<CommandModalState | null>(null);
  const [createModalOpen, setCreateModalOpen] = useState(false);
  const [createName, setCreateName] = useState('');
  const [createLoginMode, setCreateLoginMode] = useState<ClaudeLoginMode>('claudeai');
  const [createLoginHintEmail, setCreateLoginHintEmail] = useState('');
  const [createBaseUrl, setCreateBaseUrl] = useState('');
  const [createAuthToken, setCreateAuthToken] = useState('');
  const [createDisableNonessentialTraffic, setCreateDisableNonessentialTraffic] = useState(true);
  const [createSubmitting, setCreateSubmitting] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);
  const [pageError, setPageError] = useState<string | null>(null);

  useEffect(() => {
    void Promise.all([
      useClaudeAccountStore.getState().fetchAccounts(),
      useClaudeAccountStore.getState().fetchCurrentAccountId(),
    ]);
  }, []);

  const filteredAccounts = useMemo(() => {
    const query = searchQuery.trim().toLowerCase();
    const accounts = [...store.accounts];
    accounts.sort((left, right) => {
      const leftCurrent = left.id === store.currentAccountId ? 1 : 0;
      const rightCurrent = right.id === store.currentAccountId ? 1 : 0;
      if (leftCurrent !== rightCurrent) {
        return rightCurrent - leftCurrent;
      }
      const leftScore = left.last_used || left.created_at || 0;
      const rightScore = right.last_used || right.created_at || 0;
      return rightScore - leftScore;
    });

    if (!query) return accounts;
    return accounts.filter((account) => buildAccountSearchText(account).includes(query));
  }, [searchQuery, store.accounts, store.currentAccountId]);

  const openCommandModal = useCallback(
    async (account: ClaudeAccount, mode: ClaudeCommandMode) => {
      setPageError(null);
      try {
        const info =
          mode === 'login'
            ? await claudeService.getClaudeLoginCommand(account.id)
            : await claudeService.getClaudeLaunchCommand(account.id);
        setCommandModal({
          accountId: account.id,
          title:
            mode === 'login'
              ? t('claude.command.loginTitle', 'Claude 登录命令')
              : t('claude.command.launchTitle', 'Claude 启动命令'),
          hint:
            mode === 'login'
              ? t(
                  'claude.command.loginHint',
                  'Cockpit 只会在隔离的 CLAUDE_CONFIG_DIR profile 中执行官方登录命令。完成浏览器登录后，回到本页点击“同步状态”。',
                )
              : t(
                  'claude.command.launchHint',
                  account.login_mode === 'auth_token'
                    ? '这个命令会注入当前 profile 的 CLAUDE_CONFIG_DIR 与 ANTHROPIC_* 环境变量启动 Claude CLI，不会修改全局 ~/.claude。'
                    : '这个命令会使用当前 profile 的 CLAUDE_CONFIG_DIR 启动 Claude CLI，不会修改全局 ~/.claude。',
                ),
          command: info.command,
          copied: false,
          executing: false,
          executeMessage: null,
          executeError: null,
          mode,
        });
      } catch (error) {
        setPageError(String(error));
      }
    },
    [t],
  );

  const handleCreateProfile = useCallback(async () => {
    if (createSubmitting) return;
    setCreateSubmitting(true);
    setCreateError(null);
    try {
      if (createLoginMode === 'email' && !createLoginHintEmail.trim()) {
        throw new Error(t('claude.fields.loginHintEmailRequired', 'Email Hint 模式需要填写邮箱。'));
      }
      if (createLoginMode === 'auth_token') {
        if (!createBaseUrl.trim()) {
          throw new Error(
            t('claude.fields.baseUrlRequired', 'Auth Token 模式需要填写 ANTHROPIC_BASE_URL。'),
          );
        }
        if (!createAuthToken.trim()) {
          throw new Error(
            t('claude.fields.authTokenRequired', 'Auth Token 模式需要填写 ANTHROPIC_AUTH_TOKEN。'),
          );
        }
      }
      const account = await claudeService.createClaudeAccount({
        name: createName.trim() || null,
        loginMode: createLoginMode,
        loginHintEmail:
          createLoginMode === 'auth_token' ? null : createLoginHintEmail.trim() || null,
        anthropicBaseUrl:
          createLoginMode === 'auth_token' ? createBaseUrl.trim() || null : null,
        anthropicAuthToken:
          createLoginMode === 'auth_token' ? createAuthToken.trim() || null : null,
        disableNonessentialTraffic:
          createLoginMode === 'auth_token' ? createDisableNonessentialTraffic : null,
      });
      setCreateModalOpen(false);
      setCreateName('');
      setCreateLoginMode('claudeai');
      setCreateLoginHintEmail('');
      setCreateBaseUrl('');
      setCreateAuthToken('');
      setCreateDisableNonessentialTraffic(true);
      await store.fetchAccounts();
      await openCommandModal(account, createLoginMode === 'auth_token' ? 'launch' : 'login');
    } catch (error) {
      setCreateError(String(error));
    } finally {
      setCreateSubmitting(false);
    }
  }, [
    createAuthToken,
    createBaseUrl,
    createDisableNonessentialTraffic,
    createLoginHintEmail,
    createLoginMode,
    createName,
    createSubmitting,
    openCommandModal,
    store,
    t,
  ]);

  const handleRefreshAll = useCallback(async () => {
    try {
      setPageError(null);
      await store.refreshAllTokens();
      await store.fetchAccounts();
    } catch (error) {
      setPageError(String(error));
    }
  }, [store]);

  const handleRefreshAccount = useCallback(
    async (accountId: string) => {
      try {
        setPageError(null);
        await store.refreshToken(accountId);
      } catch (error) {
        setPageError(String(error));
      }
    },
    [store],
  );

  const handleSwitchCurrent = useCallback(
    async (accountId: string) => {
      try {
        setPageError(null);
        await store.switchAccount(accountId);
      } catch (error) {
        setPageError(String(error));
      }
    },
    [store],
  );

  const handleDeleteAccount = useCallback(
    async (account: ClaudeAccount) => {
      const confirmed = await confirmDialog(
        t('claude.delete.confirm', '删除这个 Claude profile？其隔离 config 目录也会一并删除。'),
        {
          title: t('claude.delete.title', '删除 Claude Profile'),
          kind: 'warning',
          okLabel: t('common.delete', '删除'),
          cancelLabel: t('common.cancel', '取消'),
        },
      );
      if (!confirmed) return;
      try {
        setPageError(null);
        await store.deleteAccounts([account.id]);
      } catch (error) {
        setPageError(String(error));
      }
    },
    [store, t],
  );

  const handleCopyCommand = useCallback(async () => {
    if (!commandModal) return;
    try {
      await navigator.clipboard.writeText(commandModal.command);
      setCommandModal((prev) => (prev ? { ...prev, copied: true } : prev));
      window.setTimeout(() => {
        setCommandModal((prev) => (prev ? { ...prev, copied: false } : prev));
      }, 1200);
    } catch {
      setCommandModal((prev) =>
        prev
          ? {
              ...prev,
              executeError: t('common.shared.export.copyFailed', '复制失败，请手动复制'),
            }
          : prev,
      );
    }
  }, [commandModal, t]);

  const handleExecuteCommand = useCallback(async () => {
    if (!commandModal || commandModal.executing) return;
    setCommandModal((prev) =>
      prev ? { ...prev, executing: true, executeError: null, executeMessage: null } : prev,
    );
    try {
      const result =
        commandModal.mode === 'login'
          ? await claudeService.executeClaudeLoginCommand(commandModal.accountId, selectedTerminal)
          : await claudeService.executeClaudeLaunchCommand(
              commandModal.accountId,
              selectedTerminal,
            );
      setCommandModal((prev) =>
        prev
          ? {
              ...prev,
              executing: false,
              executeMessage: result,
            }
          : prev,
      );
    } catch (error) {
      setCommandModal((prev) =>
        prev
          ? {
              ...prev,
              executing: false,
              executeError: String(error),
            }
          : prev,
      );
    }
  }, [commandModal, selectedTerminal]);

  return (
    <main className="main-content accounts-page fade-in">
      <div className="page-heading">
        <div>
          <h1>{t('claude.title', 'Claude Cli')}</h1>
          <p className="page-subtitle">
            {t(
              'claude.subtitle',
              '支持官方 `claude auth login` 与 `ANTHROPIC_*` 环境变量两种模式，并统一通过 `CLAUDE_CONFIG_DIR` 隔离 profile。不会改写全局 `~/.claude`。',
            )}
          </p>
        </div>
      </div>

      <div className="toolbar">
        <div className="toolbar-left">
          <div className="search-box">
            <Search size={16} className="search-icon" />
            <input
              type="text"
              placeholder={t('common.search', '搜索')}
              value={searchQuery}
              onChange={(event) => setSearchQuery(event.target.value)}
            />
          </div>
        </div>
        <div className="toolbar-right">
          <button className="btn btn-secondary" onClick={handleRefreshAll}>
            <RefreshCw size={16} />
            {t('common.refresh', '刷新')}
          </button>
          <button className="btn btn-primary" onClick={() => setCreateModalOpen(true)}>
            <Plus size={16} />
            {t('claude.actions.newProfile', '新建 Profile')}
          </button>
        </div>
      </div>

      <div className="add-status">
        <CircleAlert size={16} />
        <span>
          {t(
            'claude.notice',
            'Claude profile 支持两种认证方式：官方登录命令，或启动时注入 `ANTHROPIC_BASE_URL` / `ANTHROPIC_AUTH_TOKEN`。两者都在独立 profile 目录运行。',
          )}
        </span>
      </div>

      {pageError && <div className="form-error">{pageError}</div>}
      {store.error && <div className="form-error">{store.error}</div>}

      {filteredAccounts.length === 0 ? (
        <div className="empty-state">
          <h3>{t('claude.empty.title', '还没有 Claude Profile')}</h3>
          <p>
            {t(
              'claude.empty.desc',
              '创建一个隔离 profile 后，Cockpit 会帮你生成官方登录命令和启动命令。',
            )}
          </p>
          <button className="btn btn-primary" onClick={() => setCreateModalOpen(true)}>
            <Plus size={16} />
            {t('claude.actions.newProfile', '新建 Profile')}
          </button>
        </div>
      ) : (
        <div className="accounts-grid">
          {filteredAccounts.map((account) => {
            const isCurrent = account.id === store.currentAccountId;
            const planLabel = getClaudePlanBadge(account);
            const planClass = getClaudePlanBadgeClass(account);
            const lastSyncText = formatDateTime(account.last_synced_at);
            const lastSyncCompactText = formatDateTimeCompact(account.last_synced_at);

            return (
              <div
                key={account.id}
                className={`account-card${isCurrent ? ' current' : ''}`}
              >
                <div className="card-top">
                  <div className="card-tags">
                    {isCurrent && (
                      <span className="current-tag">
                        {t('dashboard.current', '当前账户')}
                      </span>
                    )}
                    <span className={`tier-badge ${planClass}`}>{planLabel}</span>
                    <span className={`status-pill ${account.logged_in ? '' : 'warning'}`}>
                      {account.logged_in
                        ? t('claude.status.loggedIn', '已登录')
                        : t('claude.status.loggedOut', '未登录')}
                    </span>
                  </div>
                </div>

                <div className="account-email">{getClaudeAccountDisplayName(account)}</div>

                <div className="card-notes">
                  <div className="notes-text">
                    <strong>{t('claude.fields.loginMode', '登录模式')}:</strong>{' '}
                    {getClaudeLoginModeLabel(account)}
                  </div>
                  <div className="notes-text">
                    <strong>{t('claude.fields.authMethod', '认证方式')}:</strong>{' '}
                    {account.auth_method || '--'}
                  </div>
                  {account.login_mode === 'auth_token' && (
                    <div className="notes-text">
                      <strong>{t('claude.fields.baseUrl', 'Base URL')}:</strong>{' '}
                      {account.anthropic_base_url || '--'}
                    </div>
                  )}
                  {account.login_mode === 'auth_token' && (
                    <div className="notes-text">
                      <strong>{t('claude.fields.nonessentialTraffic', '非必要流量')}:</strong>{' '}
                      {account.disable_nonessential_traffic
                        ? t('common.enabled', '已启用')
                        : t('common.disabled', '已关闭')}
                    </div>
                  )}
                  <div className="notes-text">
                    <strong>{t('claude.fields.subscription', '订阅')}:</strong>{' '}
                    {account.subscription_type || '--'}
                  </div>
                  <div className="notes-text">
                    <strong>{t('claude.fields.org', '组织')}:</strong>{' '}
                    {account.org_name || '--'}
                  </div>
                  <div className="notes-text">
                    <strong>{t('claude.fields.profileDir', 'Profile 目录')}:</strong>{' '}
                    {account.config_dir}
                  </div>
                </div>

                <div className="card-footer">
                  <span
                    className="card-date"
                    title={`${t('claude.fields.lastSync', '上次同步')}: ${lastSyncText}`}
                  >
                    {t('claude.fields.lastSync', '上次同步')}: {lastSyncCompactText}
                  </span>
                  <div className="card-actions">
                    <button
                      className="card-action-btn"
                      onClick={() => void handleRefreshAccount(account.id)}
                      title={t('common.refresh', '刷新')}
                      aria-label={t('common.refresh', '刷新')}
                    >
                      <RefreshCw size={14} />
                    </button>
                    <button
                      className="card-action-btn success"
                      onClick={() => void handleSwitchCurrent(account.id)}
                      disabled={isCurrent}
                      title={
                        isCurrent
                          ? t('claude.actions.current', '当前')
                          : t('claude.actions.setCurrent', '设为当前')
                      }
                      aria-label={
                        isCurrent
                          ? t('claude.actions.current', '当前')
                          : t('claude.actions.setCurrent', '设为当前')
                      }
                    >
                      <UserCheck size={14} />
                    </button>
                    {account.login_mode !== 'auth_token' && (
                      <button
                        className="card-action-btn"
                        onClick={() => void openCommandModal(account, 'login')}
                        title={t('claude.actions.login', '登录')}
                        aria-label={t('claude.actions.login', '登录')}
                      >
                        <LogIn size={14} />
                      </button>
                    )}
                    <button
                      className="card-action-btn success"
                      onClick={() => void openCommandModal(account, 'launch')}
                      title={t('claude.actions.launch', '启动')}
                      aria-label={t('claude.actions.launch', '启动')}
                    >
                      <Play size={14} />
                    </button>
                    <button
                      className="card-action-btn danger"
                      onClick={() => void handleDeleteAccount(account)}
                      title={t('common.delete', '删除')}
                      aria-label={t('common.delete', '删除')}
                    >
                      <Trash2 size={14} />
                    </button>
                  </div>
                </div>
              </div>
            );
          })}
        </div>
      )}

      {createModalOpen && (
        <div className="modal-overlay" onClick={() => setCreateModalOpen(false)}>
          <div className="modal" onClick={(event) => event.stopPropagation()}>
            <div className="modal-header">
              <h2>{t('claude.create.title', '新建 Claude Profile')}</h2>
              <button
                className="modal-close"
                onClick={() => setCreateModalOpen(false)}
                aria-label={t('common.close', '关闭')}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              <div className="form-group">
                <label>{t('claude.fields.profileName', 'Profile 名称（可选）')}</label>
                <input
                  className="form-input"
                  value={createName}
                  onChange={(event) => setCreateName(event.target.value)}
                  placeholder={t('claude.fields.profileNamePlaceholder', '例如：Work / Personal')}
                />
              </div>

              <div className="form-group">
                <label>{t('claude.fields.loginMode', '登录模式')}</label>
                <SingleSelectDropdown
                  value={createLoginMode}
                  onChange={(value) => setCreateLoginMode(value as ClaudeLoginMode)}
                  options={LOGIN_MODE_OPTIONS}
                  ariaLabel={t('claude.fields.loginMode', '登录模式')}
                />
              </div>

              {createLoginMode === 'auth_token' ? (
                <>
                  <div className="form-group">
                    <label>{t('claude.fields.baseUrl', 'ANTHROPIC_BASE_URL')}</label>
                    <input
                      className="form-input"
                      value={createBaseUrl}
                      onChange={(event) => setCreateBaseUrl(event.target.value)}
                      placeholder={t('claude.fields.baseUrlPlaceholder', 'https://api.example.com')}
                    />
                    <p className="form-hint">
                      {t(
                        'claude.fields.baseUrlHelp',
                        '会在启动 Claude CLI 时注入为 `ANTHROPIC_BASE_URL`。',
                      )}
                    </p>
                  </div>

                  <div className="form-group">
                    <label>{t('claude.fields.authToken', 'ANTHROPIC_AUTH_TOKEN')}</label>
                    <textarea
                      className="form-input instance-args-input"
                      value={createAuthToken}
                      onChange={(event) => setCreateAuthToken(event.target.value)}
                      placeholder={t('claude.fields.authTokenPlaceholder', 'sk-...')}
                    />
                    <p className="form-hint">
                      {t(
                        'claude.fields.authTokenHelp',
                        '只保存在 Cockpit 的 Claude profile 记录中，并在启动命令中作为环境变量注入。',
                      )}
                    </p>
                  </div>

                  <label
                    className="checkbox-label"
                    style={{ display: 'flex', alignItems: 'center', gap: 8 }}
                  >
                    <input
                      type="checkbox"
                      checked={createDisableNonessentialTraffic}
                      onChange={(event) =>
                        setCreateDisableNonessentialTraffic(event.target.checked)
                      }
                    />
                    <span>
                      {t(
                        'claude.fields.disableNonessentialTraffic',
                        '设置 `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1`',
                      )}
                    </span>
                  </label>
                </>
              ) : (
                <div className="form-group">
                  <label>{t('claude.fields.loginHintEmail', '邮箱提示（可选）')}</label>
                  <input
                    className="form-input"
                    value={createLoginHintEmail}
                    onChange={(event) => setCreateLoginHintEmail(event.target.value)}
                    placeholder={t('claude.fields.loginHintEmailPlaceholder', 'name@example.com')}
                  />
                  <p className="form-hint">
                    {createLoginMode === 'email'
                      ? t(
                          'claude.fields.loginHintEmailRequired',
                          'Email Hint 模式需要填写邮箱，用于 `claude auth login --email`。',
                        )
                      : t(
                          'claude.fields.loginHintEmailHelp',
                          '会作为登录页预填邮箱，不会直接写入全局 Claude 配置。',
                        )}
                  </p>
                </div>
              )}

              {createError && <div className="form-error">{createError}</div>}
            </div>
            <div className="modal-footer">
              <button
                className="btn btn-secondary"
                onClick={() => setCreateModalOpen(false)}
                disabled={createSubmitting}
              >
                {t('common.cancel', '取消')}
              </button>
              <button
                className="btn btn-primary"
                onClick={() => void handleCreateProfile()}
                disabled={createSubmitting}
              >
                <Plus size={16} />
                {createSubmitting
                  ? t('common.loading', '加载中...')
                  : createLoginMode === 'auth_token'
                    ? t('claude.actions.createAndLaunch', '创建并生成启动命令')
                    : t('claude.actions.createAndLogin', '创建并生成登录命令')}
              </button>
            </div>
          </div>
        </div>
      )}

      {commandModal && (
        <div className="modal-overlay" onClick={() => setCommandModal(null)}>
          <div className="modal modal-lg" onClick={(event) => event.stopPropagation()}>
            <div className="modal-header">
              <h2>{commandModal.title}</h2>
              <button
                className="modal-close"
                onClick={() => setCommandModal(null)}
                aria-label={t('common.close', '关闭')}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              <p className="form-hint">{commandModal.hint}</p>
              <div className="form-group">
                <label>{t('claude.command.command', '命令')}</label>
                <textarea
                  className="form-input instance-args-input"
                  value={commandModal.command}
                  readOnly
                />
              </div>
              <div className="form-group">
                <label>{t('instances.launchDialog.terminal', '终端')}</label>
                <SingleSelectDropdown
                  value={selectedTerminal}
                  onChange={setSelectedTerminal}
                  options={terminalOptions}
                  disabled={commandModal.executing}
                  ariaLabel={t('instances.launchDialog.terminal', '终端')}
                />
              </div>
              {commandModal.executeMessage && (
                <div className="add-status success">
                  <Check size={16} />
                  <span>{commandModal.executeMessage}</span>
                </div>
              )}
              {commandModal.executeError && (
                <div className="form-error">{commandModal.executeError}</div>
              )}
            </div>
            <div className="modal-footer">
              <button className="btn btn-secondary" onClick={() => void handleCopyCommand()}>
                <Copy size={16} />
                {commandModal.copied
                  ? t('common.success', '成功')
                  : t('common.copy', '复制')}
              </button>
              <button
                className="btn btn-primary"
                onClick={() => void handleExecuteCommand()}
                disabled={commandModal.executing}
              >
                <Play size={16} />
                {commandModal.executing
                  ? t('common.loading', '加载中...')
                  : t('claude.command.runInTerminal', '终端执行')}
              </button>
            </div>
          </div>
        </div>
      )}
    </main>
  );
}
