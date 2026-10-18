import type { CodexAccount } from "../types/codex";
import type { DeepSeekAccessMode } from "./codexDeepSeekAccess";

export type CodexProviderAccessAccount = Pick<
  CodexAccount,
  "auth_mode" | "api_wire_api" | "api_instance_access_mode" | "upstream_grok_account_id"
>;

/** Grok-linked accounts need the local gateway to supply their upstream credentials. */
export function isCodexProviderAccessAccount(
  account: CodexProviderAccessAccount,
): boolean {
  return (
    (account.auth_mode ?? "").trim().toLowerCase() === "apikey" &&
    !account.upstream_grok_account_id?.trim()
  );
}

export function canChooseCodexProviderAccessMode(
  account: CodexProviderAccessAccount,
): boolean {
  const wireApi = (account.api_wire_api ?? "").trim().toLowerCase();
  return isCodexProviderAccessAccount(account) && (!wireApi || wireApi === "responses");
}

export function resolveCodexProviderAccessMode(
  account?: CodexProviderAccessAccount | null,
): DeepSeekAccessMode {
  if (!account || !canChooseCodexProviderAccessMode(account)) return "gateway";
  const mode = (account.api_instance_access_mode ?? "").trim().toLowerCase();
  return mode === "direct" || mode === "cdp" ? mode : "gateway";
}
