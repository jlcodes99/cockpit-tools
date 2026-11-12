const OFFICIAL_LS_UNAVAILABLE_PREFIX = 'WAKEUP_OFFICIAL_LS_UNAVAILABLE:';

export function formatWakeupRuntimeError(
  message: string,
  t: (key: string, options?: Record<string, unknown>) => string,
): string {
  if (message === 'WAKEUP_DIRECT_NO_REPLY') return t('wakeup.format.noReply');
  if (!message.startsWith(OFFICIAL_LS_UNAVAILABLE_PREFIX)) return message;
  return t('wakeup.errorUi.officialLanguageServerUnavailable', {
    path: message.slice(OFFICIAL_LS_UNAVAILABLE_PREFIX.length),
  });
}
