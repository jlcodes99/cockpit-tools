import type { TFunction } from 'i18next';

export function formatAntigravitySwitchError(t: TFunction, error: unknown): string {
  const raw = String(error).replace(/^Error:\s*/, '').trim();
  if (raw === 'ANTIGRAVITY_SWITCH_BUSY') return t('messages.antigravitySwitchBusy');
  if (raw === 'ANTIGRAVITY_SWITCH_PREPARATION_TIMEOUT') return t('messages.antigravitySwitchPrepareTimeout');
  const partial = /^ANTIGRAVITY_SWITCH_STATE_SAVE_FAILED:\s*([\s\S]*)$/.exec(raw);
  if (partial) return t('messages.antigravitySwitchStateSaveFailed', { error: partial[1] });
  return String(error);
}
