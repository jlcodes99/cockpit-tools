import type { CodexLocalAccessState } from "../types/codexLocalAccess";

// Match the backend diagnostic only; user-facing guidance uses translation keys.
export function isCodexGatewayKeyRecoveryError(error: string | null | undefined): boolean {
  return error?.replace(/^Error:\s*/, "") ===
    "网关无法解析入站 API Key，已保留原配置；请恢复被引用的账号后重试";
}

export function clearRecoveredCodexGatewayError(
  error: string,
  state: Pick<CodexLocalAccessState, "running" | "recovering" | "lastError">,
): string {
  return isCodexGatewayKeyRecoveryError(error)
    && (state.recovering || (state.running && !state.lastError)) ? "" : error;
}
