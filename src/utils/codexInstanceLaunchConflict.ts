const CODEX_INSTANCE_ACCOUNT_CONFLICT_PREFIX =
  "CODEX_INSTANCE_ACCOUNT_CONFLICT:";

export interface CodexInstanceRuntimeOwner {
  instanceId: string;
  instanceName: string;
  userDataDir: string;
  pid: number;
  isDefault: boolean;
  managed: boolean;
}

export interface CodexInstanceAccountConflict {
  targetInstanceId: string;
  targetInstanceName: string;
  accountId: string;
  accountEmail: string;
  owners: CodexInstanceRuntimeOwner[];
}

export function parseCodexInstanceAccountConflict(
  error: unknown,
): CodexInstanceAccountConflict | null {
  const raw = String(error ?? "");
  const markerIndex = raw.indexOf(CODEX_INSTANCE_ACCOUNT_CONFLICT_PREFIX);
  if (markerIndex < 0) return null;
  try {
    const parsed = JSON.parse(
      raw.slice(markerIndex + CODEX_INSTANCE_ACCOUNT_CONFLICT_PREFIX.length),
    ) as Partial<CodexInstanceAccountConflict>;
    if (
      !parsed.targetInstanceId ||
      !parsed.accountId ||
      !Array.isArray(parsed.owners) ||
      parsed.owners.length === 0
    ) {
      return null;
    }
    return parsed as CodexInstanceAccountConflict;
  } catch {
    return null;
  }
}

export function isCodexInstanceAccountConflict(error: unknown): boolean {
  return parseCodexInstanceAccountConflict(error) !== null;
}
