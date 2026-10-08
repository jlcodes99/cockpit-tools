import { useEffect, useRef, useState } from 'react';

/** Match the top-layout handle: hold the left mouse button and hover over a row.
 * Preview locally while dragging; commit once on release, leaving the list, or blur.
 */
export function useProxyResourceOrder<T extends { id: string }>(
  sources: T[], disabled: boolean, onCommit: (ids: string[]) => void,
) {
  const [draft, setDraft] = useState<string[] | null>(null);
  const [draggingId, setDraggingId] = useState<string | null>(null);
  const session = useRef<{ id: string; original: string[]; order: string[] } | null>(null);
  const latest = useRef({ sources, disabled, onCommit });
  latest.current = { sources, disabled, onCommit };
  const same = (left: string[], right: string[]) => left.length === right.length && left.every((id, index) => id === right[index]);
  const clear = () => { session.current = null; setDraft(null); setDraggingId(null); };
  const finish = () => {
    const current = session.current;
    if (!current) return;
    clear();
    const state = latest.current;
    if (state.disabled || !same(current.original, state.sources.map((source) => source.id))) return;
    if (!same(current.original, current.order)) state.onCommit(current.order);
  };
  useEffect(() => {
    const key = (event: KeyboardEvent) => { if (event.key === 'Escape') clear(); };
    window.addEventListener('mouseup', finish);
    window.addEventListener('blur', finish);
    window.addEventListener('keydown', key);
    return () => {
      session.current = null;
      window.removeEventListener('mouseup', finish);
      window.removeEventListener('blur', finish);
      window.removeEventListener('keydown', key);
    };
    // Handlers read current inputs through latest; listener identity stays stable.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const sourceIds = JSON.stringify(sources.map((source) => source.id));
  useEffect(() => {
    if (session.current && (disabled || !same(session.current.original, sources.map((source) => source.id)))) clear();
    // Only changes to order/membership invalidate a drag, not fresh node metadata.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [disabled, sourceIds]);
  const move = (ids: string[], from: number, to: number) => {
    if (from < 0 || to < 0 || from >= ids.length || to >= ids.length || from === to) return ids;
    const next = [...ids]; next.splice(to, 0, next.splice(from, 1)[0]); return next;
  };
  const begin = (id: string, button: number) => {
    const state = latest.current;
    if (button !== 0 || state.disabled || state.sources.length < 2 || !state.sources.some((source) => source.id === id)) return;
    const ids = state.sources.map((source) => source.id);
    session.current = { id, original: ids, order: ids };
    setDraggingId(id);
  };
  const hover = (id: string, buttons: number) => {
    const current = session.current;
    if (!current) return;
    if (!(buttons & 1)) { finish(); return; }
    if (latest.current.disabled) { clear(); return; }
    const next = move(current.order, current.order.indexOf(current.id), current.order.indexOf(id));
    current.order = next; setDraft(next);
  };
  const moveBy = (id: string, offset: -1 | 1) => {
    if (latest.current.disabled || session.current) return;
    const ids = latest.current.sources.map((source) => source.id);
    const from = ids.indexOf(id), next = move(ids, from, from + offset);
    if (next !== ids) latest.current.onCommit(next);
  };
  const byId = new Map(sources.map((source) => [source.id, source]));
  // Render current source objects so a drag never restores stale node metadata.
  const orderedSources = draft ? draft.flatMap((id) => byId.has(id) ? [byId.get(id)!] : []) : sources;
  return { orderedSources, draggingId, begin, hover, finish, moveBy };
}
