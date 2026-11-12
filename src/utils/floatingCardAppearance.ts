export const FLOATING_CARD_OPACITY_OPTIONS = [1, 0.85, 0.7, 0.55, 0.4, 0.25, 0] as const;
export const FLOATING_CARD_WIDTH = 250;

export function normalizeFloatingCardOpacity(value: unknown): number {
  return typeof value === 'number' && Number.isFinite(value) ? Math.min(1, Math.max(0, value)) : 1;
}

export function nextFloatingCardOpacity(value: number): number {
  const current = normalizeFloatingCardOpacity(value);
  return FLOATING_CARD_OPACITY_OPTIONS.find((option) => option < current - 0.001) ?? 1;
}

export function floatingCardTargetHeight(minimal: boolean, contentHeight: number, confirmOpen: boolean): number {
  const base = minimal ? (confirmOpen ? 240 : 110) : 290;
  return Math.max(base, Math.min(520, Math.ceil(Number.isFinite(contentHeight) ? contentHeight : base)));
}

export function measureFloatingCardContentHeight(shell: HTMLElement): number {
  const originalHeight = shell.style.getPropertyValue('height');
  const priority = shell.style.getPropertyPriority('height');
  // The visible shell fills the viewport. Measure its natural layout before
  // requesting a new window size so an oversized viewport can shrink again.
  shell.style.setProperty('height', 'auto');
  try {
    return shell.getBoundingClientRect().height;
  } finally {
    if (originalHeight) shell.style.setProperty('height', originalHeight, priority);
    else shell.style.removeProperty('height');
  }
}
