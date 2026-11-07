import { useEffect, useRef, useState } from 'react';

/** Refresh persisted values without overwriting a field the user is editing. */
export function useRefreshSafeDraft<T extends string | number | boolean>(savedValue: T) {
  const [draft, setDraft] = useState(savedValue);
  const previous = useRef(savedValue);
  useEffect(() => {
    const before = previous.current;
    previous.current = savedValue;
    setDraft((current) => current === before ? savedValue : current);
  }, [savedValue]);
  return [draft, setDraft] as const;
}

/** Keys are independent: hydrate new entries and remove deleted ones, preserving edited labels. */
export function useRefreshSafeRecordDraft(savedValues: Record<string, string>) {
  const [drafts, setDrafts] = useState(savedValues);
  const previous = useRef(savedValues);
  useEffect(() => {
    const before = previous.current;
    previous.current = savedValues;
    setDrafts((current) => {
      const next = Object.fromEntries(Object.entries(savedValues).map(([id, value]) => [
        id, current[id] !== undefined && current[id] !== before[id] ? current[id] : value,
      ]));
      return Object.keys(next).length === Object.keys(current).length &&
        Object.keys(next).every((id) => next[id] === current[id]) ? current : next;
    });
  }, [savedValues]);
  return [drafts, setDrafts] as const;
}
