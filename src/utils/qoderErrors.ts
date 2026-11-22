import i18n from '../i18n';

export function formatQoderError(error: unknown): string {
  const message = String(error);
  const separator = message.indexOf(':');
  const code = separator === -1 ? message : message.slice(0, separator);
  const detail = separator === -1 ? '' : message.slice(separator + 1).trim();
  switch (code) {
    case 'APP_PATH_NOT_FOUND':
      if (detail === 'qoder_cn') {
        return i18n.t('qoder.diagnostics.qoderCnPathMissing');
      }
      return message;
    case 'QODER_PROFILE_UNINITIALIZED':
      return i18n.t('qoder.diagnostics.profileUninitialized', { detail });
    case 'QODER_STORAGE_UNAVAILABLE':
    case 'QODER_NATIVE_STORAGE':
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
    case 'QODER_PLATFORM_MISMATCH':
      return i18n.t('qoder.diagnostics.platformMismatch');
    case 'QODER_CN_RELAUNCH_FAILED':
      return i18n.t('qoder.diagnostics.cnRelaunchFailed', { detail });
    case 'QODER_CN_DESKTOP_UNSUPPORTED':
      return i18n.t('qoder.diagnostics.cnDesktopUnsupported');
    case 'QODER_CLIENT_INVALID':
      return i18n.t('qoder.diagnostics.clientInvalid');
    case 'QODER_CLIENT_RUNNING':
      return i18n.t('qoder.diagnostics.clientRunning', {
        client: detail === 'desktop' ? 'Qoder CN' : 'Qoder CN CLI',
      });
    case 'QODER_NATIVE_TOKEN_MISSING':
      return i18n.t('qoder.diagnostics.nativeTokenMissing');
    case 'QODER_NATIVE_READBACK_FAILED':
    case 'QODER_NATIVE_ROUNDTRIP_FAILED':
    case 'QODER_NATIVE_DECRYPT_FAILED':
    case 'QODER_NATIVE_ENCRYPT_FAILED':
    case 'QODER_NATIVE_DPAPI_FAILED':
      return i18n.t('qoder.diagnostics.nativeWriteVerifyFailed');
    case 'QODER_NATIVE_KEY_MISSING':
    case 'QODER_NATIVE_KEY_INVALID':
    case 'QODER_NATIVE_KEY_UNSUPPORTED':
      return i18n.t('qoder.diagnostics.nativeKeyUnreadable');
    case 'QODER_IMPORT_IDENTITY_MISSING':
    case 'QODER_IMPORT_INVALID':
    case 'QODER_NATIVE_IDENTITY_MISSING':
      return i18n.t('qoder.diagnostics.nativeImportInvalid');
    case 'QODER_NATIVE_IMPORT_FAILED':
      return i18n.t('qoder.diagnostics.nativeImportFailed');
    case 'QODER_NATIVE_PROCESS_PROBE_FAILED':
      return i18n.t('qoder.diagnostics.nativeProcessProbeFailed');
    case 'QODER_REGION_UNSUPPORTED':
    case 'QODER_REGION_INVALID':
      return i18n.t('qoder.diagnostics.regionUnsupported');
    default:
      return message;
  }
}
