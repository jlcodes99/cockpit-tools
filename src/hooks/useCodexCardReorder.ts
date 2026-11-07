import { useEffect, useLayoutEffect, useRef, type PointerEvent as ReactPointerEvent } from 'react';
import { reorderCodexCardIds } from '../utils/codexCardOrder';
import { isReducedMotionEnabled } from '../utils/reducedMotion';

const CARD_SELECTOR = '[data-codex-sort-id]';
const INTERACTIVE_SELECTOR = 'button, a, input, textarea, select, label, summary, [role="button"], [role="checkbox"], [role="switch"], [role="menuitem"], [contenteditable]:not([contenteditable="false"]), .account-email, [data-no-card-drag]';
const DRAG_THRESHOLD = 6;

interface CardSlot {
  id: string;
  element: HTMLElement;
  left: number;
  top: number;
  width: number;
  height: number;
}

interface DragSession {
  pointerId: number;
  sourceId: string;
  source: HTMLElement;
  grid: HTMLElement;
  startX: number;
  startY: number;
  allIds: string[];
  visibleIds: string[];
  active: boolean;
  slots: CardSlot[];
  gridOrigin: { left: number; top: number };
  previewId: string;
}

interface CardReorderOptions {
  orderedIds: readonly string[];
  enabled: boolean;
  viewKey: string;
  onReorder: (ids: string[]) => void;
}

interface CardMotionFrame {
  element: HTMLElement;
  rect: DOMRect;
  isSource: boolean;
}

/** Only blank space starts a drag; selecting text and using nested controls stay native. */
function isTextAtPoint(element: Element, x: number, y: number): boolean {
  const range = document.createRange();
  return Array.from(element.childNodes).some((node) => {
    if (node.nodeType !== Node.TEXT_NODE || !node.textContent?.trim()) return false;
    range.selectNodeContents(node);
    return Array.from(range.getClientRects()).some((rect) =>
      x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom,
    );
  });
}

