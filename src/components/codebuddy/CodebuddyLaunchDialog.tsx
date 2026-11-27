import { useEffect, useId, useRef, useState, type FormEvent } from 'react';
import { createPortal } from 'react-dom';
import { writeText } from '@tauri-apps/plugin-clipboard-manager';
import { open } from '@tauri-apps/plugin-dialog';
import { Check, Copy, Monitor, RefreshCw, Terminal, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useEscCloseTopmost } from '../../hooks/useEscClose';
import { useLaunchTerminalOptions } from '../../hooks/useLaunchTerminalOptions';
import { useDurableUiSetting } from '../../hooks/useDurableUiSetting';
import { CliLaunchFields } from '../CliLaunchFields';
import { useModalFocusTrap } from '../../hooks/useModalFocusTrap';
import { getCodebuddyCliLaunchCommand, getCodebuddyCliWorkingDir, launchCodebuddyCli, type CodebuddyCliAccountRef, type CodebuddyCliLaunchTarget } from '../../services/codebuddyCliService';
import { readMissingLaunchPathApp } from '../../utils/appLaunchPath';
import { presentWindowsOperationError } from '../../utils/windowsOperationDialog';
import { CODEBUDDY_CN_LAUNCH_MODE_STORAGE_KEY, previewDurableUiSetting } from '../../utils/durableUiSettings';
import { CodebuddyIcon } from '../icons/CodebuddyIcon';
import { ModalErrorMessage } from '../ModalErrorMessage';
import { ClientInitializationStatus } from '../ClientInitializationStatus';
import { PlatformLaunchPathField } from '../PlatformLaunchPathField';
import '../ClientLaunchDialog.css';
import './CodebuddyLaunchDialog.css';

type LaunchMode = 'desktop' | 'cli';

interface Props {
  account: CodebuddyCliAccountRef;
  accountLabel: string;
  plan: string;
  planClass: string;
  initialMode?: LaunchMode;
  onLaunchDesktop?: () => Promise<void>;
  onClose: (launched?: boolean) => void;
}

