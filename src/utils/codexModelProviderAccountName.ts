export function resolveCodexModelProviderAccountName(
  providerName: string,
  apiKeyName?: string | null,
): string {
  return apiKeyName?.trim() || providerName.trim();
}

export function shouldSyncCodexModelProviderAccountName(
  accountName: string | null | undefined,
  providerName: string,
  previousApiKeyName?: string | null,
): boolean {
  const current = accountName?.trim() ?? "";
  if (!current) return true;

  const candidates = [providerName, previousApiKeyName]
    .map((value) => value?.trim())
    .filter((value): value is string => Boolean(value));
  return candidates.includes(current);
}
