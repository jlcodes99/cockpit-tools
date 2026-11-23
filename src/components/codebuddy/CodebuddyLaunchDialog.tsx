import { LaunchPathSettingsButton } from '../LaunchPathSettingsButton';
import { useEffect, useId, useRef, useState, type FormEvent } from 'react';
import { createPortal } from 'react-dom';
import { writeText } from '@tauri-apps/plugin-clipboard-manager';
import { open } from '@tauri-apps/plugin-dialog';
import { openUrl } from '@tauri-apps/plugin-opener';
import { Check, Copy, ExternalLink, Monitor, RefreshCw, Terminal, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useEscCloseTopmost } from '../../hooks/useEscClose';
import { useLaunchTerminalOptions } from '../../hooks/useLaunchTerminalOptions';
import { useDurableUiSetting } from '../../hooks/useDurableUiSetting';
import { CliLaunchFields } from '../CliLaunchFields';
import { useModalFocusTrap } from '../../hooks/useModalFocusTrap';
import { getCodebuddyCliLaunchCommand, getCodebuddyCliWorkingDir, isCodebuddyCliAvailable, launchCodebuddyCli, type CodebuddyCliAccountRef, type CodebuddyCliLaunchTarget } from '../../services/codebuddyCliService';
import { presentWindowsOperationError } from '../../utils/windowsOperationDialog';
import { CODEBUDDY_CN_LAUNCH_MODE_STORAGE_KEY, previewDurableUiSetting } from '../../utils/durableUiSettings';
import { CodebuddyIcon } from '../icons/CodebuddyIcon';
import { ModalErrorMessage } from '../ModalErrorMessage';
import '../ClientLaunchDialog.css';
import './CodebuddyLaunchDialog.css';

const INSTALL_COMMAND = 'npm install -g @tencent-ai/codebuddy-code';
const INSTALL_GUIDE = 'https://www.codebuddy.cn/docs/cli/installation';

type LaunchMode = 'desktop' | 'cli';
type CliStatus = 'checking' | 'available' | 'missing' | 'failed';

interface Props {
  account: CodebuddyCliAccountRef;
  accountLabel: string;
  plan: string;
  planClass: string;
  onLaunchDesktop?: () => Promise<void>;
  onClose: (launched?: boolean) => void;
}