export function CodebuddyLaunchDialog({
  account, accountLabel, plan, planClass, initialMode = 'desktop', onLaunchDesktop, onClose,
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
  const mode = onLaunchDesktop
    ? selectedMode ?? (rememberedMode === 'cli' || rememberedMode === 'desktop' ? rememberedMode : initialMode)
    : 'cli';
  const [workingDir, setWorkingDir] = useState('');
  const [customCommand, setCustomCommand] = useState('');
  const [commandCopied, setCommandCopied] = useState(false);
  const [loadingDirectory, setLoadingDirectory] = useState(true);
  const [directoryReadFailed, setDirectoryReadFailed] = useState(false);
  const [launching, setLaunching] = useState(false);
  const [pathSaving, setPathSaving] = useState(false);
  const [pathRecovery, setPathRecovery] = useState<{ copy: boolean; revision: number }>();
  const [error, setError] = useState<string>();
  const [invalidDirectory, setInvalidDirectory] = useState(false);
  const isCli = mode === 'cli';
  const pathTarget = isCli ? 'codebuddy_cli' : account.region === 'china' ? 'codebuddy_cn' : 'codebuddy';
  const disabled = launching || pathSaving;
  const { terminalOptions, selectedTerminal, setSelectedTerminal } = useLaunchTerminalOptions(isCli);
  const cliTarget: CodebuddyCliLaunchTarget = {
    kind: 'account', account, workingDir: workingDir.trim() || undefined, rememberWorkingDir: true,
  };
  const directoryPending = isCli && loadingDirectory && !edited.current;
  const title = t('codebuddyCli.launchTitle', { app: account.region === 'china' ? 'CodeBuddy CN' : 'CodeBuddy' });
  const launchLabel = t(isCli ? 'instances.launchDialog.runInTerminal' : 'codebuddyCli.launchDesktop');
  const LaunchIcon = isCli ? Terminal : Monitor;

  useModalFocusTrap(dialog, true);
  useEscCloseTopmost(true, () => { if (!disabled) onClose(); }, dialog);

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

  function selectMode(nextMode: LaunchMode) {
    setSelectedMode(nextMode);
    if (launchModeKey && nextMode !== rememberedMode) {
      previewDurableUiSetting(launchModeKey, nextMode);
    }
    if (nextMode === mode) return;
    setPathRecovery(undefined);
    setError(undefined);
    setInvalidDirectory(false);
    setCommandCopied(false);
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

  async function run(copy: boolean) {
    if (pending.current || directoryPending) return;
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
          setPathRecovery(undefined);
          setCommandCopied(true);
          return;
        }
        await launchCodebuddyCli(cliTarget, selectedTerminal, customCommand);
      } else {
        await onLaunchDesktop?.();
      }
      onClose(true);
    } catch (reason) {
      const code = String(reason).replace(/^Error: /, '');
      if ((isCli && code === 'cliMissing') || readMissingLaunchPathApp(reason) === pathTarget) {
        setPathRecovery((previous) => ({ copy, revision: (previous?.revision ?? 0) + 1 }));
        return;
      }
      setPathRecovery(undefined);
      if (isCli) {
        if (code === 'workingDirInvalid') {
          setInvalidDirectory(true);
          requestAnimationFrame(() => directory.current?.focus());
        } else {
          setError(t(`codebuddyCli.errors.${code}`, { defaultValue: t('codebuddyCli.errors.launchFailed') }));
        }
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
        onSubmit={(event: FormEvent) => { event.preventDefault(); if (!disabled) void run(false); }}
      >
        <div className="modal-header">
          <span className="client-launch-title-icon" aria-hidden="true"><CodebuddyIcon /></span>
          <h2 id={titleId} className="client-launch-heading" title={title}>
            {title}
          </h2>
          <button type="button" className="modal-close" onClick={() => onClose()} disabled={disabled} aria-label={t('common.close')}>
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
            <button type="button" aria-pressed={!isCli} disabled={disabled} onClick={() => selectMode('desktop')}>
              <Monitor size={14} aria-hidden="true" />
              <span title={t('instances.form.launchModeApp')}>{t('instances.form.launchModeApp')}</span>
            </button>
            <button type="button" aria-pressed={isCli} disabled={disabled} onClick={() => selectMode('cli')}>
              <Terminal size={14} aria-hidden="true" />
              <span>CLI</span>
            </button>
          </div>}
          <PlatformLaunchPathField
            key={pathTarget}
            target={pathTarget}
            disabled={disabled}
            recovery={pathRecovery?.revision}
            onSaving={setPathSaving}
            onSaved={() => { if (pathRecovery) void run(pathRecovery.copy); }}
          />
          {isCli && (
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
                disabled={disabled} />
            </div>
          )}
          {!isCli && <ClientInitializationStatus platform={account.region === 'china' ? 'codebuddy_cn' : 'codebuddy'} targetId="__default__" />}
          <ModalErrorMessage
            message={error ?? (isCli && directoryReadFailed
              ? t('codebuddyCli.errors.storageError')
              : undefined)}
            position="bottom"
          />
        </div>
        <div className="modal-footer">
          <button type="button" className="btn btn-secondary" onClick={() => onClose()} disabled={disabled}>{t('common.cancel')}</button>
          {isCli && <button type="button" className="btn btn-secondary" disabled={disabled || directoryPending}
            onClick={() => void run(true)}>
            {commandCopied ? <Check size={16} /> : <Copy size={16} />}
            {t(commandCopied ? 'common.copied' : 'common.copy')}
          </button>}
          <button type="submit" className="btn btn-primary" disabled={disabled || directoryPending}>
            {launching ? <RefreshCw size={16} className="loading-spinner" /> : <LaunchIcon size={16} />}
            {launching ? t('common.loading') : launchLabel}
          </button>
        </div>
      </form>
    </div>,
    document.body,
  );
}
