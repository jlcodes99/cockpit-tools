/**
 * Compatibility re-export for callers introduced during the OpenCode Go rollout.
 * `openCodeGoService.ts` is the sole Tauri command boundary.
 */
export {
  OPENCODE_GO_CONNECTION_LIMIT,
  normalizeOpenCodeGoConnection,
  normalizeOpenCodeGoQuotaSnapshot,
  toOpenCodeGoCommandError,
} from './openCodeGoService.ts';
export type {
  OpenCodeGoConnectionSummary,
  OpenCodeGoUsageErrorKind,
} from './openCodeGoService.ts';
