import type { CSSProperties } from 'react';

export const BACKGROUND_PRESETS = ['default', 'mist', 'forest', 'dusk', 'sand', 'custom'] as const;
export type BackgroundPreset = typeof BACKGROUND_PRESETS[number];
export type BackgroundPosition = 'center' | 'top' | 'bottom';
export interface AppBackgroundSettings {
  preset: BackgroundPreset;
  strength: number;
  blur: number;
  position: BackgroundPosition;
}
export const DEFAULT_APP_BACKGROUND: AppBackgroundSettings = {
  preset: 'default', strength: 55, blur: 0, position: 'center',
};
const GRADIENTS: Record<Exclude<BackgroundPreset, 'custom' | 'default'>, string> = {
  mist: 'radial-gradient(ellipse at 10% 5%, #86c5dc, transparent 65%), linear-gradient(130deg, #dfeaf5, #9bafe5 65%, #d0d9ed)',
  forest: 'radial-gradient(ellipse at 85% 15%, #b4d8ad, transparent 60%), linear-gradient(135deg, #9ecdbf, #3d857b 70%, #284d55)',
  dusk: 'radial-gradient(ellipse at 15% 10%, #edc2da, transparent 60%), linear-gradient(140deg, #bdb4e5, #8c88c4 60%, #4b688b)',
  sand: 'radial-gradient(ellipse at 85% 5%, #fae5b5, transparent 60%), linear-gradient(130deg, #ecdbcb, #d4ac8c 65%, #a98c87)',
};
const clamp = (value: unknown, fallback: number, max: number) =>
  typeof value === 'number' && Number.isFinite(value) ? Math.min(max, Math.max(0, value)) : fallback;

export function normalizeAppBackground(value: Partial<AppBackgroundSettings> = {}): AppBackgroundSettings {
  return {
    preset: BACKGROUND_PRESETS.includes(value.preset as BackgroundPreset) ? value.preset! : 'default',
    strength: clamp(value.strength, 55, 100),
    blur: clamp(value.blur, 0, 20),
    position: value.position === 'top' || value.position === 'bottom' ? value.position : 'center',
  };
}

export function appBackgroundStyle(settings: AppBackgroundSettings, imageUrl: string | null): CSSProperties {
  const backgroundImage = settings.preset === 'custom'
    ? imageUrl ? `url("${imageUrl}")` : 'none'
    : settings.preset === 'default' ? 'none' : GRADIENTS[settings.preset];
  return {
    backgroundImage,
    opacity: settings.strength / 100,
    filter: `blur(${settings.blur}px)`,
    backgroundPosition: `center ${settings.position}`,
  };
}
