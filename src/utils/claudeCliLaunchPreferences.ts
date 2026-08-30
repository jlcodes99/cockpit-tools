const CLAUDE_CLI_LAST_WORKING_DIR_KEY = 'agtools.claude.cli.last_working_dir';

/**
 * 清理 Claude CLI 实例名称，确保名称可安全用于跨平台的实例目录与配置文件。
 */
export function sanitizeClaudeCliInstanceName(value: string): string {
  return value.replace(/[\\/:*?"<>|]/g, ' ').replace(/\s+/g, ' ').trim() || 'Claude CLI';
}

/** 读取上次成功选择的 Claude CLI 工作目录，供下次打开启动弹框时复用。 */
export function readLastClaudeCliWorkingDir(): string {
  try {
    return localStorage.getItem(CLAUDE_CLI_LAST_WORKING_DIR_KEY)?.trim() || '';
  } catch {
    return '';
  }
}

/** 持久化有效的 Claude CLI 工作目录；存储失败不影响本次启动流程。 */
export function persistLastClaudeCliWorkingDir(value: string): void {
  const trimmed = value.trim();
  if (!trimmed) return;
  try {
    localStorage.setItem(CLAUDE_CLI_LAST_WORKING_DIR_KEY, trimmed);
  } catch {
    // 工作目录只用于改善下次启动体验，浏览器存储不可用时允许静默降级。
  }
}