export function CodebuddyLaunchDialog({
  account, accountLabel, plan, planClass, onLaunchDesktop, onClose,
}: Props) {
  const { t } = useTranslation();
  const titleId = useId();
  const dialog = useRef<HTMLFormElement>(null);
  const directory = useRef<HTMLInputElement>(null);
  const edited = useRef(false);
  const pending = useRef(false);
  const launchModeKey = account.region === 'china' && onLaunchDesktop ? CODEBUDDY_CN_LAUNCH_MODE_STORAGE_KEY : undefined;
  const rememberedMode = useDurableUiSetting(launchModeKey);
  const [selectedMode, setSelectedMode] = useState<LaunchMode>();
  const mode = onLaunchDesktop ? selectedMode ?? (rememberedMode === 'cli' ? 'cli' : 'desktop') : 'cli';
  const [cliStatus, setCliStatus] = useState<CliStatus>('checking');
  const [cliCheck, setCliCheck] = useState(0);
  const [workingDir, setWorkingDir] = useState('');
  const [customCommand, setCustomCommand] = useState('');
  const [commandCopied, setCommandCopied] = useState(false);
  const [loadingDirectory, setLoadingDirectory] = useState(true);
  const [directoryReadFailed, setDirectoryReadFailed] = useState(false);
  const [launching, setLaunching] = useState(false);
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState<string>();
  const [invalidDirectory, setInvalidDirectory] = useState(false);
  const isCli = mode === 'cli';
  const { terminalOptions, selectedTerminal, setSelectedTerminal } = useLaunchTerminalOptions(isCli);
  const cliTarget: CodebuddyCliLaunchTarget = {
    kind: 'account', account, workingDir: workingDir.trim() || undefined, rememberWorkingDir: true,
  };
  const cliBlocked = isCli && (cliStatus === 'checking' || cliStatus === 'failed' || loadingDirectory);
  const title = t(onLaunchDesktop ? 'codebuddyCli.launchTitle' : 'codebuddyCli.launch');
  const launchLabel = t(isCli && cliStatus === 'missing' ? 'appPath.missing.configurePath' : isCli ? 'instances.launchDialog.runInTerminal' : 'codebuddyCli.launchDesktop');
  const LaunchIcon = isCli ? Terminal : Monitor;

  useModalFocusTrap(dialog, true);
  useEscCloseTopmost(true, () => { if (!launching) onClose(); }, dialog);

  useEffect(() => {
    let active = true;
    void getCodebuddyCliWorkingDir({ region: account.region, accountId: account.accountId })
      .then((saved) => {
        if (active && !edited.current && saved) setWorkingDir(saved);
      })
      .catch(() => { if (active && !edited.current) setDirectoryReadFailed(true); })
      .finally(() => { if (active) setLoadingDirectory(false); });
    return () => { active = false; };
  }, [account.accountId, account.region]);

  useEffect(() => {
    if (!isCli) return;
    const refresh = () => setCliCheck((check) => check + 1);
    window.addEventListener('config-updated', refresh);
    return () => window.removeEventListener('config-updated', refresh);
  }, [isCli]);

  useEffect(() => {
    if (!isCli) return;
    let active = true;
    void isCodebuddyCliAvailable()
      .then((available) => { if (active) setCliStatus(available ? 'available' : 'missing'); })
      .catch(() => { if (active) setCliStatus('failed'); });
    return () => { active = false; };
  }, [isCli, cliCheck]);

  function selectMode(nextMode: LaunchMode) {
    setSelectedMode(nextMode);
    if (launchModeKey && nextMode !== rememberedMode) {
      previewDurableUiSetting(launchModeKey, nextMode);
    }
    if (nextMode === mode) return;
    if (nextMode === 'cli') setCliStatus('checking');
    setError(undefined);
    setInvalidDirectory(false);
    setCopied(false);
    setCommandCopied(false);
  }

  function recheckCli() {
    setCliStatus('checking');
    setCliCheck((check) => check + 1);
    setError(undefined);
    setCopied(false);
  }

  function updateDirectory(value: string) {
    edited.current = true;
    setWorkingDir(value);
    setCommandCopied(false);
    setDirectoryReadFailed(false);
    setInvalidDirectory(false);
    setError(undefined);
  }

  async function chooseDirectory() {
    try {
      const selected = await open({ directory: true, multiple: false, defaultPath: workingDir || undefined });
      if (selected) updateDirectory(selected);
    } catch (reason) {
      setError(t('messages.actionFailed', { action: t('codebuddyCli.chooseDirectory'), error: String(reason) }));
    }
  }

  async function copyInstallCommand() {
    setError(undefined);
    try {
      await writeText(INSTALL_COMMAND);
      setCopied(true);
    } catch (reason) {
      setError(t('messages.actionFailed', { action: t('common.copy'), error: String(reason) }));
    }
  }

  async function openInstallGuide() {
    setError(undefined);
    try {
      await openUrl(INSTALL_GUIDE);
    } catch (reason) {
      setError(t('messages.actionFailed', { action: t('codebuddyCli.installGuide'), error: String(reason) }));
    }
  }

  async function run(copy: boolean) {
    if (pending.current || cliBlocked) return;
    pending.current = true;
    setLaunching(true);
    setError(undefined);
    setInvalidDirectory(false);
    setCommandCopied(false);
    try {
      if (isCli) {
        if (copy) {
          const command = await getCodebuddyCliLaunchCommand(cliTarget, customCommand);
          try {
            await writeText(command);
          } catch {
            setError(t('common.shared.export.copyFailed'));
            return;
          }
          setCommandCopied(true);
          return;
        }
        await launchCodebuddyCli(cliTarget, selectedTerminal, customCommand);
      } else {
        await onLaunchDesktop?.();
      }
      onClose(true);
    } catch (reason) {
      const code = String(reason);
      if (isCli) {
        if (code === 'cliMissing') {
          setCliStatus('missing');
        } else if (code === 'workingDirInvalid') {
          setInvalidDirectory(true);
          requestAnimationFrame(() => directory.current?.focus());
        } else {
          setError(t(`codebuddyCli.errors.${code}`, { defaultValue: t('codebuddyCli.errors.launchFailed') }));
        }
      } else if (code.includes('APP_PATH_NOT_FOUND:')) {
        // The existing application-path dialog owns path selection and retry.
        onClose();
      } else if (onLaunchDesktop && !presentWindowsOperationError({
        error: reason,
        operation: 'launch_app',
        retry: onLaunchDesktop,
        manualContinue: onLaunchDesktop,
        onResolved: () => onClose(true),
      })) {
        setError(t('messages.actionFailed', { action: launchLabel, error: code }));
      }
    } finally {
      pending.current = false;
      setLaunching(false);
    }
  }

  return createPortal(
    <div className="modal-overlay">
      <form
        className="modal client-launch-dialog codebuddy-launch-dialog"
        ref={dialog}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        onSubmit={(event: FormEvent) => { event.preventDefault(); void run(false); }}
      >
        <div className="modal-header">
          <span className="client-launch-title-icon" aria-hidden="true"><CodebuddyIcon /></span>
          <h2 id={titleId} className="client-launch-heading" title={title}>
            {title}
          </h2>
          <button type="button" className="modal-close" onClick={() => onClose()} disabled={launching} aria-label={t('common.close')}>
            <X />
          </button>
        </div>
        <div className="modal-body">
          <div className="client-launch-account">
            <span>{t('codebuddyCli.account')}</span>
            <strong title={accountLabel}>{accountLabel}</strong>
            <span className={`tier-badge account-picker-plan ${planClass}`} title={plan}>{plan}</span>
          </div>
          {onLaunchDesktop && <div className="client-launch-options codebuddy-launch-modes" role="group" aria-label={t('instances.form.launchMode')}>
            <button type="button" aria-pressed={!isCli} disabled={launching} onClick={() => selectMode('desktop')}>
              <Monitor size={14} aria-hidden="true" />
              <span title={t('instances.form.launchModeApp')}>{t('instances.form.launchModeApp')}</span>
            </button>
            <button type="button" aria-pressed={isCli} disabled={launching} onClick={() => selectMode('cli')}>
              <Terminal size={14} aria-hidden="true" />
              <span>CLI</span>
            </button>
          </div>}
          {isCli && <LaunchPathSettingsButton app="codebuddy_cli" />}
          {isCli && cliStatus === 'checking' && (
            <p className="codebuddy-launch-status" role="status">
              <RefreshCw size={15} className="loading-spinner" aria-hidden="true" />
              {t('codebuddyCli.checking')}
            </p>
          )}
          {isCli && cliStatus === 'missing' && (
            <div className="codebuddy-launch-install">
              <p>{t('codebuddyCli.errors.cliMissing')}</p>
              <div className="codebuddy-launch-command">
                <code>{INSTALL_COMMAND}</code>
                <button type="button" className="btn btn-secondary icon-only"
                  aria-label={t(copied ? 'common.copied' : 'common.copy')}
                  title={t(copied ? 'common.copied' : 'common.copy')}
                  onClick={() => void copyInstallCommand()}>
                  {copied ? <Check size={15} /> : <Copy size={15} />}
                </button>
              </div>
              <div className="codebuddy-launch-install-actions">
                <button type="button" className="btn btn-secondary btn-sm" onClick={() => void openInstallGuide()}>
                  <ExternalLink size={14} aria-hidden="true" /> {t('codebuddyCli.installGuide')}
                </button>
                <button type="button" className="btn btn-secondary btn-sm" onClick={recheckCli}>
                  <RefreshCw size={14} aria-hidden="true" /> {t('codebuddyCli.recheck')}
                </button>
              </div>
            </div>
          )}
          {isCli && cliStatus === 'failed' && (
            <div className="codebuddy-launch-install">
              <p role="alert">{t('codebuddyCli.errors.cliCheckFailed')}</p>
              <button type="button" className="btn btn-secondary btn-sm" onClick={recheckCli}>
                <RefreshCw size={14} aria-hidden="true" /> {t('codebuddyCli.recheck')}
              </button>
            </div>
          )}
          {isCli && cliStatus === 'available' && (
            <div className="codebuddy-launch-cli-fields">
              <CliLaunchFields workingDir={{
                value: workingDir, inputRef: directory,
                placeholder: t(loadingDirectory ? 'common.loading' : 'codebuddyCli.homeDirectory'),
                hint: t('codebuddyCli.rememberDirectory'),
                error: invalidDirectory ? t('codebuddyCli.errors.workingDirInvalid') : undefined,
                onChange: updateDirectory, onChoose: () => void chooseDirectory(),
              }}
                terminal={selectedTerminal} terminalOptions={terminalOptions} onTerminalChange={setSelectedTerminal}
                customCommand={customCommand} commandPlaceholder="codebuddy"
                onCustomCommandChange={(value) => { setCustomCommand(value); setCommandCopied(false); setError(undefined); }}
                disabled={launching} />
            </div>
          )}
          <ModalErrorMessage
            message={error ?? (isCli && cliStatus === 'available' && directoryReadFailed
              ? t('codebuddyCli.errors.storageError')
              : undefined)}
            position="bottom"
          />
        </div>
        <div className="modal-footer">
          <button type="button" className="btn btn-secondary" onClick={() => onClose()} disabled={launching}>{t('common.cancel')}</button>
          {isCli && cliStatus === 'available' && <button type="button" className="btn btn-secondary" disabled={launching || cliBlocked}
            onClick={() => void run(true)}>
            {commandCopied ? <Check size={16} /> : <Copy size={16} />}
            {t(commandCopied ? 'common.copied' : 'common.copy')}
          </button>}
          <button type="submit" className="btn btn-primary" disabled={launching || cliBlocked}>
            {launching ? <RefreshCw size={16} className="loading-spinner" /> : <LaunchIcon size={16} />}
            {launching ? t('common.loading') : launchLabel}
          </button>
        </div>
      </form>
    </div>,
    document.body,
  );
}
