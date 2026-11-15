import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { FolderOpen, Monitor, Pencil, Plus, RefreshCw, Terminal, Trash2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { SingleSelectDropdown } from '../SingleSelectDropdown';
import { CodebuddyLaunchDialog } from './CodebuddyLaunchDialog';
import { getCodebuddyAccountDisplayEmail, getCodebuddyPlanBadge, getCodebuddyTierBadgeClass, type CodebuddyAccount } from '../../types/codebuddy';
import {
  listCodebuddyCliInstances,
  removeCodebuddyCliInstance,
  saveCodebuddyCliInstance,
  type CodebuddyCliRegion,
  type CodebuddyCliInstance,
} from '../../services/codebuddyCliService';
import './CodebuddyCliInstances.css';

interface Props {
  region: CodebuddyCliRegion;
  accounts: CodebuddyAccount[];
  accountsLoading: boolean;
  accountsError: string | null;
  fetchAccounts: () => Promise<void>;
  maskAccountText: (value: string) => string;
}

export function CodebuddyInstancesPanel({ children, ...props }: Props & { children: ReactNode }) {
  const { t } = useTranslation();
  const [cli, setCli] = useState(false);

  return (
    <>
      <div className="codebuddy-instance-modes" role="group" aria-label={t('instances.form.launchMode')}>
        <button type="button" className={`btn btn-${cli ? 'secondary' : 'primary'}`} aria-pressed={!cli}
          onClick={() => setCli(false)}>
          <Monitor size={16} aria-hidden="true" />
          {t('instances.form.launchModeApp')}
        </button>
        <button type="button" className={`btn btn-${cli ? 'primary' : 'secondary'}`} aria-pressed={cli}
          onClick={() => setCli(true)}>
          <Terminal size={16} aria-hidden="true" />
          {t('instances.form.launchModeCli')}
        </button>
      </div>
      {cli ? <CodebuddyCliInstances {...props} /> : children}
    </>
  );
}

function CodebuddyCliInstances({
  region, accounts: savedAccounts, accountsLoading, accountsError, fetchAccounts, maskAccountText,
}: Props) {
  const { t } = useTranslation();
  const [savedInstances, setInstances] = useState<CodebuddyCliInstance[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadFailed, setLoadFailed] = useState(false);
  const [selected, setSelected] = useState('');
  const [workingDir, setWorkingDir] = useState('');
  const [launchRequest, setLaunchRequest] = useState<CodebuddyCliInstance>();
  const [editor, setEditor] = useState<{ id?: string; name: string } | null>(null);
  const [deleteId, setDeleteId] = useState<string | null>(null);
  const [pending, setPending] = useState<string | null>(null);
  const [feedback, setFeedback] = useState<{ operation: string; error?: string } | null>(null);
  const pendingRef = useRef(false);
  const editorRef = useRef<HTMLFormElement>(null);

  useEffect(() => {
    let cancelled = false;
    void listCodebuddyCliInstances()
      .then((result) => { if (!cancelled) setInstances(result); })
      .catch(() => { if (!cancelled) setLoadFailed(true); })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, []);

  const accounts = useMemo(
    () => new Map(savedAccounts.map((account) => [account.id, {
      label: maskAccountText(getCodebuddyAccountDisplayEmail(account)),
      plan: getCodebuddyPlanBadge(account),
    }])),
    [savedAccounts, maskAccountText],
  );

  const instances = savedInstances.filter((instance) => instance.account.region === region);
  const selectedAccount = accounts.get(selected);
  const launchAccount = launchRequest && accounts.get(launchRequest.account.accountId);
  const accountOptions = Array.from(accounts, ([value, account]) => ({
    value,
    label: account.label,
    badge: <PlanBadge plan={account.plan} />,
  }));

  async function operate(operation: string, action: () => Promise<void>) {
    if (pendingRef.current) return;
    pendingRef.current = true;
    setPending(operation);
    setFeedback(null);
    try {
      await action();
    } catch (error) {
      setFeedback({ operation, error: String(error) });
    } finally {
      pendingRef.current = false;
      setPending(null);
    }
  }

  function refresh() {
    void fetchAccounts();
    void operate('load', async () => {
      setInstances(await listCodebuddyCliInstances());
      setLoadFailed(false);
    });
  }

  function startEditing(instance?: CodebuddyCliInstance) {
    setFeedback(null);
    if (instance) {
      setSelected(instance.account.accountId);
      setWorkingDir(instance.workingDir);
    } else {
      setSelected('');
      setWorkingDir('');
    }
    setEditor({ id: instance?.id, name: instance?.name ?? '' });
    requestAnimationFrame(() => editorRef.current?.scrollIntoView({ block: 'nearest' }));
  }

  function launch(instance: CodebuddyCliInstance) {
    const accountId = instance.account.accountId;
    if (!accounts.has(accountId)) {
      setFeedback({ operation: instance.id, error: 'accountMissing' });
      return;
    }
    setFeedback(null);
    setLaunchRequest(instance);
  }

  function renderFeedback(operation: string) {
    if (feedback?.operation !== operation) return null;
    const key = feedback.error ? `codebuddyCli.errors.${feedback.error}` : 'codebuddyCli.terminalOpened';
    return (
      <div role={feedback.error ? 'alert' : 'status'} className={feedback.error ? 'codebuddy-cli-error' : 'codebuddy-cli-note'}>
        {t(key, { defaultValue: t('codebuddyCli.errors.launchFailed') })}
      </div>
    );
  }

  return (
    <section className="codebuddy-cli-section" aria-label={t('codebuddyCli.instances')}>
      <div className="codebuddy-cli-heading">
        <h2>{t('codebuddyCli.instances')}</h2>
        <div className="codebuddy-cli-actions">
          <button type="button" className="btn btn-secondary" disabled={!!pending || loading} onClick={refresh}
            title={t('common.refresh')} aria-label={t('common.refresh')}>
            <RefreshCw size={16} className={pending === 'load' ? 'loading-spinner' : undefined} />
          </button>
          <button type="button" className="btn btn-secondary" disabled={!!pending || loading || loadFailed || !accounts.size} onClick={() => startEditing()}>
            <Plus size={16} />{t('codebuddyCli.newInstance')}
          </button>
        </div>
      </div>
      {editor && (
        <form ref={editorRef} className="codebuddy-cli-form" onSubmit={(event) => {
          event.preventDefault();
          if (!selectedAccount || !editor.name.trim() || !workingDir.trim()) return;
          void operate('save', async () => {
            setInstances(await saveCodebuddyCliInstance({
              id: editor.id,
              name: editor.name,
              account: { region, accountId: selected },
              workingDir,
            }));
            setEditor(null);
          });
        }}>
          <h3>{t(editor.id ? 'codebuddyCli.editInstance' : 'codebuddyCli.newInstance')}</h3>
          <label className="codebuddy-cli-field">
            <span>{t('instances.form.name')}</span>
            <input className="codebuddy-cli-input" value={editor.name} required disabled={!!pending}
              aria-invalid={feedback?.error === 'nameRequired'}
              onChange={(event) => setEditor({ ...editor, name: event.target.value })} />
          </label>
          <div className="codebuddy-cli-field">
            <span>{t('codebuddyCli.account')}</span>
            <SingleSelectDropdown value={selected} options={accountOptions} ariaLabel={t('codebuddyCli.account')}
              placeholder={t('codebuddyCli.selectAccount')} disabled={!!pending || accountOptions.length === 0}
              onChange={(value) => { setSelected(value); setFeedback(null); }} />
          </div>
          <div className="codebuddy-cli-field">
            <span>{t('instances.form.workingDir')}</span>
            <div className="codebuddy-cli-directory">
              <input className="codebuddy-cli-input" value={workingDir} required disabled={!!pending}
                aria-label={t('instances.form.workingDir')} title={workingDir}
                aria-invalid={feedback?.error === 'workingDirInvalid'}
                onChange={(event) => { setWorkingDir(event.target.value); setFeedback(null); }} />
              <button type="button" className="btn btn-secondary" disabled={!!pending} onClick={() => void operate('directory', async () => {
                const result = await open({ directory: true, multiple: false });
                if (typeof result === 'string') setWorkingDir(result);
              })}><FolderOpen size={16} />{t('codebuddyCli.chooseDirectory')}</button>
            </div>
          </div>
          {renderFeedback('directory')}
          {renderFeedback('save')}
          <div className="codebuddy-cli-actions">
            <button type="button" className="btn btn-secondary" disabled={!!pending}
              onClick={() => { setEditor(null); setFeedback(null); }}>{t('common.cancel')}</button>
            <button type="submit" className="btn btn-primary" disabled={!!pending || !selectedAccount || !workingDir.trim() || !editor.name.trim()}>
              {pending === 'save' && <RefreshCw size={16} className="loading-spinner" />}
              {t('common.save')}
            </button>
          </div>
        </form>
      )}
      {accountsError && <p role="alert" className="codebuddy-cli-error">{t('codebuddyCli.errors.accountUnreadable')}</p>}
      {!accounts.size && !accountsError && <p className="codebuddy-cli-note">
        {t(accountsLoading ? 'common.loading' : region === 'china' ? 'codebuddyCn.noAccounts' : 'codebuddy.noAccounts')}
      </p>}
      {renderFeedback('load')}
      {loadFailed && feedback?.operation !== 'load' && <p role="alert" className="codebuddy-cli-error">{t('codebuddyCli.errors.storageError')}</p>}
      {loading ? <p className="codebuddy-cli-note">{t('common.loading')}</p> : !loadFailed && !instances.length && <p className="codebuddy-cli-note">{t('codebuddyCli.noInstances')}</p>}
      {instances.map((instance) => {
        const account = accounts.get(instance.account.accountId);
        const label = account?.label ?? t('codebuddyCli.errors.accountMissing');
        return (
          <div className="codebuddy-cli-instance" key={instance.id}>
            <div className="codebuddy-cli-instance-row">
              <div className="codebuddy-cli-instance-info">
                <strong title={instance.name}>{instance.name}</strong>
                <div className="codebuddy-cli-instance-account">
                  <span title={label}>{label}</span>
                  {account && <PlanBadge plan={account.plan} />}
                </div>
                <span className="codebuddy-cli-note" title={instance.workingDir}>{instance.workingDir}</span>
              </div>
              <div className="codebuddy-cli-actions">
                <button type="button" className="btn btn-secondary" disabled={!!pending} onClick={() => launch(instance)}>
                  <Terminal size={16} />
                  {t('codebuddyCli.launch')}
                </button>
                <button type="button" className="btn btn-secondary" disabled={!!pending} onClick={() => startEditing(instance)}
                  title={t('common.edit')} aria-label={t('common.edit')}><Pencil size={16} /></button>
                <button type="button" className="btn btn-secondary" disabled={!!pending} onClick={() => setDeleteId(instance.id)}
                  title={t('common.delete')} aria-label={t('common.delete')}><Trash2 size={16} /></button>
              </div>
            </div>
            {deleteId === instance.id && <div className="codebuddy-cli-confirm">
              <p>{t('codebuddyCli.removeConfirmation')}</p>
              <div className="codebuddy-cli-actions">
                <button type="button" className="btn btn-secondary" disabled={!!pending}
                  onClick={() => { setDeleteId(null); setFeedback(null); }}>{t('common.cancel')}</button>
                <button type="button" className="btn btn-danger" disabled={!!pending} onClick={() => void operate(`delete-${instance.id}`, async () => {
                  setInstances(await removeCodebuddyCliInstance(instance.id));
                  if (editor?.id === instance.id) setEditor(null);
                  setDeleteId(null);
                })}>{t('common.delete')}</button>
              </div>
              {renderFeedback(`delete-${instance.id}`)}
            </div>}
            {renderFeedback(instance.id)}
          </div>
        );
      })}
      {launchRequest && launchAccount && (
        <CodebuddyLaunchDialog account={launchRequest.account} accountLabel={launchAccount.label}
          plan={launchAccount.plan} planClass={getCodebuddyTierBadgeClass(launchAccount.plan)}
          instance={launchRequest}
          onDirectoryRemembered={(directory) => {
            const instanceId = launchRequest.id;
            setInstances((current) => current.map((instance) =>
              instance.id === instanceId ? { ...instance, workingDir: directory } : instance,
            ));
          }}
          onClose={(launched) => {
            setLaunchRequest(undefined);
            if (launched) {
              const operation = launchRequest.id;
              setFeedback({ operation });
            }
          }} />
      )}
    </section>
  );
}

function PlanBadge({ plan }: { plan: string }) {
  return <span className={`tier-badge account-picker-plan ${getCodebuddyTierBadgeClass(plan)}`} title={plan}>{plan}</span>;
}
