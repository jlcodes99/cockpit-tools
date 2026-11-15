import type { TFunction } from 'i18next';

export function formatAntigravitySwitchError(t: TFunction, error: unknown): string {
  const raw = String(error).replace(/^Error:\s*/, '').trim();
  if (raw === 'ANTIGRAVITY_SWITCH_BUSY') return t('messages.antigravitySwitchBusy');
  if (raw === 'ANTIGRAVITY_SWITCH_PREPARATION_TIMEOUT') return t('messages.antigravitySwitchPrepareTimeout');
  if (raw === 'ANTIGRAVITY_WINDOWS_CREDENTIAL_STORE_CAPACITY') return t('messages.antigravityWindowsCredentialStoreCapacity');
  if (raw === 'ANTIGRAVITY_WINDOWS_CREDENTIAL_TOO_LARGE') return t('messages.antigravityWindowsCredentialTooLarge');
  if (raw === 'ANTIGRAVITY_IDE_INITIALIZATION_REQUIRED') return t('antigravityLaunch.initializeHint');
  if (raw === 'ANTIGRAVITY_CLI_NOT_FOUND') return t('antigravityLaunch.cliMissing');
  const terminal = /^ANTIGRAVITY_CLI_(SWITCHED_)?TERMINAL_FAILED:\s*([\s\S]*)$/.exec(raw);
  if (terminal) return t(terminal[1] ? 'antigravityLaunch.switchedTerminalFailed' : 'antigravityLaunch.terminalFailed', { error: terminal[2] });
  const partial = /^ANTIGRAVITY_SWITCH_STATE_SAVE_FAILED:\s*([\s\S]*)$/.exec(raw);
  if (partial) return t('messages.antigravitySwitchStateSaveFailed', { error: partial[1] });
  return String(error);
}
