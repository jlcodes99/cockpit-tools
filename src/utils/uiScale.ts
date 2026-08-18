import { getCurrentWebview } from '@tauri-apps/api/webview';

/** 与设置页「界面缩放」可选档位保持一致 */
export const UI_SCALE_OPTION_VALUES = [0.3, 0.5, 0.7, 0.9, 1, 1.1, 1.25, 1.5] as const;

export const UI_SCALE_OPTION_STRINGS = UI_SCALE_OPTION_VALUES.map(String) as readonly string[];

export const UI_SCALE_MIN = 0.3;
export const UI_SCALE_MAX = 2.0;
export const UI_SCALE_DEFAULT = 1;

export function normalizeUiScale(raw?: number | null): number {
  if (typeof raw !== 'number' || !Number.isFinite(raw)) {
    return UI_SCALE_DEFAULT;
  }
  return Math.min(UI_SCALE_MAX, Math.max(UI_SCALE_MIN, raw));
}

/**
 * 按设置档位步进缩放。当前值落在档位之间时，放大取下一个档，缩小取上一个档。
 */
export function stepUiScale(current: number, direction: 1 | -1): number {
  const scale = normalizeUiScale(current);
  const options = UI_SCALE_OPTION_VALUES;

  if (direction > 0) {
    const next = options.find((value) => value > scale + 1e-9);
    return next ?? options[options.length - 1];
  }

  for (let index = options.length - 1; index >= 0; index -= 1) {
    if (options[index] < scale - 1e-9) {
      return options[index];
    }
  }
  return options[0];
}

export async function applyWebviewUiScale(rawScale?: number | null): Promise<number> {
  const normalized = normalizeUiScale(rawScale);
  await getCurrentWebview().setZoom(normalized);
  return normalized;
}

export function isUiScaleZoomInKey(event: KeyboardEvent): boolean {
  const key = event.key;
  const code = event.code;
  return (
    key === '+' ||
    key === '=' ||
    code === 'Equal' ||
    code === 'NumpadAdd'
  );
}

export function isUiScaleZoomOutKey(event: KeyboardEvent): boolean {
  const key = event.key;
  const code = event.code;
  return (
    key === '-' ||
    key === '_' ||
    code === 'Minus' ||
    code === 'NumpadSubtract'
  );
}

export function isUiScaleResetKey(event: KeyboardEvent): boolean {
  const key = event.key;
  const code = event.code;
  return key === '0' || code === 'Digit0' || code === 'Numpad0';
}