/** Pointer-based sorting avoids Tauri's native file-drop handling. Persist only on drop. */
export function useCodexCardReorder(options: CardReorderOptions) {
  const latest = useRef(options);
  latest.current = options;
  const drag = useRef<DragSession | null>(null);
  const cleanup = useRef<() => void>(() => {});
  const suppressClick = useRef(false);
  const clickTimer = useRef<number | undefined>(undefined);
  const landingAnimations = useRef(new Map<HTMLElement, Animation>());
  const pendingLanding = useRef<{ orderKey: string; frames: CardMotionFrame[] } | null>(null);
  const orderKey = JSON.stringify(options.orderedIds);

  const motionDisabled = () => isReducedMotionEnabled() || window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  const stopLanding = () => {
    for (const [element, animation] of landingAnimations.current) {
      animation.onfinish = null;
      animation.oncancel = null;
      animation.cancel();
      element.removeAttribute('data-codex-settling');
    }
    landingAnimations.current.clear();
  };
  const playLanding = (frames: CardMotionFrame[]) => {
    stopLanding();
    if (motionDisabled()) return;
    for (const { element, rect: before, isSource } of frames) {
      if (!element.isConnected) continue;
      element.dataset.codexSettling = isSource ? 'source' : 'peer';
      const after = element.getBoundingClientRect();
      const x = before.left + before.width / 2 - after.left - after.width / 2;
      const y = before.top + before.height / 2 - after.top - after.height / 2;
      if (!after.width || !after.height || (Math.abs(x) < 0.5 && Math.abs(y) < 0.5
        && Math.abs(before.width - after.width) < 0.5 && Math.abs(before.height - after.height) < 0.5)) {
        element.removeAttribute('data-codex-settling');
        continue;
      }
      // Continue from the exact visible frame, including an unfinished preview
      // transition, after React has placed the cards in their saved positions.
      const animation = element.animate([
        { translate: `${x}px ${y}px`, scale: `${before.width / after.width} ${before.height / after.height}` },
        { translate: '0px 0px', scale: '1 1' },
      ], { duration: 260, easing: 'cubic-bezier(0.22, 1, 0.36, 1)', fill: 'both' });
      landingAnimations.current.set(element, animation);
      const release = () => {
        if (landingAnimations.current.get(element) !== animation) return;
        landingAnimations.current.delete(element);
        element.removeAttribute('data-codex-settling');
      };
      animation.oncancel = release;
      animation.onfinish = () => { release(); animation.cancel(); };
    }
  };

  useLayoutEffect(() => {
    const pending = pendingLanding.current;
    pendingLanding.current = null;
    if (pending?.orderKey === orderKey && options.enabled) playLanding(pending.frames);
    return stopLanding;
  }, [orderKey, options.enabled, options.viewKey]);

  useEffect(() => {
    let frame: number | null = null;
    let pointer: { x: number; y: number } | null = null;

    const findTarget = (session: DragSession, x: number, y: number) => {
      const hit = document.elementFromPoint(x, y);
      if (!hit || !session.grid.contains(hit)) return null;
      // Hit-test the original slots, not moving cards, so filling a vacancy cannot
      // immediately trigger another reorder or lose the destination under the cursor.
      const gridRect = session.grid.getBoundingClientRect();
      const localX = x - gridRect.left;
      const localY = y - gridRect.top;
      return session.slots.find((slot) => localX >= slot.left && localX <= slot.left + slot.width
        && localY >= slot.top && localY <= slot.top + slot.height) ?? null;
    };
    const preview = (session: DragSession, targetId: string) => {
      if (session.previewId === targetId) return;
      session.previewId = targetId;
      const order = reorderCodexCardIds(session.visibleIds, session.visibleIds, session.sourceId, targetId);
      const positions = new Map(order.map((id, index) => [id, index]));
      for (const slot of session.slots) {
        if (slot.id === session.sourceId) continue;
        const destination = session.slots[positions.get(slot.id)!];
        slot.element.dataset.codexShifting = 'true';
        slot.element.style.setProperty('--codex-card-shift-x', `${destination.left - slot.left}px`);
        slot.element.style.setProperty('--codex-card-shift-y', `${destination.top - slot.top}px`);
      }
    };
    const paint = () => {
      frame = null;
      const session = drag.current;
      if (!session?.active || !pointer) return;
      const target = findTarget(session, pointer.x, pointer.y);
      if (target) preview(session, target.id);
      const gridRect = session.grid.getBoundingClientRect();
      session.source.style.setProperty('--codex-card-drag-x', `${pointer.x - session.startX + session.gridOrigin.left - gridRect.left}px`);
      session.source.style.setProperty('--codex-card-drag-y', `${pointer.y - session.startY + session.gridOrigin.top - gridRect.top}px`);
    };
    const finish = () => {
      const session = drag.current;
      drag.current = null;
      if (frame !== null) window.cancelAnimationFrame(frame);
      frame = null;
      pointer = null;
      if (!session) return;
      for (const slot of session.slots) {
        slot.element.removeAttribute('data-codex-shifting');
        slot.element.style.removeProperty('--codex-card-shift-x');
        slot.element.style.removeProperty('--codex-card-shift-y');
      }
      session.source.removeAttribute('data-codex-dragging');
      session.source.style.removeProperty('--codex-card-drag-x');
      session.source.style.removeProperty('--codex-card-drag-y');
      // Clear preview offsets with transitions still disabled before React commits
      // the real order. Otherwise cards animate back from stale visual positions.
      if (session.active) session.grid.getBoundingClientRect();
      session.grid.removeAttribute('data-codex-sorting');
      if (session.source.hasPointerCapture(session.pointerId)) {
        session.source.releasePointerCapture(session.pointerId);
      }
      if (session.active) {
        suppressClick.current = true;
        window.clearTimeout(clickTimer.current);
        clickTimer.current = window.setTimeout(() => { suppressClick.current = false; }, 0);
      }
    };
    cleanup.current = finish;
    const captureMotion = (session: DragSession): CardMotionFrame[] => session.active && !motionDisabled()
      ? session.slots.map(({ element }) => ({ element, rect: element.getBoundingClientRect(), isSource: element === session.source }))
      : [];
    const cancelWithMotion = () => {
      const frames = drag.current ? captureMotion(drag.current) : [];
      finish();
      playLanding(frames);
    };
    const cancelImmediately = () => {
      finish();
      stopLanding();
      pendingLanding.current = null;
    };

    const move = (event: PointerEvent) => {
      const session = drag.current;
      if (!session || event.pointerId !== session.pointerId) return;
      if (event.buttons !== 1) { finish(); return; }
      if (!session.active) {
        if (Math.hypot(event.clientX - session.startX, event.clientY - session.startY) < DRAG_THRESHOLD) return;
        session.active = true;
        session.grid.dataset.codexSorting = 'true';
        const gridRect = session.grid.getBoundingClientRect();
        session.gridOrigin = { left: gridRect.left, top: gridRect.top };
        const elements = new Map(Array.from(session.grid.querySelectorAll<HTMLElement>(CARD_SELECTOR))
          .filter((card) => card.closest('.codex-accounts-grid') === session.grid)
          .map((card) => [card.dataset.codexSortId, card]));
        session.slots = session.visibleIds.flatMap((id) => {
          const element = elements.get(id);
          if (!element) return [];
          const rect = element.getBoundingClientRect();
          return [{ id, element, left: rect.left - gridRect.left, top: rect.top - gridRect.top,
            width: rect.width, height: rect.height }];
        });
        if (session.slots.length < 2 || !session.slots.some((slot) => slot.id === session.sourceId)) {
          finish(); return;
        }
        session.visibleIds = session.slots.map((slot) => slot.id);
        session.source.dataset.codexDragging = 'true';
        session.source.setPointerCapture(session.pointerId);
      }
      event.preventDefault();
      pointer = { x: event.clientX, y: event.clientY };
      if (frame === null) frame = window.requestAnimationFrame(paint);
    };
    const drop = (event: PointerEvent) => {
      const session = drag.current;
      if (!session || event.pointerId !== session.pointerId) return;
      const targetId = session.active
        ? findTarget(session, event.clientX, event.clientY)?.id : null;
      if (session.active) event.preventDefault();
      const frames = captureMotion(session);
      finish();
      if (targetId && targetId !== session.sourceId && latest.current.enabled
        && JSON.stringify(latest.current.orderedIds) === JSON.stringify(session.allIds)) {
        const nextOrder = reorderCodexCardIds(session.allIds, session.visibleIds, session.sourceId, targetId);
        pendingLanding.current = { orderKey: JSON.stringify(nextOrder), frames };
        latest.current.onReorder(nextOrder);
      } else playLanding(frames);
    };
    const cancelPointer = (event: PointerEvent) => {
      if (event.pointerId === drag.current?.pointerId) finish();
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && drag.current) { event.preventDefault(); cancelWithMotion(); }
    };
    const scroll = () => {
      if (drag.current?.active && pointer && frame === null) frame = window.requestAnimationFrame(paint);
    };
    window.addEventListener('pointermove', move, { passive: false });
    window.addEventListener('pointerup', drop);
    window.addEventListener('pointercancel', cancelPointer);
    window.addEventListener('lostpointercapture', cancelPointer);
    window.addEventListener('blur', cancelImmediately);
    window.addEventListener('resize', cancelImmediately);
    window.addEventListener('scroll', scroll, true);
    window.addEventListener('keydown', escape);
    return () => {
      finish();
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', drop);
      window.removeEventListener('pointercancel', cancelPointer);
      window.removeEventListener('lostpointercapture', cancelPointer);
      window.removeEventListener('blur', cancelImmediately);
      window.removeEventListener('resize', cancelImmediately);
      window.removeEventListener('scroll', scroll, true);
      window.removeEventListener('keydown', escape);
    };
  }, [options.enabled, options.viewKey, orderKey]);

  // Saving the new order rebuilds the drag effect before the browser dispatches click.
  useEffect(() => {
    const blockClick = (event: MouseEvent) => {
      if (suppressClick.current) { event.preventDefault(); event.stopPropagation(); }
    };
    window.addEventListener('click', blockClick, true);
    return () => {
      pendingLanding.current = null;
      window.clearTimeout(clickTimer.current);
      window.removeEventListener('click', blockClick, true);
    };
  }, []);

  const cardProps = (accountId: string, visibleIds: readonly string[]) => ({
    'data-codex-sort-id': options.enabled ? accountId : undefined,
    onPointerDown: (event: ReactPointerEvent<HTMLDivElement>) => {
      if (!latest.current.enabled || event.button !== 0 || !event.isPrimary || event.pointerType === 'touch') return;
      const target = event.target;
      if (!(target instanceof Element) || target.closest(INTERACTIVE_SELECTOR)
        || isTextAtPoint(target, event.clientX, event.clientY)
        || document.getSelection()?.isCollapsed === false) return;
      const grid = event.currentTarget.closest<HTMLElement>('.codex-accounts-grid');
      const allIds = [...latest.current.orderedIds];
      const scope = allIds.filter((id) => visibleIds.includes(id));
      if (!grid || scope.length < 2 || !scope.includes(accountId)) return;
      pendingLanding.current = null;
      stopLanding();
      cleanup.current();
      drag.current = {
        pointerId: event.pointerId, sourceId: accountId, source: event.currentTarget, grid,
        startX: event.clientX, startY: event.clientY, allIds, visibleIds: scope,
        active: false, slots: [], gridOrigin: { left: 0, top: 0 }, previewId: accountId,
      };
    },
  });
  return { cardProps };
}
