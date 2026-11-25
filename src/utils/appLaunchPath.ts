export const APP_PATH_LABELS = {
  codex: 'Codex',
  claude: 'Claude Desktop',
  vscode: 'VS Code',
  windsurf: 'Devin',
  kiro: 'Kiro',
  cursor: 'Cursor',
  codebuddy: 'CodeBuddy',
  codebuddy_cn: 'CodeBuddy CN',
  qoder: 'Qoder',
  trae: 'Trae',
  trae_solo: 'Trae Solo',
  trae_cn: 'Trae CN',
  trae_solo_cn: 'Trae Solo CN',
  workbuddy: 'WorkBuddy',
  workbuddy_ai: 'WorkBuddy AI',
  minimaxcode: 'MiniMax Code',
  zcode: 'ZCode',
  t3code: 'T3 Code',
  zed: 'Zed',
  opencode: 'OpenCode',
  antigravity: 'Antigravity',
} as const;

export type LaunchPathApp = keyof typeof APP_PATH_LABELS;

export function isLaunchPathApp(app: string): app is LaunchPathApp {
  return Object.prototype.hasOwnProperty.call(APP_PATH_LABELS, app);
}

export function readInvalidWorkbuddyPath(error: unknown): string | undefined {
  return String(error).match(/^(?:Error: )?WORKBUDDY_NODE_APP_PATH:(.+)$/s)?.[1];
}

export function readMissingLaunchPathApp(error: unknown): LaunchPathApp | undefined {
  const code = String(error).replace(/^Error: /, '');
  const app = code.match(/^APP_PATH_(?:NOT_FOUND|MISSING_AFTER_SWITCH):([a-z_]+)$/)?.[1];
  return app && isLaunchPathApp(app) ? app : undefined;
}

export const INSTANCE_START_COMMANDS: Partial<Record<LaunchPathApp, string>> = {
  claude: 'claude_start_instance',
  vscode: 'github_copilot_start_instance',
  windsurf: 'windsurf_start_instance',
  kiro: 'kiro_start_instance',
  cursor: 'cursor_start_instance',
  codebuddy: 'codebuddy_start_instance',
  codebuddy_cn: 'codebuddy_cn_start_instance',
  qoder: 'qoder_start_instance',
  workbuddy: 'workbuddy_start_instance',
  minimaxcode: 'minimax_code_start_instance',
  zcode: 'zcode_start_instance',
  t3code: 't3code_start_instance',
};

export const LAUNCH_ONLY_COMMANDS: Partial<Record<LaunchPathApp, string>> = {
  workbuddy_ai: 'workbuddy_ai_launch_current',
  zcode: 'zcode_launch_current',
  t3code: 't3code_launch_current',
  cursor: 'cursor_launch_current',
  codebuddy: 'codebuddy_launch_current',
  codebuddy_cn: 'codebuddy_cn_launch_current',
  zed: 'zed_start_default_session',
  opencode: 'launch_opencode_current',
};
