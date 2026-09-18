export function isAinipyBaseUrl(value?: string | null): boolean {
  try {
    const url = new URL(value?.trim() || '');
    return url.protocol === 'https:' && !url.username && !url.password &&
      (!url.port || url.port === '443') &&
      ['ainipy.com', 'www.ainipy.com', 'api.ainipy.com'].includes(url.hostname);
  } catch {
    return false;
  }
}
