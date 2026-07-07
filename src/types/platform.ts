import { Page } from './navigation';

export type PlatformId =
  | 'antigravity'
  | 'antigravity_ide'
  | 'codex'
  | 'claude_manager'
  | 'zed'
  | 'github-copilot'
  | 'windsurf'
  | 'kiro'
  | 'cursor'
  | 'gemini'
  | 'codebuddy'
  | 'codebuddy_cn'
  | 'qoder'
  | 'trae'
  | 'trae_solo'
  | 'trae_cn'
  | 'trae_solo_cn'
  | 'workbuddy';

export const ALL_PLATFORM_IDS: PlatformId[] = [
  'claude_manager',
  'codex',
  'antigravity',
  'antigravity_ide',
  'zed',
  'github-copilot',
  'windsurf',
  'kiro',
  'cursor',
  'gemini',
  'codebuddy',
  'codebuddy_cn',
  'qoder',
  'trae',
  'trae_solo',
  'trae_cn',
  'trae_solo_cn',
  'workbuddy',
];

export const MENU_HIDDEN_PLATFORM_IDS: PlatformId[] = [];

export const MENU_VISIBLE_PLATFORM_IDS: PlatformId[] = ALL_PLATFORM_IDS.filter(
  (platformId) => !MENU_HIDDEN_PLATFORM_IDS.includes(platformId),
);

export function isMenuVisiblePlatform(platformId: PlatformId): boolean {
  return !MENU_HIDDEN_PLATFORM_IDS.includes(platformId);
}

export const PLATFORM_PAGE_MAP: Record<PlatformId, Page> = {
  antigravity: 'overview',
  antigravity_ide: 'overview',
  codex: 'codex',
  claude_manager: 'claude',
  zed: 'zed',
  'github-copilot': 'github-copilot',
  windsurf: 'windsurf',
  kiro: 'kiro',
  cursor: 'cursor',
  gemini: 'gemini',
  codebuddy: 'codebuddy',
  codebuddy_cn: 'codebuddy-cn',
  qoder: 'qoder',
  trae: 'trae',
  trae_solo: 'trae-solo',
  trae_cn: 'trae-cn',
  trae_solo_cn: 'trae-solo-cn',
  workbuddy: 'workbuddy',
};
