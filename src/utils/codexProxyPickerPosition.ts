/** Prefer below when content fits; upward menus remain anchored by their bottom edge. */
export function proxyPickerPosition(top: number, bottom: number, viewport: number, contentHeight: number) {
  const below = Math.max(0, viewport - bottom - 12);
  const above = Math.max(0, top - 12);
  const desired = Math.min(400, contentHeight);
  const upward = below < desired && above > below;
  return {
    top: upward ? undefined : bottom + 6,
    bottom: upward ? viewport - top + 6 : undefined,
    maxHeight: Math.min(400, upward ? above : below),
  };
}
