/** This optional field is a management-page URL, never an API credential. */
export function isValidProviderApiKeyUrl(value: string): boolean {
  const trimmed = value.trim();
  if (!trimmed) return true;
  if (!/^https?:\/\//i.test(trimmed)) return false;
  try {
    const url = new URL(trimmed);
    return !!url.hostname && !url.username && !url.password;
  } catch {
    return false;
  }
}
