import { useId, useRef, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { writeText } from '@tauri-apps/plugin-clipboard-manager';
import { Check, Copy, Play, RefreshCw, Terminal, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useEscCloseTopmost } from '../hooks/useEscClose';
import { useLaunchTerminalOptions } from '../hooks/useLaunchTerminalOptions';
import { useModalFocusTrap } from '../hooks/useModalFocusTrap';
import { getAntigravityCliLaunchCommand, type AntigravityCliLaunchOptions } from '../services/accountService';
import { formatAntigravitySwitchError } from '../utils/antigravitySwitchError';
import { presentWindowsOperationError } from '../utils/windowsOperationDialog';
import { ANTIGRAVITY_LAUNCH_CLIENT_STORAGE_KEY, previewDurableUiSetting } from '../utils/durableUiSettings';
import { useDurableUiSetting } from '../hooks/useDurableUiSetting';
import { antigravityAppPathTarget, ANTIGRAVITY_CLIENT_LABELS, ANTIGRAVITY_DESKTOP_CLIENTS, getMissingAntigravityPathTarget, type AntigravityLaunchClient } from '../utils/antigravityRuntimeTarget';
import { ModalErrorMessage, useModalErrorState } from './ModalErrorMessage';
import { AntigravityClientIcon } from './AntigravityClientIcon';
import { ClientInitializationStatus } from './ClientInitializationStatus';
import { CliLaunchFields } from './CliLaunchFields';
import { PlatformLaunchPathField } from './PlatformLaunchPathField';
import './ClientLaunchDialog.css';
import './AntigravityLaunchDialog.css';

const LAST_DIRECTORY_KEY = 'agtools.antigravity.cli_working_dir.v1';
const CLIENTS = [...ANTIGRAVITY_DESKTOP_CLIENTS, 'cli'] as const;
type Action = 'launch' | 'switch' | 'copy';

interface Props {
  initialClient: AntigravityLaunchClient;
  ideInstanceId?: string;
  ideInstanceHasBoundAccount?: boolean;
  account?: { label: string; badge: ReactNode };
  renderClientDetails?: (client: AntigravityLaunchClient, disabled: boolean) => ReactNode;
  primaryLabel?: (client: AntigravityLaunchClient) => string;
  canSubmit?: (client: AntigravityLaunchClient) => boolean;
  onClose: () => void;
  onLaunch: (client: AntigravityLaunchClient, options: AntigravityCliLaunchOptions) => Promise<void>;
  onSwitchOnly?: (client: AntigravityLaunchClient) => Promise<void>;
  onPrepareCopy?: () => Promise<void>;
}

export function AntigravityLaunchDialog({
  initialClient, account, ideInstanceId, ideInstanceHasBoundAccount, renderClientDetails, primaryLabel, canSubmit,
  onClose, onLaunch, onSwitchOnly, onPrepareCopy,
}: Props) {
  const { t } = useTranslation();
  const titleId = useId();
  const rememberedClient = useDurableUiSetting(ANTIGRAVITY_LAUNCH_CLIENT_STORAGE_KEY);
  const [selectedClient, setSelectedClient] = useState<AntigravityLaunchClient>();
  const client = selectedClient ?? CLIENTS.find((target) => target === rememberedClient) ?? initialClient;
  const isCli = client === 'cli';
  const initializationId = client === 'antigravity_ide' ? (ideInstanceId ?? (account ? '__default__' : undefined)) : undefined;
  const rememberedDirectory = useDurableUiSetting(LAST_DIRECTORY_KEY);
  const [directoryDraft, setDirectoryDraft] = useState<string>();
  const workingDir = directoryDraft ?? rememberedDirectory ?? '';
  const [customCommand, setCustomCommand] = useState('');
  const [copied, setCopied] = useState(false);
  const { terminalOptions, selectedTerminal, setSelectedTerminal } = useLaunchTerminalOptions(isCli);
  const [busy, setBusy] = useState<Action | null>(null);
  const [pathSaving, setPathSaving] = useState(false);
  const [pathRecovery, setPathRecovery] = useState<{ missingClient: AntigravityLaunchClient; action: Action; run: () => Promise<void>; switched?: boolean }>();
  const pathClient = pathRecovery?.missingClient ?? client;
  const pathTarget = pathClient === 'cli' ? 'antigravity_cli' : antigravityAppPathTarget(pathClient);
  const [directoryInvalid, setDirectoryInvalid] = useState(false);
  const pending = useRef(false);
  const dialog = useRef<HTMLDivElement>(null);
  const directory = useRef<HTMLInputElement>(null);
  const error = useModalErrorState();
  useModalFocusTrap(dialog, true);
  useEscCloseTopmost(true, () => { if (!busy && !pathSaving) onClose(); }, dialog);

  const options = { workingDir: workingDir.trim() || undefined, terminal: selectedTerminal, customCommand };
  const disabled = !!busy || pathSaving;
  const blocked = disabled || canSubmit?.(client) === false;
  const resetFeedback = () => {
    error.clear();
    setDirectoryInvalid(false);
    setCopied(false);
  };

  const selectClient = (target: AntigravityLaunchClient) => {
    if (target === client) return;
    setPathRecovery(undefined);
    setSelectedClient(target);
    if (target !== rememberedClient) {
      previewDurableUiSetting(ANTIGRAVITY_LAUNCH_CLIENT_STORAGE_KEY, target);
    }
    resetFeedback();
  };

  const submit = async (
    action: Action,
    run = () => onLaunch(client, options),
  ) => {
    if (pending.current || canSubmit?.(client) === false) return;
    pending.current = true;
    resetFeedback();
    setBusy(action);
    try {
      if (action === 'copy') {
        const command = await getAntigravityCliLaunchCommand(options);
        await onPrepareCopy?.();
        previewDurableUiSetting(LAST_DIRECTORY_KEY, workingDir.trim());
        try {
          await writeText(command);
        } catch {
          error.report(t('common.shared.export.copyFailed'));
          return;
        }
        setCopied(true);
        return;
      }
      if (action === 'switch') {
        await onSwitchOnly?.(client);
      } else {
        await run();
        if (isCli) previewDurableUiSetting(LAST_DIRECTORY_KEY, workingDir.trim());
      }
      onClose();
    } catch (cause) {
      const code = String(cause);
      const missingClient = getMissingAntigravityPathTarget(cause);
      if (code === 'ANTIGRAVITY_CLI_INVALID_DIRECTORY') {
        setDirectoryInvalid(true);
        requestAnimationFrame(() => directory.current?.focus());
      } else if (isCli && code === 'ANTIGRAVITY_CLI_NOT_FOUND') {
        setPathRecovery({ missingClient: 'cli', action, run });
      } else if (action === 'launch' && missingClient) {
        const switched = code.replace(/^Error: /, '').startsWith('APP_PATH_MISSING_AFTER_SWITCH:');
        setPathRecovery({
          missingClient, action, switched,
          run: switched ? () => invoke('antigravity_launch_current', { runtimeTarget: missingClient }) : run,
        });
      } else if (action === 'launch' && !isCli) {
        if (!presentWindowsOperationError({ error: cause, operation: 'launch_app', retry: run, manualContinue: run, onResolved: onClose })) {
          error.report(formatAntigravitySwitchError(t, cause));
        }
      } else {
        error.report(formatAntigravitySwitchError(t, cause));
      }
    } finally {
      pending.current = false;
      setBusy(null);
    }
  };

  const updateDirectory = (value: string) => {
    resetFeedback();
    setDirectoryDraft(value);
  };

  const chooseDirectory = async () => {
    resetFeedback();
    try {
      const selected = await open({ directory: true, multiple: false, defaultPath: workingDir || undefined });
      if (typeof selected === 'string') updateDirectory(selected);
    } catch (cause) {
      error.report(t('antigravityLaunch.directoryFailed', { error: String(cause) }));
    }
  };

  return createPortal(
    <div className="modal-overlay">
      <div className="modal client-launch-dialog antigravity-launch-dialog" role="dialog" aria-modal="true"
        aria-labelledby={titleId} tabIndex={-1} ref={dialog}>
        <div className="modal-header">
          <span className="client-launch-title-icon" aria-hidden="true"><AntigravityClientIcon client={client} /></span>
          <h2 id={titleId} className="client-launch-heading">
            {t(account ? 'antigravityLaunch.title' : 'instances.launchDialog.title')}
          </h2>
          <button type="button" className="modal-close" disabled={disabled} onClick={onClose} aria-label={t('common.close')}>
            <X />
          </button>
        </div>
        <div className="modal-body">
          {account && <div className="client-launch-account">
            <span>{t('common.shared.columns.account')}</span>
            <strong title={account.label}>{account.label}</strong>
            {account.badge}
          </div>}
          <div className="client-launch-options antigravity-launch-options" role="group" aria-label={t('antigravityLaunch.title')}>
            {CLIENTS.map((target) => (
              <button key={target} type="button" aria-pressed={client === target} disabled={disabled}
                onClick={() => selectClient(target)} title={ANTIGRAVITY_CLIENT_LABELS[target]}>
                <AntigravityClientIcon client={target} />
                <span>{ANTIGRAVITY_CLIENT_LABELS[target]}</span>
              </button>
            ))}
          </div>
          {renderClientDetails?.(client, disabled || !!pathRecovery)}
          {client === 'antigravity_ide' && (account || ideInstanceHasBoundAccount) && (
            <p className="client-launch-hint">{t('antigravityLaunch.ideSharedAccountHint')}</p>
          )}
          <PlatformLaunchPathField
            key={pathTarget}
            target={pathTarget}
            disabled={disabled}
            recovery={!!pathRecovery}
            onSaving={setPathSaving}
            onSaved={() => { if (pathRecovery) void submit(pathRecovery.action, pathRecovery.run); }}
          />
          {initializationId && <ClientInitializationStatus platform="antigravity_ide" targetId={initializationId} />}
          {pathRecovery?.switched && <p className="client-launch-hint">{t('appPath.missing.accountAlreadySwitched')}</p>}
          {isCli && <div className="antigravity-launch-cli">
            <CliLaunchFields
              workingDir={{
                value: workingDir,
                placeholder: t('antigravityLaunch.homeDirectory'),
                hint: t('instances.launchDialog.rememberDirectory'),
                error: directoryInvalid ? t('antigravityLaunch.invalidDirectory') : undefined,
                inputRef: directory,
                onChange: updateDirectory,
                onChoose: () => void chooseDirectory(),
              }}
              terminal={selectedTerminal} terminalOptions={terminalOptions}
              onTerminalChange={setSelectedTerminal}
              customCommand={customCommand} commandPlaceholder="agy"
              onCustomCommandChange={(value) => { setCustomCommand(value); resetFeedback(); }}
              disabled={disabled}
            />
            <p className="client-launch-hint">{t('antigravityLaunch.cliAuth')}</p>
          </div>}
          <ModalErrorMessage message={error.message} scrollKey={error.scrollKey} position="bottom" />
        </div>
        <div className="modal-footer">
          <button type="button" className="btn btn-secondary" disabled={disabled} onClick={onClose}>{t('common.cancel')}</button>
          {isCli && <button type="button" className="btn btn-secondary" disabled={blocked} onClick={() => void submit('copy')}>
            {busy === 'copy' ? <RefreshCw size={16} className="loading-spinner" /> : copied ? <Check size={16} /> : <Copy size={16} />}
            {t(copied ? 'common.copied' : onPrepareCopy ? 'antigravityLaunch.switchAndCopy' : 'common.copy')}
          </button>}
            {onSwitchOnly && <button type="button" className="btn btn-secondary" disabled={blocked} onClick={() => void submit('switch')}>
              {busy === 'switch' && <RefreshCw size={16} className="loading-spinner" />}
              {t('antigravityLaunch.switchOnly')}
            </button>}
            <button type="button" className="btn btn-primary" disabled={blocked}
              onClick={() => void submit('launch', pathRecovery?.action === 'launch' ? pathRecovery.run : undefined)}>
              {busy === 'launch' ? <RefreshCw size={16} className="loading-spinner" /> : isCli ? <Terminal size={16} /> : <Play size={16} />}
              {primaryLabel?.(client) ?? t(isCli ? 'instances.launchDialog.runInTerminal' : 'instances.actions.start')}
            </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
