import { useId, type ReactNode, type Ref } from 'react';
import { FolderOpen } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import type { LaunchTerminalOption } from '../hooks/useLaunchTerminalOptions';
import { SingleSelectDropdown } from './SingleSelectDropdown';
import './CliLaunchFields.css';

export interface CliWorkingDirectoryField {
  value: string;
  error?: string | null;
  placeholder?: string;
  hint?: string;
  inputRef?: Ref<HTMLInputElement>;
  onChange: (value: string) => void;
  onBlur?: () => void;
  onChoose: () => void;
}

interface Props {
  workingDir?: CliWorkingDirectoryField;
  terminal: string;
  terminalOptions: LaunchTerminalOption[];
  onTerminalChange: (value: string) => void;
  command?: string;
  commandPlaceholder?: string;
  commandAction?: ReactNode;
  customCommand?: string;
  onCustomCommandChange?: (value: string) => void;
  disabled?: boolean;
}

export function CliLaunchFields({
  workingDir, terminal, terminalOptions, onTerminalChange, command, commandPlaceholder, commandAction,
  customCommand, onCustomCommandChange, disabled,
}: Props) {
  const { t } = useTranslation();
  const id = useId();

  return (
    <div className={workingDir ? 'cli-launch-fields cli-launch-fields-with-directory' : 'cli-launch-fields'}>
      {workingDir && (
        <div className="cli-launch-field">
          <label htmlFor={`${id}-dir`}>{t('instances.form.workingDir')}</label>
          <div className="cli-launch-directory-control">
            <input id={`${id}-dir`} ref={workingDir.inputRef} className="cli-launch-input"
              value={workingDir.value} title={workingDir.value}
              placeholder={workingDir.placeholder ?? t('instances.form.workingDirPlaceholder')}
              disabled={disabled} autoComplete="off" aria-invalid={Boolean(workingDir.error)}
              aria-describedby={workingDir.error || workingDir.hint ? `${id}-dir-note` : undefined}
              onChange={(event) => workingDir.onChange(event.target.value)} onBlur={workingDir.onBlur} />
            <button type="button" className="btn btn-secondary icon-only cli-launch-folder" disabled={disabled}
              onClick={workingDir.onChoose} title={t('instances.launchDialog.chooseDirectory')}
              aria-label={t('instances.launchDialog.chooseDirectory')}><FolderOpen size={16} /></button>
          </div>
          {(workingDir.error || workingDir.hint) && (
            <p id={`${id}-dir-note`} className="cli-launch-note" role={workingDir.error ? 'alert' : undefined}>
              {workingDir.error || workingDir.hint}
            </p>
          )}
        </div>
      )}
      <div className="cli-launch-field">
        <span>{t('instances.launchDialog.terminal')}</span>
        <SingleSelectDropdown value={terminal} options={terminalOptions} onChange={onTerminalChange}
          disabled={disabled} ariaLabel={t('instances.launchDialog.terminal')} />
      </div>
      {(command !== undefined || onCustomCommandChange) && (
        <div className="cli-launch-field cli-launch-command-field">
          <div className="cli-launch-command-heading">
            <label htmlFor={`${id}-command`}>{t('instances.launchDialog.command')}</label>
            {commandAction}
          </div>
          <textarea id={`${id}-command`} className="cli-launch-input" rows={2} spellCheck={false}
            value={onCustomCommandChange ? customCommand : command}
            onChange={onCustomCommandChange ? (event) => onCustomCommandChange(event.target.value) : undefined}
            placeholder={commandPlaceholder} readOnly={!onCustomCommandChange}
            disabled={onCustomCommandChange ? disabled : undefined}
            aria-describedby={onCustomCommandChange ? `${id}-command-note` : undefined} />
          {onCustomCommandChange && (
            <p id={`${id}-command-note`} className="cli-launch-note">{t('instances.launchDialog.customCommandHint')}</p>
          )}
        </div>
      )}
    </div>
  );
}
