function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function isSub2ApiExport(value: Record<string, unknown>): boolean {
  const accounts = value.accounts;
  if (!Array.isArray(accounts)) return false;
  return (
    "exported_at" in value ||
    "proxies" in value ||
    accounts.some(
      (account) =>
        isRecord(account) &&
        "credentials" in account &&
        "platform" in account,
    )
  );
}

function splitParsedJson(value: unknown): unknown[] {
  if (Array.isArray(value)) return value;
  if (!isRecord(value) || !isSub2ApiExport(value)) return [value];

  const accounts = (value.accounts as unknown[]).filter((account) => {
    if (!isRecord(account)) return false;
    return (
      String(account.platform ?? "").toLowerCase() === "openai" &&
      String(account.type ?? "").toLowerCase() === "oauth"
    );
  });
  return accounts.length > 0 ? accounts : [value];
}

function readNonEmptyString(
  value: Record<string, unknown>,
  key: string,
): string | undefined {
  const raw = value[key];
  if (typeof raw !== "string") return undefined;
  const trimmed = raw.trim();
  return trimmed || undefined;
}

function normalizeWebSessionValue(
  value: unknown,
  depth = 0,
): Record<string, unknown> | null {
  if (!isRecord(value) || depth > 4) return null;
  for (const key of ["session_json", "session"]) {
    const nested = value[key];
    if (isRecord(nested)) {
      const normalized = normalizeWebSessionValue(nested, depth + 1);
      if (normalized) return normalized;
    } else if (typeof nested === "string") {
      try {
        const normalized = normalizeWebSessionValue(
          JSON.parse(nested) as unknown,
          depth + 1,
        );
        if (normalized) return normalized;
      } catch {
        // Keep checking the current object.
      }
    }
  }

  const accessToken =
    readNonEmptyString(value, "accessToken") ||
    readNonEmptyString(value, "access_token");
  if (!accessToken) return null;
  const authProvider = readNonEmptyString(value, "authProvider");
  const hasSessionMarker =
    isRecord(value.user) ||
    isRecord(value.account) ||
    "expires" in value ||
    "sessionToken" in value ||
    authProvider?.toLowerCase() === "openai";
  return hasSessionMarker ? value : null;
}

function resolveWebSessionLabel(
  session: Record<string, unknown>,
  index: number,
): string {
  const user = isRecord(session.user) ? session.user : null;
  const account = isRecord(session.account) ? session.account : null;
  return (
    (user && readNonEmptyString(user, "email")) ||
    (user && readNonEmptyString(user, "name")) ||
    (account && readNonEmptyString(account, "name")) ||
    (account && readNonEmptyString(account, "id")) ||
    (user && readNonEmptyString(user, "id")) ||
    `Web Session ${index + 1}`
  );
}

export interface CodexWebSessionImportInfo {
  label: string;
}

export function findCodexWebSessionImports(
  rawContent: string,
): CodexWebSessionImportInfo[] {
  const payloads = splitCodexImportPayloads(rawContent);
  const result: CodexWebSessionImportInfo[] = [];
  for (const payload of payloads) {
    try {
      const session = normalizeWebSessionValue(JSON.parse(payload) as unknown);
      if (!session) continue;
      result.push({ label: resolveWebSessionLabel(session, result.length) });
    } catch {
      // Non-JSON token inputs are not Web Sessions.
    }
  }
  return result;
}

export function splitCodexImportPayloads(rawContent: string): string[] {
  const trimmed = rawContent.trim();
  if (!trimmed) return [];

  try {
    return splitParsedJson(JSON.parse(trimmed)).map((item) =>
      JSON.stringify(item),
    );
  } catch {
    const lines = trimmed
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter(Boolean);
    if (lines.length <= 1) return [trimmed];

    const parsedLines: unknown[] = [];
    for (const line of lines) {
      try {
        const value = JSON.parse(line) as unknown;
        if (!isRecord(value)) return lines;
        parsedLines.push(value);
      } catch {
        return trimmed.startsWith("{") || trimmed.startsWith("[")
          ? [trimmed]
          : lines;
      }
    }
    return parsedLines.map((item) => JSON.stringify(item));
  }
}
