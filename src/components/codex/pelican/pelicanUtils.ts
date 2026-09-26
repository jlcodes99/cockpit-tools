import type { TFunction } from 'i18next';

export function pelicanError(cause: unknown, t: TFunction): string {
  const message = String(cause);
  const known: Record<string, string> = {
    PELICAN_TIMEOUT: 'pelican.errorTimeout',
    PELICAN_STREAM_INCOMPLETE: 'pelican.errorStream',
    PELICAN_RESPONSE_TOO_LARGE: 'pelican.errorLimit',
    PELICAN_PREVIEW_LIMIT: 'pelican.errorPreviewLimit',
    PELICAN_UNSUPPORTED_ACCOUNT: 'pelican.error.accountUnavailable',
    PELICAN_CANCELLED: 'pelican.cancelled',
  };
  for (const [code, key] of Object.entries(known)) if (message.includes(code)) return t(key);
  const localized = message.match(/^(pelican\.error\.[A-Za-z]+)(?::\s*([\s\S]*))?$/);
  if (localized) return `${t(localized[1])}${localized[2] ? `: ${localized[2]}` : ''}`;
  if (message === 'pelican.noHtml') return t(message);
  return message;
}
