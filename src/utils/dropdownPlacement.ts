export interface DropdownPlacement {
  top?: number;
  bottom?: number;
  left: number;
  width: number;
  maxHeight: number;
  zIndex: number;
}

/** Keep body-portaled menus above their owning dialog, without eclipsing later dialogs. */
export function dropdownLayer(ancestorLayers: string[], baseLayer = 11000): number {
  return ancestorLayers.reduce((layer, value) => {
    const parsed = Number(value);
    return Number.isFinite(parsed) ? Math.max(layer, parsed + 1) : layer;
  }, baseLayer);
}

export function dropdownPlacement(
  rect: { left: number; top: number; bottom: number; width: number },
  viewport: { width: number; height: number },
  preferred: 'down' | 'up',
  requestedWidth: number | undefined,
  requestedHeight: number,
  zIndex: number,
): DropdownPlacement {
  const margin = 12;
  const gap = 10;
  const above = Math.max(0, rect.top - gap - margin);
  const below = Math.max(0, viewport.height - rect.bottom - gap - margin);
  const preferredRoom = preferred === 'up' ? above : below;
  const otherRoom = preferred === 'up' ? below : above;
  const usePreferred = preferredRoom >= Math.min(requestedHeight, 160) || preferredRoom >= otherRoom;
  const up = usePreferred ? preferred === 'up' : preferred !== 'up';
  const width = Math.min(Math.max(rect.width, requestedWidth ?? 0), Math.max(1, viewport.width - margin * 2));
  return {
    ...(up ? { bottom: Math.max(margin, viewport.height - rect.top + gap) } : { top: Math.max(margin, rect.bottom + gap) }),
    left: Math.max(margin, Math.min(rect.left, viewport.width - width - margin)),
    width,
    maxHeight: Math.min(requestedHeight, up ? above : below),
    zIndex,
  };
}
