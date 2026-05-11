import { Page } from './navigation';

export type PlatformId =
  | 'antigravity'
  | 'codex'
  | 'zed'
  | 'github-copilot'
  | 'windsurf'
  | 'kiro'
  | 'cursor'
  | 'gemini'
  | 'devin-cli'
  | 'codebuddy'
  | 'codebuddy_cn'
  | 'qoder'
  | 'trae'
  | 'workbuddy';

export const ALL_PLATFORM_IDS: PlatformId[] = [
  'antigravity',
  'codex',
  'zed',
  'github-copilot',
  'windsurf',
  'kiro',
  'cursor',
  'gemini',
  'devin-cli',
  'codebuddy',
  'codebuddy_cn',
  'qoder',
  'trae',
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
  codex: 'codex',
  zed: 'zed',
  'github-copilot': 'github-copilot',
  windsurf: 'windsurf',
  kiro: 'kiro',
  cursor: 'cursor',
  gemini: 'gemini',
  'devin-cli': 'devin-cli',
  codebuddy: 'codebuddy',
  codebuddy_cn: 'codebuddy-cn',
  qoder: 'qoder',
  trae: 'trae',
  workbuddy: 'workbuddy',
};
