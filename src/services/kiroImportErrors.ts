type Translate = (key: string, options?: Record<string, unknown>) => string;

export function formatKiroImportError(message: string, t: Translate): string {
  const item = /^KIRO_IMPORT_ITEM:(\d+):(.*)$/s.exec(message);
  if (item) return t('common.shared.import.itemFailed', {
    index: Number(item[1]), error: formatKiroImportError(item[2], t),
  });
  if (message === 'KIRO_IMPORT_INVALID_JSON') return t('common.shared.import.invalidJson');
  if (message === 'KIRO_IMPORT_EMPTY') return t('common.shared.import.empty');
  if (message === 'KIRO_IMPORT_TOO_LARGE') return t('common.shared.import.sizeLimit', { limit: 8 });
  if (message === 'KIRO_IMPORT_TOO_MANY') return t('common.shared.import.countLimit', { count: 1000 });
  if (message === 'KIRO_REFRESH_BUSY') return t('common.dataQueryBusy');
  if (message === 'KIRO_USAGE_UNAVAILABLE') return t('common.shared.quota.noData');
  if (message === 'KIRO_USAGE_UNAUTHORIZED') return t('common.reauthorize');
  const http = /^KIRO_(?:USAGE_HTTP|EXTERNAL_IDP_REFRESH_HTTP):(\d+)$/.exec(message);
  if (http) return t('common.shared.refreshFailed', { error: `HTTP ${http[1]}` });
  if (message === 'KIRO_REFRESH_MISSING_ACCESS_TOKEN') return t('kiro.import.invalidCredentials');
  const transport = /^KIRO_EXTERNAL_IDP_REFRESH_(?:NETWORK|JSON):\s*(.*)$/s.exec(message);
  if (transport) return t('common.shared.refreshFailed', { error: transport[1] });
  if (/^KIRO_IMPORT_(INVALID_OBJECT|MISSING_CREDENTIALS|MISSING_CLIENT_ID|MISSING_TOKEN_ENDPOINT|INVALID_TOKEN_ENDPOINT)$/.test(message)) {
    return t('kiro.import.invalidCredentials');
  }
  return message;
}
