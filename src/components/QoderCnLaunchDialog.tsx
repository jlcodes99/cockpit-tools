import { useId, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { open } from '@tauri-apps/plugin-dialog';
import { Play, RefreshCw, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useEscCloseTopmost } from '../hooks/useEscClose';
import { useDurableUiSetting } from '../hooks/useDurableUiSetting';
import { useLaunchTerminalOptions } from '../hooks/useLaunchTerminalOptions';
import { useModalFocusTrap } from '../hooks/useModalFocusTrap';
import { usePlatformRuntimeSupport } from '../hooks/usePlatformRuntimeSupport';
import { getQoderPlanBadge, resolveQoderPlanBadgeClass, type QoderAccount } from '../types/qoder';
import { previewDurableUiSetting } from '../utils/durableUiSettings';
import { defaultQoderClient, isQoderClientAvailable, type QoderClient, type QoderRegion } from '../utils/qoderRegion';
import { QoderIcon } from './icons/QoderIcon';
import { ClientInitializationStatus } from './ClientInitializationStatus';
import { CliLaunchFields } from './CliLaunchFields';
import { ModalErrorMessage, useModalErrorState } from './ModalErrorMessage';
import { PlatformLaunchPathField } from './PlatformLaunchPathField';
import './ClientLaunchDialog.css';

const TARGETS = [
  { id: 'desktop', global: 'qoder_desktop', cn: 'qoder_cn', cnLabel: 'Qoder CN' },
  { id: 'ide', global: 'qoder', cn: 'qoder_cn_ide', cnLabel: 'Qoder CN IDE' },
  { id: 'cli', global: 'qoder_cli', cn: 'qoder_cn_cli', cnLabel: 'Qoder CN CLI' },
] as const;

export interface QoderCnLaunchOptions {
  launch: boolean;
  workingDir?: string;
  terminal?: string;
}

export function QoderLaunchPathFields({ region, settingsRows = false }: { region: QoderRegion; settingsRows?: boolean }) {
  const isMac = usePlatformRuntimeSupport('macos-only');
  const isWindows = usePlatformRuntimeSupport('windows-only');
  return TARGETS.filter(target => isQoderClientAvailable(region, target.id, isWindows, isMac))
    .map(target => target[region])
    .map(app => settingsRows
      ? <div className="settings-row" key={app}><PlatformLaunchPathField target={app} /></div>
      : <PlatformLaunchPathField key={app} target={app} />);
}

interface Props {
  region: QoderRegion;
  account: QoderAccount;
  accountLabel: string;
  onClose: () => void;
  onSubmit: (client: QoderClient, options: QoderCnLaunchOptions) => Promise<void>;
  formatError: (error: unknown) => string;
}

export function QoderClientLaunchDialog({ region, account, accountLabel, onClose, onSubmit, formatError }: Props) {
  const { t } = useTranslation();
  const titleId = useId();
  const dialog = useRef<HTMLDivElement>(null);
  const pending = useRef(false);
  const error = useModalErrorState();
  const isMac = usePlatformRuntimeSupport('macos-only');
  const isWindows = usePlatformRuntimeSupport('windows-only');
  const clientKey = `agtools.qoder.${region}.launch_client.v1` as const;
  const directoryKey = `agtools.qoder.${region}.cli_working_dir.v1` as const;
  const rememberedClient = useDurableUiSetting(clientKey);
  const rememberedDirectory = useDurableUiSetting(directoryKey);
  const [selectedClient, setSelectedClient] = useState<QoderClient>();
  const [directoryDraft, setWorkingDir] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [pathSaving, setPathSaving] = useState(false);
  const [recoverPath, setRecoverPath] = useState(false);
  const remembered = TARGETS.find(({ id }) => id === rememberedClient && isQoderClientAvailable(region, id, isWindows, isMac));
  const client = selectedClient ?? remembered?.id ?? defaultQoderClient(region, isWindows, isMac);
  const workingDir = directoryDraft ?? rememberedDirectory ?? '';
  const isCli = client === 'cli';
  const { terminalOptions, selectedTerminal, setSelectedTerminal } = useLaunchTerminalOptions(isCli);
  const target = TARGETS.find(item => item.id === client) ?? TARGETS[0];
  const targetLabel = region === 'cn' ? target.cnLabel : t(`qoder.clientTarget.${client}`);
  const launchPathTarget = target[region];
  const plan = getQoderPlanBadge(account);
  const disabled = Boolean(busy) || pathSaving;
  useModalFocusTrap(dialog, true);
  useEscCloseTopmost(true, () => { if (!disabled) onClose(); }, dialog);

  function selectClient(id: QoderClient) {
    if (id === client) return;
    setSelectedClient(id);
    previewDurableUiSetting(clientKey, id);
    setRecoverPath(false);
    error.clear();
  }

  async function submit() {
    if (pending.current) return;
    pending.current = true;
    error.clear();
    setRecoverPath(false);
    setBusy(true);
    try {
      await onSubmit(client, {
        launch: true, workingDir: isCli ? workingDir.trim() || undefined : undefined,
        terminal: isCli ? selectedTerminal : undefined,
      });
      if (isCli) previewDurableUiSetting(directoryKey, workingDir.trim());
      onClose();
    } catch (cause) {
      const code = String(cause).replace(/^Error: /, '');
      if (code.startsWith('APP_PATH_NOT_FOUND:') || code.startsWith('CLI_PATH_NOT_FOUND:')) {
        setRecoverPath(true);
        error.report(t('appPath.missing.invalidPath', { app: targetLabel }));
      } else if (code === 'QODER_CN_CLI_INVALID_DIRECTORY') {
        error.report(t('antigravityLaunch.invalidDirectory'));
      } else {
        error.report(formatError(cause));
      }
    } finally {
      pending.current = false;
      setBusy(false);
    }
  }

  async function chooseDirectory() {
    try {
      const path = await open({ directory: true, multiple: false, defaultPath: workingDir || undefined });
      if (typeof path === 'string') {
        setWorkingDir(path);
        error.clear();
      }
    } catch (cause) {
      error.report(t('antigravityLaunch.directoryFailed', { error: String(cause) }));
    }
  }

  return createPortal(
    <div className="modal-overlay">
      <div className="modal client-launch-dialog" role="dialog" aria-modal="true" aria-labelledby={titleId}
        tabIndex={-1} ref={dialog}>
        <div className="modal-header">
          <span className="client-launch-title-icon" aria-hidden="true"><QoderIcon region={region} client={client} /></span>
          <h2 id={titleId} className="client-launch-heading">{t('antigravityLaunch.title')}</h2>
          <button className="modal-close" type="button" disabled={disabled} onClick={onClose} aria-label={t('common.close')}><X /></button>
        </div>
        <div className="modal-body">
          <div className="client-launch-account">
            <span>{t('common.shared.columns.account')}</span>
            <strong title={accountLabel}>{accountLabel}</strong>
            <span className={`tier-badge account-picker-plan qoder-cn-launch-plan raw-value ${resolveQoderPlanBadgeClass(plan)}`}
              title={plan}>{plan}</span>
          </div>
          <div className="client-launch-options client-launch-options-equal" role="group" aria-label={t('antigravityLaunch.title')}>
            {TARGETS.map(({ id, cnLabel }) => {
              const name = region === 'cn' ? cnLabel : t(`qoder.clientTarget.${id}`);
              return (
                <button key={id} type="button" aria-pressed={id === client}
                  disabled={disabled || !isQoderClientAvailable(region, id, isWindows, isMac)} title={name}
                  onClick={() => selectClient(id)}>
                  <QoderIcon className="client-launch-option-icon" region={region} client={id} /><span>{name}</span>
                </button>
              );
            })}
          </div>
          <PlatformLaunchPathField key={launchPathTarget} target={launchPathTarget} disabled={disabled}
            recovery={recoverPath} onSaving={setPathSaving}
            onSaved={() => {
              setRecoverPath(false);
              error.clear();
              if (recoverPath) void submit();
            }} />
          {isCli && <CliLaunchFields
            workingDir={{ value: workingDir, placeholder: t('antigravityLaunch.homeDirectory'),
              hint: t('instances.launchDialog.rememberDirectory'), onChange: setWorkingDir,
              onChoose: () => void chooseDirectory() }}
            terminal={selectedTerminal} terminalOptions={terminalOptions} onTerminalChange={setSelectedTerminal} disabled={disabled}
          />}
          <ClientInitializationStatus platform={region === 'cn' ? 'qoder_cn' : 'qoder'} targetId={client} />
          <ModalErrorMessage message={error.message} scrollKey={error.scrollKey} position="bottom" />
        </div>
        <div className="modal-footer">
          <button className="btn btn-secondary" type="button" disabled={disabled} onClick={onClose}>{t('common.cancel')}</button>
          <button className="btn btn-primary" type="button" disabled={disabled} onClick={() => void submit()}>
            {busy ? <RefreshCw size={16} className="loading-spinner" /> : <Play size={16} />}
            {t('antigravityLaunch.switchAndLaunch')}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
