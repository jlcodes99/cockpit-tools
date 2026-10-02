import type {
  CodexLocalAccessCollection,
  CodexLocalAccessState,
} from "../types/codexLocalAccess";

export const CODEX_LOCAL_ACCESS_STATUS_KEYS = {
  disabled: "codex.localAccess.statusDisabled",
  stopped: "codex.localAccess.statusStopped",
  running: "codex.localAccess.statusRunning",
} as const;

/** Every API-service surface follows the public entry's enabled state first. */
export function resolveCodexLocalAccessRuntimeStatus(
  collection: Pick<CodexLocalAccessCollection, "enabled"> | null | undefined,
  state: Pick<CodexLocalAccessState, "running"> | null | undefined,
): keyof typeof CODEX_LOCAL_ACCESS_STATUS_KEYS {
  if (!collection?.enabled) return "disabled";
  return state?.running ? "running" : "stopped";
}
