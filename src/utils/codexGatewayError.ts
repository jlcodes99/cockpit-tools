import type { TFunction } from "i18next";

const gatewayErrorKeys: Record<string, string> = {
  NATIVE_API_BUSY: "codex.localAccess.gatewayEngineRequestsBusy",
  NATIVE_API_STOPPED: "codex.localAccess.gatewayEngineStopped",
};

export function formatCodexGatewayModeError(t: TFunction, error: unknown): string {
  const raw = String(error).replace(/^Error:\s*/, "").trim();
  const match = /^(NATIVE_API_[A-Z_]+)(?::\s*([\s\S]*))?$/.exec(raw);
  const key = match && gatewayErrorKeys[match[1]];
  return key ? `${t(key)}${match?.[2] ? `\n${match[2]}` : ""}` : raw;
}
