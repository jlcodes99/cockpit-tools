import { invoke } from "@tauri-apps/api/core";
import type {
  CodexSwitcherAccount,
  CodexSwitcherListResponse,
  CodexSwitcherQuota,
  CodexSwitcherRemoteAd,
  CodexSwitcherSettings,
} from "./types";

const desktopOnlyMessage = "当前是网页预览模式。本机账号导入、切换和额度刷新需要在桌面应用内运行。";

export function isDesktopRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

async function invokeDesktop<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isDesktopRuntime()) {
    throw new Error(desktopOnlyMessage);
  }
  return await invoke<T>(command, args);
}

function normalizeListResponse(raw: CodexSwitcherAccount[] | CodexSwitcherListResponse): CodexSwitcherListResponse {
  if (Array.isArray(raw)) {
    const current = raw.find((account) => account.is_current || account.current);
    return {
      accounts: raw,
      current_account_id: current?.id ?? null,
    };
  }

  return {
    accounts: raw.accounts ?? [],
    current_account_id: raw.current_account_id ?? null,
  };
}

export async function listAccounts(): Promise<CodexSwitcherListResponse> {
  const raw = await invokeDesktop<CodexSwitcherAccount[] | CodexSwitcherListResponse>(
    "codex_switcher_list_accounts",
  );
  return normalizeListResponse(raw);
}

export async function importAccessToken(accessToken: string): Promise<CodexSwitcherAccount> {
  return await invokeDesktop("codex_switcher_import_access_token", { accessToken });
}

// Safety boundary: the backend may reserve codex_switcher_redeem_activation_code,
// but this frontend intentionally does not expose a callable redeem flow.

export async function switchAccount(
  accountId: string,
  restartApp: boolean,
): Promise<CodexSwitcherAccount | void> {
  return await invokeDesktop("codex_switcher_switch_account", { accountId, restartApp });
}

export async function refreshAccountQuota(accountId: string): Promise<CodexSwitcherQuota> {
  return await invokeDesktop("codex_switcher_refresh_account_quota", { accountId });
}

export async function refreshAllQuotas(): Promise<void | number | CodexSwitcherAccount[]> {
  return await invokeDesktop("codex_switcher_refresh_all_quotas");
}

export async function deleteAccount(accountId: string): Promise<void> {
  return await invokeDesktop("codex_switcher_delete_account", { accountId });
}

export async function getSettings(): Promise<CodexSwitcherSettings> {
  return await invokeDesktop("codex_switcher_get_settings");
}

export async function updateSettings(settings: CodexSwitcherSettings): Promise<CodexSwitcherSettings> {
  return await invokeDesktop("codex_switcher_update_settings", { settings });
}

export async function fetchRemoteAd(): Promise<CodexSwitcherRemoteAd | null> {
  if (!isDesktopRuntime()) {
    return null;
  }
  return await invokeDesktop("codex_switcher_fetch_remote_ad");
}
