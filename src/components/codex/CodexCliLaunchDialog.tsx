import { useId, useRef, useState, type ReactNode } from "react";
import { Check, ChevronLeft, Copy, Play, RefreshCw, Terminal, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { LaunchTerminalOption } from "../../hooks/useLaunchTerminalOptions";
import { useEscCloseTopmost } from "../../hooks/useEscClose";
import { useModalFocusTrap } from "../../hooks/useModalFocusTrap";
import { CliLaunchFields, type CliWorkingDirectoryField } from "../CliLaunchFields";
import { ModalErrorMessage } from "../ModalErrorMessage";
import "../ClientLaunchDialog.css";
import "./CodexCliLaunchDialog.css";

interface CodexCliLaunchDialogProps {
  subjectLabel: string;
  subjectValue: string;
  subjectBadge?: ReactNode;
  account?: { label: string; badge?: ReactNode };
  displayWorkingDir?: string;
  statusMessage?: string | null;
  workingDir?: CliWorkingDirectoryField;
  terminal: string;
  terminalOptions: LaunchTerminalOption[];
  onTerminalChange: (value: string) => void;
  command: string;
  commandPlaceholder?: string;
  preparing: boolean;
  copied: boolean;
  executing: boolean;
  successMessage?: string | null;
  errorMessage?: string | null;
  onClose: () => void;
  onBack?: () => void;
  onCopy: (customCommand: string) => Promise<void>;
  onExecute: (customCommand: string) => void;
  showCancelButton?: boolean;
}

export function CodexCliLaunchDialog({
  subjectLabel, subjectValue, subjectBadge, account, displayWorkingDir, statusMessage, workingDir, terminal, terminalOptions,
  onTerminalChange, command, commandPlaceholder, preparing, copied, executing, successMessage,
  errorMessage, onClose, onBack, onCopy, onExecute, showCancelButton = false,
}: CodexCliLaunchDialogProps) {
  const { t } = useTranslation();
  const [customCommand, setCustomCommand] = useState('');
  const [editingCommand, setEditingCommand] = useState(false);
  const [copying, setCopying] = useState(false);
  const busy = preparing || executing || copying;
  const title = useId();
  const dialog = useRef<HTMLDivElement>(null);
  useModalFocusTrap(dialog, true);
  useEscCloseTopmost(true, () => { if (!executing && !copying) onClose(); }, dialog);

  async function copyCommand() {
    setCopying(true);
    try {
      await onCopy(editingCommand ? customCommand : '');
    } finally {
      setCopying(false);
    }
  }

  return (
    <div className="modal-overlay">
      <div className="modal client-launch-dialog codex-cli-launch-dialog" role="dialog" aria-modal="true"
        aria-labelledby={title} tabIndex={-1} ref={dialog}>
        <div className="modal-header">
          {onBack && <button type="button" className="btn btn-secondary icon-only" onClick={onBack}
            disabled={executing || copying} title={t('common.back')} aria-label={t('common.back')}><ChevronLeft size={16} /></button>}
          <span className="client-launch-title-icon" aria-hidden="true"><Terminal size={17} /></span>
          <h2 id={title} className="client-launch-heading">{t('instances.launchDialog.title')}</h2>
          <button type="button" className="modal-close" onClick={onClose}
            disabled={executing || copying} aria-label={t('common.close')}><X /></button>
        </div>
        <div className="modal-body">
          <div className="client-launch-account">
            <span>{subjectLabel}</span>
            <strong title={subjectValue}>{subjectValue}</strong>
            {subjectBadge}
          </div>
          {account && <div className="client-launch-account">
            <span>{t('common.shared.columns.account')}</span>
            <strong title={account.label}>{account.label}</strong>
            {account.badge}
          </div>}
          {displayWorkingDir && <div className="client-launch-account">
            <span>{t('instances.form.workingDir')}</span>
            <strong title={displayWorkingDir}>{displayWorkingDir}</strong>
          </div>}
          {statusMessage && <p className="client-launch-hint">{statusMessage}</p>}
          <CliLaunchFields workingDir={workingDir} terminal={terminal} terminalOptions={terminalOptions}
            onTerminalChange={onTerminalChange} command={command}
            commandPlaceholder={editingCommand ? 'codex' : commandPlaceholder} disabled={busy}
            customCommand={customCommand} onCustomCommandChange={editingCommand ? setCustomCommand : undefined}
            commandAction={<button type="button" className="client-launch-text-button" disabled={busy}
              onClick={() => setEditingCommand((value) => !value)}>
              {t(editingCommand ? 'instances.launchDialog.useDefaultCommand' : 'instances.launchDialog.customCommand')}
            </button>} />
          {successMessage && <p className="client-launch-hint" role="status">{successMessage}</p>}
          <ModalErrorMessage message={errorMessage} position="bottom" />
        </div>
        <div className="modal-footer">
          {showCancelButton && <button type="button" className="btn btn-secondary" onClick={onClose}
            disabled={executing || copying}>{t('common.cancel')}</button>}
          <button type="button" className="btn btn-secondary" onClick={() => void copyCommand()} disabled={busy}>
            {copying ? <RefreshCw size={16} className="loading-spinner" /> : copied ? <Check size={16} /> : <Copy size={16} />}
            {t(copied ? 'common.copied' : 'common.copy')}
          </button>
          <button type="button" className="btn btn-primary" onClick={() => onExecute(editingCommand ? customCommand : '')} disabled={busy}>
            {busy ? <RefreshCw size={16} className="loading-spinner" /> : <Play size={16} />}
            {t(busy ? 'common.loading' : 'instances.launchDialog.runInTerminal')}
          </button>
        </div>
      </div>
    </div>
  );
}
