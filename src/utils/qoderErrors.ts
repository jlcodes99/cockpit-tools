import i18n from '../i18n';

export function formatQoderError(error: unknown): string {
  const message = String(error);
  const separator = message.indexOf(':');
  const code = separator === -1 ? message : message.slice(0, separator);
  const detail = separator === -1 ? '' : message.slice(separator + 1).trim();
  switch (code) {
    case 'QODER_PROFILE_UNINITIALIZED':
      return i18n.t('qoder.diagnostics.profileUninitialized', { detail });
    case 'QODER_STORAGE_UNAVAILABLE':
      return i18n.t('qoder.diagnostics.storageUnavailable', { detail });
    case 'QODER_PROFILE_CONFLICT':
      return i18n.t('qoder.diagnostics.profileConflict', { detail });
    case 'QODER_DATA_ROOT_UNAVAILABLE':
      return i18n.t('qoder.diagnostics.dataRootUnavailable');
    case 'QODER_INSTANCE_BUSY':
    case 'QODER_ACCOUNT_BUSY':
      return i18n.t('qoder.diagnostics.busy');
    case 'QODER_REAUTH_REQUIRED':
      return i18n.t('qoder.diagnostics.reauthRequired');
    case 'QODER_OPENAPI_UNAUTHORIZED':
      return i18n.t('qoder.diagnostics.apiUnauthorized');
    case 'QODER_RENEWAL_FAILED':
      return i18n.t('qoder.diagnostics.renewalFailed');
    case 'QODER_SESSION_CHANGED':
      return i18n.t('qoder.diagnostics.sessionChanged');
    case 'QODER_SESSION_ENDPOINT_UNAVAILABLE':
      return i18n.t('qoder.diagnostics.endpointUnavailable');
    case 'QODER_LAUNCH_UNCONFIRMED':
      return i18n.t('qoder.diagnostics.launchUnconfirmed');
    case 'QODER_LAUNCH_FAILED':
      return [i18n.t('qoder.diagnostics.launchUnconfirmed'), detail].filter(Boolean).join(' ');
    case 'QODER_IMPORT_WORKER_FAILED':
    case 'QODER_START_WORKER_FAILED':
      return i18n.t('qoder.diagnostics.workerFailed', { detail });
    case 'QODER_SWITCH_LAUNCH_FAILED':
      return i18n.t('qoder.diagnostics.switchLaunchFailed', { detail: formatQoderError(detail) });
    default:
      return message;
  }
}
