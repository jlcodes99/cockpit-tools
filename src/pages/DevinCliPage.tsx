import { useCallback, useEffect, useMemo, useState } from 'react';
import {
  AlertCircle,
  CheckCircle2,
  ListChecks,
  LogIn,
  Play,
  Plus,
  RefreshCw,
  ShieldCheck,
  Terminal,
  X,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { SingleSelectDropdown } from '../components/SingleSelectDropdown';
import { PlatformOverviewTabsHeader } from '../components/platform/PlatformOverviewTabsHeader';
import { useLaunchTerminalOptions } from '../hooks/useLaunchTerminalOptions';
import {
  executeDevinCliCommand,
  isDevinCliSwitcherInstalled,
  listDevinCliAccounts,
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
  return account.email?.trim() || account.orgId?.trim() || account.id;
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
        isDevinCliSwitcherInstalled(),
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

  const runCommand = useCallback(
    async (args: string[], successKey: string, successDefault: string) => {
      if (running) return;
      setRunning(true);
      setMessage(null);
      try {
        const result = await executeDevinCliCommand(args, selectedTerminal);
        setMessage({
          tone: 'success',
          text: t(successKey, successDefault, { result }),
        });
        window.setTimeout(() => {
          void refresh();
        }, 1200);
      } catch (error) {
        setMessage({
          tone: 'error',
          text: t('devinCli.messages.commandFailed', 'Failed to run dsw command: {{error}}', {
            error: String(error),
          }),
        });
      } finally {
        setRunning(false);
      }
    },
    [refresh, running, selectedTerminal, t],
  );

  const addNameTrimmed = addName.trim();
  const addNameInvalid = addNameTrimmed.length > 0 && !isValidAccountName(addNameTrimmed);

  const handleAddAccount = () => {
    if (addNameInvalid) return;
    const args = addNameTrimmed ? ['add', addNameTrimmed] : ['add'];
    void runCommand(
      args,
      'devinCli.messages.addStarted',
      'Opened dsw add in a terminal. Complete the Devin login flow there.',
    );
  };

  return (
    <div className="ghcp-accounts-page devin-cli-page">
      <PlatformOverviewTabsHeader platform="devin-cli" active="overview" tabs={['overview']} />

      <div className="ghcp-flow-notice" role="note" aria-live="polite">
        <div className="ghcp-flow-notice-toggle">
          <div className="ghcp-flow-notice-title">
            <ShieldCheck size={16} />
            <span>{t('devinCli.notice.title', 'Safe Devin account switching through dsw')}</span>
          </div>
        </div>
        <div className="ghcp-flow-notice-body">
          <div className="ghcp-flow-notice-desc">
            {t(
              'devinCli.notice.desc',
              'Cockpit Tools only reads dsw account metadata and launches dsw commands in a terminal. Credentials stay isolated in dsw profile directories.',
            )}
          </div>
          <ul className="ghcp-flow-notice-list">
            <li>
              {t(
                'devinCli.notice.storage',
                'Data source: ~/.dsw/accounts.json or DSW_DATA_HOME/accounts.json.',
              )}
            </li>
            <li>
              {t(
                'devinCli.notice.runtime',
                'Runtime: dsw selects the account and sets the isolated Devin CLI profile environment.',
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
                ? t('devinCli.status.installed', 'dsw installed')
                : t('devinCli.status.notInstalled', 'dsw not found')}
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
          <button
            className="btn btn-secondary"
            onClick={() =>
              runCommand(['list'], 'devinCli.messages.listStarted', 'Opened dsw list in a terminal.')
            }
            disabled={running}
          >
            <ListChecks size={16} />
            {t('devinCli.actions.list', 'List')}
          </button>
          <button
            className="btn btn-secondary"
            onClick={() =>
              runCommand(['quota'], 'devinCli.messages.quotaStarted', 'Opened dsw quota in a terminal.')
            }
            disabled={running}
          >
            <RefreshCw size={16} />
            {t('devinCli.actions.quota', 'Quota')}
          </button>
          <button
            className="btn btn-primary"
            onClick={() =>
              runCommand([], 'devinCli.messages.rotateStarted', 'Opened dsw with quota-aware account rotation.')
            }
            disabled={running}
          >
            <Play size={16} />
            {t('devinCli.actions.rotate', 'Run best account')}
          </button>
        </div>
      </div>

      {!installed && (
        <div className="add-status error">
          <AlertCircle size={16} />
          <span>
            {t(
              'devinCli.installHint',
              'Install Devin Switcher first: npm install -g @itsddvn/dsw',
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
              'Leave the name empty to let dsw infer it after login, or enter a safe profile name.',
            )}
          </p>
          <div className="qs-path-control">
            <input
              className="qs-path-input"
              value={addName}
              onChange={(event) => setAddName(event.target.value)}
              placeholder={t('devinCli.add.placeholder', 'work or personal')}
              disabled={running}
            />
            <button
              className="btn btn-primary"
              onClick={handleAddAccount}
              disabled={running || addNameInvalid}
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
          <Terminal size={40} />
          <h3>{t('devinCli.empty.title', 'No Devin CLI profiles yet')}</h3>
          <p>
            {t(
              'devinCli.empty.desc',
              'Use dsw add to create an isolated Devin CLI profile and log in.',
            )}
          </p>
        </div>
      ) : (
        <div className="accounts-grid">
          {sortedAccounts.map((account) => (
            <div key={account.id} className="account-card">
              <div className="card-header">
                <div>
                  <h3>{account.name}</h3>
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
                  onClick={() =>
                    runCommand(
                      ['use', account.name],
                      'devinCli.messages.useStarted',
                      'Opened dsw use for the selected account.',
                    )
                  }
                  disabled={running || account.needsLogin}
                >
                  <Play size={14} />
                  {t('devinCli.actions.use', 'Use')}
                </button>
                <button
                  className="btn btn-sm btn-secondary"
                  onClick={() =>
                    runCommand(
                      ['login', account.name],
                      'devinCli.messages.loginStarted',
                      'Opened dsw login for the selected account.',
                    )
                  }
                  disabled={running}
                >
                  <LogIn size={14} />
                  {t('devinCli.actions.login', 'Login')}
                </button>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
