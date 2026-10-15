import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { CodexAccount } from '../types/codex';

/**
 * 官方客户端临时登录：打开官方 Codex 客户端完成一次真实登录，
 * 关闭临时客户端后读取最终登录信息；导入成功后清理，失败保留供重试。
 */
export type CodexTempLoginPhase =
  | 'preparing'
  | 'launching'
  | 'waiting-login'
  | 'importing'
  | 'closing'
  | 'cleaning'
  | 'completed'
  | 'failed'
  | 'cancelled';

export interface CodexTempLoginProgress {
  sessionId: string;
  phase: CodexTempLoginPhase;
  progress: number;
  accountId?: string | null;
  email?: string | null;
  /** completed 阶段回传的账号快照（与账号列表返回的结构一致）。 */
  account?: CodexAccount | null;
  /** 失败原因；completed / cancelled 时为清理告警（后台巡检重试）。 */
  error?: string | null;
}

export interface CodexTempLoginSession {
  sessionId: string;
  profileDir: string;
}

export function retryCodexTempLoginImport(sessionId: string): Promise<CodexTempLoginSession> {
  return invoke('retry_codex_temp_login_import', { sessionId });
}

export interface CodexTempLoginCleanupFailure {
  path: string;
  error: string;
}

export interface CodexTempLoginCleanupReport {
  removed: string[];
  failed: CodexTempLoginCleanupFailure[];
}

export const CODEX_TEMP_LOGIN_PROGRESS_EVENT = 'codex:temp-login-progress';

/** 本次登录流程中需要展示的步骤顺序。 */
export const CODEX_TEMP_LOGIN_STEPS: CodexTempLoginPhase[] = [
  'preparing',
  'launching',
  'waiting-login',
  'closing',
  'importing',
  'cleaning',
];

/** 打开官方客户端，由官方正常打开浏览器并完成授权。 */
export async function startCodexTempLogin(): Promise<CodexTempLoginSession> {
  return await invoke<CodexTempLoginSession>('start_codex_temp_login');
}

export async function cancelCodexTempLogin(sessionId: string): Promise<void> {
  await invoke('cancel_codex_temp_login', { sessionId });
}

/** 手动触发一次残留清理（正常由启动巡检与定期巡检自动完成）。 */
export async function cleanupCodexTempLoginArtifacts(): Promise<CodexTempLoginCleanupReport> {
  return await invoke<CodexTempLoginCleanupReport>('cleanup_codex_temp_login_artifacts');
}

export async function listenCodexTempLoginProgress(
  handler: (payload: CodexTempLoginProgress) => void,
): Promise<UnlistenFn> {
  return await listen<CodexTempLoginProgress>(CODEX_TEMP_LOGIN_PROGRESS_EVENT, (event) => {
    handler(event.payload);
  });
}
