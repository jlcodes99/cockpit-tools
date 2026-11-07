const OVERFLOW_PROPERTIES = ['overflow', 'overflow-x', 'overflow-y'] as const;

interface OverflowSnapshot {
  property: (typeof OVERFLOW_PROPERTIES)[number];
  value: string;
  priority: string;
}

interface ScrollLockState {
  count: number;
  overflow: OverflowSnapshot[];
}

const scrollLocks = new WeakMap<HTMLElement, ScrollLockState>();

function acquireElementScrollLock(element: HTMLElement): () => void {
  let state = scrollLocks.get(element);
  if (state) {
    state.count += 1;
  } else {
    state = {
      count: 1,
      overflow: OVERFLOW_PROPERTIES.map((property) => ({
        property,
        value: element.style.getPropertyValue(property),
        priority: element.style.getPropertyPriority(property),
      })),
    };
    scrollLocks.set(element, state);
    // Preserve the scroll offset and the modal's own scroll containers. Do not
    // use position: fixed or intercept wheel/touch events on the document.
    element.style.setProperty('overflow-x', 'hidden', 'important');
    element.style.setProperty('overflow-y', 'hidden', 'important');
  }

  const acquiredState = state;
  return () => {
    acquiredState.count -= 1;
    if (acquiredState.count !== 0) return;

    // Remove our longhands before restoring the original shorthand/longhands,
    // including mixed priorities. Leave unrelated inline edits untouched.
    for (const property of OVERFLOW_PROPERTIES) {
      element.style.removeProperty(property);
    }
    for (const { property, value, priority } of acquiredState.overflow) {
      if (value) element.style.setProperty(property, value, priority);
    }
    scrollLocks.delete(element);
  };
}

/** Lock background scrolling while allowing body-portaled modals to scroll. */
export function acquireModalScrollLock(doc: Document): () => void {
  const containers = new Set([
    doc.documentElement,
    doc.body,
    ...doc.querySelectorAll<HTMLElement>('.main-wrapper'),
  ]);
  const releases = [...containers]
    .filter((element): element is HTMLElement => Boolean(element?.style))
    .map(acquireElementScrollLock);
  let released = false;

  return () => {
    if (released) return;
    released = true;
    releases.forEach((release) => release());
  };
}
