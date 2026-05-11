import { useCallback, useEffect, useMemo, useState } from 'react';
import {
  AlertCircle,
  CheckCircle2,
  Edit3,
  LogIn,
  Play,
  Plus,
  RefreshCw,
  ShieldCheck,
  Trash2,
  X,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { SingleSelectDropdown } from '../components/SingleSelectDropdown';
import { PlatformOverviewTabsHeader } from '../components/platform/PlatformOverviewTabsHeader';
import { useLaunchTerminalOptions } from '../hooks/useLaunchTerminalOptions';
import {
  addDevinCliAccount,
  checkDevinCliAuthStatus,
  isDevinCliInstalled,
  listDevinCliAccounts,
  loginDevinCliAccount,
  removeDevinCliAccount,
  renameDevinCliAccount,
  syncAllDevinCliAccounts,
  useDevinCliAccount,
  type DevinCliAccount,
} from '../services/devinCliService';

type MessageTone = 'success' | 'error';

interface PageMessage {
  tone: MessageTone;
  text: string;
}

function formatDateTime(timestamp?: number | null): string {
  if (!timestamp) return '—';
  const date = new Date(timestamp * 1000);
  if (Number.isNaN(date.getTime())) return '—';
  return date.toLocaleString(undefined, {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  });
}

function getAccountSubtitle(account: DevinCliAccount): string {
  return account.email?.trim() || account.orgId?.trim() || account.id.slice(0, 8);
}

function getPlanLabel(account: DevinCliAccount): string {
  const parts = [account.plan, account.tier]
    .map((value) => value?.trim())
    .filter((value): value is string => Boolean(value));
  return parts.length > 0 ? parts.join(' · ') : '—';
}

function isValidAccountName(value: string): boolean {
  return /^[A-Za-z0-9_-]+$/.test(value.trim());
}

export function DevinCliPage() {
  const { t } = useTranslation();
  const [accounts, setAccounts] = useState<DevinCliAccount[]>([]);
  const [loading, setLoading] = useState(true);
  const [running, setRunning] = useState(false);
  const [installed, setInstalled] = useState<boolean | null>(null);
  const [addName, setAddName] = useState('');
  const [message, setMessage] = useState<PageMessage | null>(null);
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState('');
  const { terminalOptions, selectedTerminal, setSelectedTerminal } =
    useLaunchTerminalOptions();

  const readyAccounts = useMemo(
    () => accounts.filter((account) => !account.needsLogin),
    [accounts],
  );

  const sortedAccounts = useMemo(
    () =>
      [...accounts].sort(
        (a, b) =>
          (b.lastUsedAt ?? b.createdAt ?? 0) - (a.lastUsedAt ?? a.createdAt ?? 0),
      ),
    [accounts],
  );

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const [installedResult, accountResult] = await Promise.all([
        isDevinCliInstalled(),
        listDevinCliAccounts(),
      ]);
      setInstalled(installedResult);
      setAccounts(accountResult);
      setMessage(null);
    } catch (error) {
      setMessage({
        tone: 'error',
        text: t('devinCli.messages.loadFailed', 'Failed to load Devin CLI accounts: {{error}}', {
          error: String(error),
        }),
      });
    } finally {
      setLoading(false);
    }
  }, [t]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const handleAddAccount = useCallback(async () => {
    const trimmed = addName.trim();
    if (!trimmed || !isValidAccountName(trimmed) || running) return;
    setRunning(true);
    setMessage(null);
    try {
      const account = await addDevinCliAccount(trimmed);
      setAddName('');
      setMessage({
        tone: 'success',
        text: t('devinCli.messages.accountAdded', 'Account "{{name}}" created. Use Login to authenticate.', {
          name: account.name,
        }),
      });
      await refresh();
    } catch (error) {
      setMessage({
        tone: 'error',
        text: t('devinCli.messages.addFailed', 'Failed to add account: {{error}}', {
          error: String(error),
        }),
      });
    } finally {
      setRunning(false);
    }
  }, [addName, refresh, running, t]);

  const handleRemoveAccount = useCallback(async (id: string) => {
    if (running) return;
    setRunning(true);
    setMessage(null);
    try {
      const removed = await removeDevinCliAccount(id);
      setMessage({
        tone: 'success',
        text: t('devinCli.messages.accountRemoved', 'Account "{{name}}" removed.', { name: removed.name }),
      });
      await refresh();
    } catch (error) {
      setMessage({
        tone: 'error',
        text: t('devinCli.messages.removeFailed', 'Failed to remove account: {{error}}', {
          error: String(error),
        }),
      });
    } finally {
      setRunning(false);
    }
  }, [refresh, running, t]);

  const handleRename = useCallback(async (id: string) => {
    const trimmed = renameValue.trim();
    if (!trimmed || !isValidAccountName(trimmed) || running) return;
    setRunning(true);
    setMessage(null);
    try {
      await renameDevinCliAccount(id, trimmed);
      setRenamingId(null);
      setRenameValue('');
      setMessage({
        tone: 'success',
        text: t('devinCli.messages.accountRenamed', 'Account renamed to "{{name}}".', { name: trimmed }),
      });
      await refresh();
    } catch (error) {
      setMessage({
        tone: 'error',
        text: t('devinCli.messages.renameFailed', 'Failed to rename account: {{error}}', {
          error: String(error),
        }),
      });
    } finally {
      setRunning(false);
    }
  }, [renameValue, refresh, running, t]);

  const handleLogin = useCallback(async (id: string) => {
    if (running) return;
    setRunning(true);
    setMessage(null);
    try {
      const result = await loginDevinCliAccount(id, selectedTerminal);
      setMessage({
        tone: 'success',
        text: t('devinCli.messages.loginStarted', '{{result}} Complete the Devin login flow in the terminal, then refresh.', { result }),
      });
      // Auto-refresh after a delay to pick up auth status changes.
      window.setTimeout(() => { void refresh(); }, 5000);
    } catch (error) {
      setMessage({
        tone: 'error',
        text: t('devinCli.messages.loginFailed', 'Failed to start login: {{error}}', {
          error: String(error),
        }),
      });
    } finally {
      setRunning(false);
    }
  }, [refresh, running, selectedTerminal, t]);

  const handleUse = useCallback(async (id: string) => {
    if (running) return;
    setRunning(true);
    setMessage(null);
    try {
      const result = await useDevinCliAccount(id, [], selectedTerminal);
      setMessage({
        tone: 'success',
        text: t('devinCli.messages.useStarted', '{{result}}', { result }),
      });
    } catch (error) {
      setMessage({
        tone: 'error',
        text: t('devinCli.messages.useFailed', 'Failed to launch Devin: {{error}}', {
          error: String(error),
        }),
      });
    } finally {
      setRunning(false);
    }
  }, [running, selectedTerminal, t]);

  const handleCheckStatus = useCallback(async (id: string) => {
    if (running) return;
    setRunning(true);
    setMessage(null);
    try {
      const status = await checkDevinCliAuthStatus(id);
      if (status.loggedIn) {
        setMessage({
          tone: 'success',
          text: t('devinCli.messages.statusLoggedIn', 'Logged in as {{email}} ({{tier}}, {{plan}})', {
            email: status.email ?? 'unknown',
            tier: status.tier ?? '—',
            plan: status.plan ?? '—',
          }),
        });
      } else {
        setMessage({
          tone: 'error',
          text: t('devinCli.messages.statusNotLoggedIn', 'Not logged in'),
        });
      }
      await refresh();
    } catch (error) {
      setMessage({
        tone: 'error',
        text: t('devinCli.messages.statusCheckFailed', 'Failed to check auth status: {{error}}', {
          error: String(error),
        }),
      });
    } finally {
      setRunning(false);
    }
  }, [refresh, running, t]);

  const handleSyncAll = useCallback(async () => {
    if (running) return;
    setRunning(true);
    setMessage(null);
    try {
      const updated = await syncAllDevinCliAccounts();
      setAccounts(updated);
      setMessage({
        tone: 'success',
        text: t('devinCli.messages.syncDone', 'Synced {{count}} accounts.', { count: updated.length }),
      });
    } catch (error) {
      setMessage({
        tone: 'error',
        text: t('devinCli.messages.syncFailed', 'Sync failed: {{error}}', { error: String(error) }),
      });
    } finally {
      setRunning(false);
    }
  }, [running, t]);

  const addNameTrimmed = addName.trim();
  const addNameInvalid = addNameTrimmed.length > 0 && !isValidAccountName(addNameTrimmed);

  return (
    <div className="ghcp-accounts-page devin-cli-page">
      <PlatformOverviewTabsHeader platform="devin-cli" active="overview" tabs={['overview']} />

      <div className="ghcp-flow-notice" role="note" aria-live="polite">
        <div className="ghcp-flow-notice-toggle">
          <div className="ghcp-flow-notice-title">
            <ShieldCheck size={16} />
            <span>{t('devinCli.notice.title', 'Built-in Devin CLI account switching')}</span>
          </div>
        </div>
        <div className="ghcp-flow-notice-body">
          <div className="ghcp-flow-notice-desc">
            {t(
              'devinCli.notice.desc',
              'Each account gets an isolated profile directory. Credentials stay separate via XDG_DATA_HOME/XDG_CONFIG_HOME environment isolation. Shared CLI state (sessions, logs) is symlinked across profiles.',
            )}
          </div>
          <ul className="ghcp-flow-notice-list">
            <li>
              {t(
                'devinCli.notice.storage',
                'Data: ~/.antigravity_cockpit/devin-cli/accounts.json + profiles/<id>/',
              )}
            </li>
            <li>
              {t(
                'devinCli.notice.runtime',
                'Runtime: Cockpit Tools sets XDG env vars and spawns `devin` with the selected profile\'s isolated environment.',
              )}
            </li>
          </ul>
        </div>
      </div>

      {message && (
        <div className={`message-bar ${message.tone === 'error' ? 'error' : 'success'}`}>
          {message.text}
          <button type="button" onClick={() => setMessage(null)}>
            <X size={14} />
          </button>
        </div>
      )}

      <div className="toolbar">
        <div className="toolbar-left">
          <div className={`status-badge ${installed ? 'success' : 'warning'}`}>
            {installed ? <CheckCircle2 size={14} /> : <AlertCircle size={14} />}
            <span>
              {installed
                ? t('devinCli.status.installed', 'devin CLI installed')
                : t('devinCli.status.notInstalled', 'devin CLI not found')}
            </span>
          </div>
          <SingleSelectDropdown
            value={selectedTerminal}
            options={terminalOptions}
            onChange={setSelectedTerminal}
            ariaLabel={t('devinCli.terminal.ariaLabel', 'Terminal')}
            placeholder={t('devinCli.terminal.placeholder', 'Terminal')}
          />
        </div>

        <div className="toolbar-actions">
          <button className="btn btn-secondary" onClick={refresh} disabled={loading || running}>
            <RefreshCw size={16} className={loading ? 'loading-spinner' : undefined} />
            {t('common.refresh', 'Refresh')}
          </button>
          <button className="btn btn-secondary" onClick={handleSyncAll} disabled={running || !installed}>
            <RefreshCw size={16} />
            {t('devinCli.actions.syncAll', 'Sync status')}
          </button>
        </div>
      </div>

      {!installed && (
        <div className="add-status error">
          <AlertCircle size={16} />
          <span>
            {t(
              'devinCli.installHint',
              'Install Devin CLI first: https://docs.devin.ai/cli',
            )}
          </span>
        </div>
      )}

      <div className="add-section">
        <div className="token-format-group">
          <div className="token-format-label">{t('devinCli.add.title', 'Add Devin account')}</div>
          <p className="section-desc">
            {t(
              'devinCli.add.desc',
              'Enter a profile name. After adding, use Login to authenticate with Devin.',
            )}
          </p>
          <div className="qs-path-control">
            <input
              className="qs-path-input"
              value={addName}
              onChange={(event) => setAddName(event.target.value)}
              placeholder={t('devinCli.add.placeholder', 'work or personal')}
              disabled={running}
              onKeyDown={(event) => {
                if (event.key === 'Enter') void handleAddAccount();
              }}
            />
            <button
              className="btn btn-primary"
              onClick={handleAddAccount}
              disabled={running || addNameInvalid || !addNameTrimmed}
            >
              <Plus size={16} />
              {t('devinCli.actions.add', 'Add')}
            </button>
          </div>
          {addNameInvalid && (
            <div className="add-status error">
              <AlertCircle size={16} />
              <span>
                {t(
                  'devinCli.add.invalidName',
                  'Use only letters, numbers, underscores, and hyphens.',
                )}
              </span>
            </div>
          )}
        </div>
      </div>

      <div className="accounts-summary">
        <div className="summary-card">
          <span className="summary-label">{t('devinCli.summary.total', 'Total profiles')}</span>
          <span className="summary-value">{accounts.length}</span>
        </div>
        <div className="summary-card">
          <span className="summary-label">{t('devinCli.summary.ready', 'Ready')}</span>
          <span className="summary-value">{readyAccounts.length}</span>
        </div>
        <div className="summary-card">
          <span className="summary-label">{t('devinCli.summary.needsLogin', 'Needs login')}</span>
          <span className="summary-value">{accounts.length - readyAccounts.length}</span>
        </div>
      </div>

      {loading ? (
        <div className="loading-state">
          <RefreshCw size={24} className="loading-spinner" />
          <p>{t('common.loading', 'Loading...')}</p>
        </div>
      ) : sortedAccounts.length === 0 ? (
        <div className="empty-state">
          <Play size={40} />
          <h3>{t('devinCli.empty.title', 'No Devin CLI profiles yet')}</h3>
          <p>
            {t(
              'devinCli.empty.desc',
              'Add a profile above, then use Login to authenticate with your Devin account.',
            )}
          </p>
        </div>
      ) : (
        <div className="accounts-grid">
          {sortedAccounts.map((account) => (
            <div key={account.id} className="account-card">
              <div className="card-header">
                <div>
                  {renamingId === account.id ? (
                    <div className="qs-path-control" style={{ marginTop: 0 }}>
                      <input
                        className="qs-path-input"
                        value={renameValue}
                        onChange={(event) => setRenameValue(event.target.value)}
                        onKeyDown={(event) => {
                          if (event.key === 'Enter') void handleRename(account.id);
                          if (event.key === 'Escape') setRenamingId(null);
                        }}
                        autoFocus
                      />
                      <button
                        className="btn btn-sm btn-primary"
                        onClick={() => void handleRename(account.id)}
                        disabled={!renameValue.trim() || !isValidAccountName(renameValue.trim())}
                      >
                        {t('devinCli.actions.save', 'Save')}
                      </button>
                      <button
                        className="btn btn-sm btn-secondary"
                        onClick={() => setRenamingId(null)}
                      >
                        <X size={14} />
                      </button>
                    </div>
                  ) : (
                    <h3>{account.name}</h3>
                  )}
                  <p className="account-email">{getAccountSubtitle(account)}</p>
                </div>
                <span className={`instance-plan-badge ${account.needsLogin ? 'free' : 'pro'}`}>
                  {account.needsLogin
                    ? t('devinCli.account.needsLogin', 'Needs login')
                    : t('devinCli.account.ready', 'Ready')}
                </span>
              </div>
              <div className="quota-display">
                <div className="quota-row">
                  <span>{t('common.shared.columns.plan', 'Plan')}</span>
                  <strong>{getPlanLabel(account)}</strong>
                </div>
                <div className="quota-row">
                  <span>{t('devinCli.account.lastUsed', 'Last used')}</span>
                  <strong>{formatDateTime(account.lastUsedAt)}</strong>
                </div>
                <div className="quota-row">
                  <span>{t('common.shared.columns.createdAt', 'Created Time')}</span>
                  <strong>{formatDateTime(account.createdAt)}</strong>
                </div>
              </div>
              <div className="card-actions">
                <button
                  className="btn btn-sm btn-primary"
                  onClick={() => void handleUse(account.id)}
                  disabled={running || account.needsLogin}
                >
                  <Play size={14} />
                  {t('devinCli.actions.use', 'Use')}
                </button>
                <button
                  className="btn btn-sm btn-secondary"
                  onClick={() => void handleLogin(account.id)}
                  disabled={running}
                >
                  <LogIn size={14} />
                  {t('devinCli.actions.login', 'Login')}
                </button>
                <button
                  className="btn btn-sm btn-secondary"
                  onClick={() => void handleCheckStatus(account.id)}
                  disabled={running || !installed}
                >
                  <RefreshCw size={14} />
                  {t('devinCli.actions.checkStatus', 'Status')}
                </button>
                {renamingId !== account.id && (
                  <>
                    <button
                      className="btn btn-sm btn-secondary"
                      onClick={() => {
                        setRenamingId(account.id);
                        setRenameValue(account.name);
                      }}
                      disabled={running}
                    >
                      <Edit3 size={14} />
                    </button>
                    <button
                      className="btn btn-sm btn-secondary"
                      onClick={() => void handleRemoveAccount(account.id)}
                      disabled={running}
                    >
                      <Trash2 size={14} />
                    </button>
                  </>
                )}
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
