import type {
  CodexLocalAccessCollection,
  CodexLocalAccessState,
} from "../types/codexLocalAccess";

export type CodexLocalAccessRuntimeStatus =
  | "disabled"
  | "stopped"
  | "running"
  | "internal";

export type CodexLocalAccessStatusBadgeTone =
  | "disabled"
  | "stopped"
  | "running"
  | "warning";

/** Keep every API-service surface on the same interpretation of enabled vs. process state. */
export function resolveCodexLocalAccessRuntimeStatus(
  collection: Pick<CodexLocalAccessCollection, "enabled"> | null | undefined,
  state:
    | Pick<CodexLocalAccessState, "running" | "internalRequired">
    | null
    | undefined,
): CodexLocalAccessRuntimeStatus {
  if (!collection) return "disabled";
  if (!collection.enabled) {
    return state?.running && state.internalRequired ? "internal" : "disabled";
  }
  return state?.running ? "running" : "stopped";
}

export function resolveCodexLocalAccessStatusBadgeTone(
  status: CodexLocalAccessRuntimeStatus,
): CodexLocalAccessStatusBadgeTone {
  return status === "internal" ? "warning" : status;
}
