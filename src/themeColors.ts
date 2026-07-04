export type ThemeColorId =
  | 'neutral'
  | 'tokyonight'
  | 'everforest'
  | 'ayu'
  | 'catppuccin'
  | 'catppuccin-macchiato'
  | 'gruvbox'
  | 'kanagawa'
  | 'nord'
  | 'one-dark';

export type ThemeColorPreset = {
  id: ThemeColorId;
  lightHex: string;
  darkHex: string;
  accentHex: string;
  label: string;
};

export const DEFAULT_THEME_COLOR: ThemeColorId = 'neutral';

export const THEME_COLOR_PRESETS: ThemeColorPreset[] = [
  { id: 'neutral', lightHex: '#f6f5f2', darkHex: '#0f172a', accentHex: '#1d4ed8', label: 'Neutral' },
  { id: 'tokyonight', lightHex: '#d5d6db', darkHex: '#1a1b26', accentHex: '#7aa2f7', label: 'Tokyo Night' },
  { id: 'everforest', lightHex: '#fff9e8', darkHex: '#2d353b', accentHex: '#a7c080', label: 'Everforest' },
  { id: 'ayu', lightHex: '#fafafa', darkHex: '#0a0e14', accentHex: '#ffb454', label: 'Ayu' },
  { id: 'catppuccin', lightHex: '#eff1f5', darkHex: '#1e1e2e', accentHex: '#cba6f7', label: 'Catppuccin' },
  { id: 'catppuccin-macchiato', lightHex: '#eff1f5', darkHex: '#24273a', accentHex: '#c6a0f6', label: 'Macchiato' },
  { id: 'gruvbox', lightHex: '#fbf1c7', darkHex: '#282828', accentHex: '#fabd2f', label: 'Gruvbox' },
  { id: 'kanagawa', lightHex: '#f2ecbc', darkHex: '#1f1f28', accentHex: '#7e9cd8', label: 'Kanagawa' },
  { id: 'nord', lightHex: '#eceff4', darkHex: '#2e3440', accentHex: '#88c0d0', label: 'Nord' },
  { id: 'one-dark', lightHex: '#fafafa', darkHex: '#282c34', accentHex: '#61afef', label: 'One Dark' },
];

export function normalizeThemeColorId(value: string | null | undefined): ThemeColorId {
  return THEME_COLOR_PRESETS.some((preset) => preset.id === value)
    ? value as ThemeColorId
    : DEFAULT_THEME_COLOR;
}

export function getThemeColorPreset(value: string | null | undefined): ThemeColorPreset {
  const id = normalizeThemeColorId(value);
  return THEME_COLOR_PRESETS.find((preset) => preset.id === id) ?? THEME_COLOR_PRESETS[0];
}

export function resolveAppliedTheme(theme: string | null | undefined): 'light' | 'dark' {
  if (theme === 'system' && typeof window !== 'undefined') {
    return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
  }
  return theme === 'dark' ? 'dark' : 'light';
}

export function applyThemeToDocument(theme: string | null | undefined, themeColor?: string | null): void {
  if (typeof document === 'undefined') {
    return;
  }

  const appliedTheme = resolveAppliedTheme(theme);
  const appliedThemeColor = normalizeThemeColorId(themeColor);
  const root = document.documentElement;
  root.setAttribute('data-theme', appliedTheme);
  root.setAttribute('data-theme-color', appliedThemeColor);
  document.body?.setAttribute('data-theme', appliedTheme);
  document.body?.setAttribute('data-theme-color', appliedThemeColor);
}
