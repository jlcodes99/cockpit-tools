export interface ModalStackEntry {
  node: () => HTMLElement | null;
  focus?: boolean;
  close?: () => void;
  /** Preserve the registration-order policy of legacy Escape callers without a ref. */
  withoutNode?: boolean;
}

type RegisteredModal = ModalStackEntry & { order: number };
const entries: RegisteredModal[] = [];
let registrationOrder = 0;

export function registerModal(entry: ModalStackEntry): RegisteredModal {
  const node = entry.node();
  const existing = node ? entries.find((other) => other.node() === node) : undefined;
  // Focus and Escape belong to the same opening. Re-enabling a busy dialog's
  // Escape handler must not move that existing dialog ahead of a newer sibling.
  const registered = { ...entry, order: existing?.order ?? ++registrationOrder };
  entries.push(registered);
  return registered;
}

export function unregisterModal(entry: RegisteredModal): void {
  const index = entries.indexOf(entry);
  if (index >= 0) entries.splice(index, 1);
}

function dialogLayer(view: Pick<Window, 'getComputedStyle'>, node: HTMLElement | null): number {
  let layer = 0;
  for (let current = node; current; current = current.parentElement) {
    const value = Number(view.getComputedStyle(current).zIndex);
    if (Number.isFinite(value)) layer = Math.max(layer, value);
  }
  return layer;
}

/** Both keyboard policies use the visible layer, then most recent registration. */
export function topModal(
  view: Pick<Window, 'getComputedStyle'>,
  { focusOnly = false, exclude, closingNode }: {
    focusOnly?: boolean;
    exclude?: HTMLElement | null;
    closingNode?: HTMLElement | null;
  } = {},
): RegisteredModal | undefined {
  let top: RegisteredModal | undefined;
  let topLayer = -Infinity;
  for (const entry of entries) {
    if (focusOnly && !entry.focus) continue;
    const node = entry.node();
    if (node === exclude || (!node && !entry.withoutNode)
      || (node && !node.isConnected && node !== closingNode)) continue;
    const layer = dialogLayer(view, node);
    if (layer > topLayer || (layer === topLayer && (!top || entry.order >= top.order))) {
      top = entry; topLayer = layer;
    }
  }
  return top;
}

export function modalCloseHandler(top: ModalStackEntry): (() => void) | undefined {
  const node = top.node();
  if (!node) return top.close;
  // A dialog has separate focus/ESC effects. Its focused entry may be registered
  // last; find its own close handler, never fall through to a background dialog.
  for (let index = entries.length - 1; index >= 0; index--) {
    const entry = entries[index];
    if (entry.close && entry.node() === node) return entry.close;
  }
  return undefined;
}
