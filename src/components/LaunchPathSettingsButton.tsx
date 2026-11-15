import { FolderOpen } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { APP_PATH_LABELS, type LaunchPathApp } from '../utils/appLaunchPath';
import { CLI_PATH_LABELS, isCliPathApp, type CliPathApp } from '../utils/cliLaunchPath';

export function LaunchPathSettingsButton({ app, iconOnly }: {
  app: LaunchPathApp | CliPathApp;
  iconOnly?: boolean;
}) {
  const { t } = useTranslation();
  const label = t('appPath.missing.pathLabel', {
    app: isCliPathApp(app) ? CLI_PATH_LABELS[app] : APP_PATH_LABELS[app],
  });
  return (
    <button
      type="button"
      className={`btn btn-secondary${iconOnly ? ' icon-only' : ''}`}
      title={label}
      aria-label={label}
      onClick={() => window.dispatchEvent(new CustomEvent('app-path-missing', {
        detail: { app, retry: { kind: 'configure' } },
      }))}
    >
      <FolderOpen size={16} aria-hidden="true" />
      {!iconOnly && label}
    </button>
  );
}
