type Translate = (
  key: string,
  options?: Record<string, string | number>,
) => string;

const WINDOWS_TCP_EXCLUDED_PORT_ERROR_CODE =
  "COCKPIT_WINDOWS_TCP_EXCLUDED_PORT";
const WINDOWS_PORT_PERMISSION_DENIED_ERROR_CODE =
  "COCKPIT_WINDOWS_PORT_PERMISSION_DENIED";

function parseStructuredError(raw: string): {
  code: string;
  params: Record<string, string>;
} | null {
  const parts = raw.split("|");
  const code = parts.shift()?.trim();
  if (!code) return null;

  const params: Record<string, string> = {};
  for (const part of parts) {
    const separatorIndex = part.indexOf("=");
    if (separatorIndex <= 0) continue;
    const key = part.slice(0, separatorIndex).trim();
    const value = part.slice(separatorIndex + 1).trim();
    if (key) params[key] = value;
  }

  return { code, params };
}

export function rawErrorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  return String(error ?? "");
}

export function formatCodexLocalAccessErrorMessage(
  error: unknown,
  t: Translate,
): string {
  const raw = rawErrorMessage(error).trim();
  if (!raw) return raw;

  const parsed = parseStructuredError(raw);
  if (!parsed) return raw;

  if (parsed.code === WINDOWS_TCP_EXCLUDED_PORT_ERROR_CODE) {
    return t("codex.localAccess.errors.windowsTcpExcludedPort", {
      bindHost: parsed.params.bindHost || "-",
      port: parsed.params.port || "-",
      rangeStart: parsed.params.rangeStart || "-",
      rangeEnd: parsed.params.rangeEnd || "-",
      defaultValue:
        "API Service port {{port}} is reserved by Windows, so {{bindHost}}:{{port}} cannot be listened on. The port falls in the reserved TCP range {{rangeStart}}-{{rangeEnd}}. Use an unreserved port, for example 18080 or 28080.",
    });
  }

  if (parsed.code === WINDOWS_PORT_PERMISSION_DENIED_ERROR_CODE) {
    return t("codex.localAccess.errors.windowsPortPermissionDenied", {
      bindHost: parsed.params.bindHost || "-",
      port: parsed.params.port || "-",
      defaultValue:
        "API Service failed to listen on port {{port}} because Windows denied access to {{bindHost}}:{{port}}. Check Windows reserved ports, antivirus, firewall, VPN, Hyper-V, WSL, or Docker restrictions, or use another port.",
    });
  }

  return raw;
}
