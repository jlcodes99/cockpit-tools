import { useEffect, useRef } from 'react';

/**
 * Enter-to-confirm for secondary confirmation dialogs.
 *
 * Uses a LIFO stack so only the topmost open confirm receives Enter when dialogs nest.
 * Ignores IME composition, modifier combos, textarea/select/contenteditable input.
 */

type EnterConfirmEntry = {
  id: number;
  run: () => void;
};

const enterConfirmStack: EnterConfirmEntry[] = [];
let nextEnterConfirmId = 1;
let globalListenerAttached = false;

function shouldIgnoreEnterTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;

  const tag = target.tagName;
  if (tag === 'TEXTAREA' || tag === 'SELECT') return true;

  // Multi-line rich text / custom editors often use role="textbox"
  if (target.getAttribute('role') === 'textbox' && target.getAttribute('aria-multiline') === 'true') {
    return true;
  }

  return false;
}

function handleGlobalEnterKeyDown(event: KeyboardEvent) {
  if (event.key !== 'Enter') return;
  if (event.defaultPrevented) return;
  if (event.shiftKey || event.ctrlKey || event.metaKey || event.altKey) return;
  if (event.isComposing || event.keyCode === 229) return;
  if (shouldIgnoreEnterTarget(event.target)) return;

  const top = enterConfirmStack[enterConfirmStack.length - 1];
  if (!top) return;

  event.preventDefault();
  event.stopImmediatePropagation();
  top.run();
}

function ensureGlobalListener() {
  if (globalListenerAttached) return;
  window.addEventListener('keydown', handleGlobalEnterKeyDown, true);
  globalListenerAttached = true;
}

function teardownGlobalListenerIfIdle() {
  if (enterConfirmStack.length > 0 || !globalListenerAttached) return;
  window.removeEventListener('keydown', handleGlobalEnterKeyDown, true);
  globalListenerAttached = false;
}

export type UseEnterConfirmOptions = {
  /** Extra gate; when false the confirm is not registered even if isOpen is true. */
  enabled?: boolean;
};

/**
 * When `isOpen` is true, pressing Enter triggers `onConfirm` (topmost dialog only).
 */
export function useEnterConfirm(
  isOpen: boolean,
  onConfirm: () => void,
  options?: UseEnterConfirmOptions,
) {
  const onConfirmRef = useRef(onConfirm);
  onConfirmRef.current = onConfirm;
  const enabled = options?.enabled !== false;

  useEffect(() => {
    if (!isOpen || !enabled) return;

    const entry: EnterConfirmEntry = {
      id: nextEnterConfirmId++,
      run: () => {
        onConfirmRef.current();
      },
    };
    enterConfirmStack.push(entry);
    ensureGlobalListener();

    return () => {
      const index = enterConfirmStack.findIndex((item) => item.id === entry.id);
      if (index >= 0) {
        enterConfirmStack.splice(index, 1);
      }
      teardownGlobalListenerIfIdle();
    };
  }, [isOpen, enabled]);
}
