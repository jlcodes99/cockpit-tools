export function resolveCodexHealthIssueDisplayName(
  accountName: string | null | undefined,
  accountEmail: string | null | undefined,
  healthEmail: string | null | undefined,
  accountId: string,
): string {
  const name = accountName?.trim();
  if (name) return name;

  const email = accountEmail?.trim() || healthEmail?.trim();
  if (email) return email;

  return accountId;
}
