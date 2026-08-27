export type Page =
  | 'dashboard'
  | 'manual'
  | 'api-relay'
  | 'overview'
  | 'codex'
  | 'claude'
  | 'claude-cli'
  | 'codex-api-service'
  | 'zed'
  | 'github-copilot'
  | 'windsurf'
  | 'kiro'
  | 'cursor'
  | 'grok'
  | 'kimi'
  | 'codebuddy'
  | 'codebuddy-cn'
  | 'qoder'
  | 'zcode'
  | 'trae'
  | 'trae-solo'
  | 'trae-cn'
  | 'trae-solo-cn'
  | 'workbuddy'
  | 'codex-instances'
  | 'instances'
  | 'accounts'
  | 'wakeup'
  | 'verification'
  | '2fa'
  | 'settings';

/** Pages that tray / floating-card restore may navigate to after main-window recreate. */
export const MAIN_WINDOW_NAVIGABLE_PAGES: readonly Page[] = [
  'dashboard',
  'manual',
  'api-relay',
  'overview',
  'codex',
  'claude',
  'claude-cli',
  'codex-api-service',
  'zed',
  'github-copilot',
  'windsurf',
  'kiro',
  'cursor',
  'grok',
  'kimi',
  'codebuddy',
  'codebuddy-cn',
  'qoder',
  'zcode',
  'trae',
  'trae-solo',
  'trae-cn',
  'trae-solo-cn',
  'workbuddy',
  'settings',
] as const;

export function isMainWindowNavigablePage(page: string): page is Page {
  return (MAIN_WINDOW_NAVIGABLE_PAGES as readonly string[]).includes(page);
}
