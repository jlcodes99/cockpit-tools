import md5 from 'blueimp-md5';

/** Cache identity only, not authentication. Works in desktop custom-scheme WebViews. */
export async function codexRemoteBinding(baseUrl: string, apiKey: string): Promise<string> {
  return codexRemoteBindingValue(baseUrl, apiKey);
}

export function codexRemoteBindingValue(baseUrl: string, apiKey: string): string {
  return md5(JSON.stringify([baseUrl.trim().replace(/\/+$/, ''), apiKey.trim()]));
}
