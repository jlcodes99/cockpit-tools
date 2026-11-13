import { useEffect, useRef, useState, type PointerEvent } from 'react';
import {
  hydrateUserMemory, mergeIdLists, persistUserMemoryList, readUserMemoryList,
  subscribeUserMemory, USER_MEMORY_LISTS,
} from '../utils/userMemory';

const orderId = USER_MEMORY_LISTS.codexTagGroupOrder;

function moveCodexTagGroup(order: string[], from: string, to: string): string[] {
  const source = order.indexOf(from);
  const destination = order.indexOf(to);
  if (source < 0 || destination < 0 || source === destination) return order;
  const next = [...order];
  next.splice(source, 1);
  next.splice(destination, 0, from);
  return next;
}

export function useCodexTagGroupLayout<T extends { groupKey: string }>(
  groups: readonly T[],
  availableTags: string[],
) {
  const [order, setOrder] = useState(() => readUserMemoryList(orderId));
  const [fold, setFold] = useState({ all: false, exceptions: new Set<string>() });
  const [dropTarget, setDropTarget] = useState<string | null>(null);
  const dragging = useRef<{ key: string; pointerId: number } | null>(null);

  useEffect(() => {
    const unsubscribe = subscribeUserMemory(() => setOrder(readUserMemoryList(orderId)));
    void hydrateUserMemory().catch((error) => console.warn('[Codex tag groups]', error));
    return unsubscribe;
  }, []);

  const allKeys = mergeIdLists(order, [
    '__untagged__', ...availableTags, ...groups.map((group) => group.groupKey),
  ]);
  const byKey = new Map(groups.map((group) => [group.groupKey, group]));
  const visible = allKeys.flatMap((key) => {
    const group = byKey.get(key);
    return group ? [group] : [];
  });

  const isCollapsed = (key: string) => fold.all !== fold.exceptions.has(key);
  const toggle = (key: string) => setFold((current) => {
    const exceptions = new Set(current.exceptions);
    if (exceptions.has(key)) exceptions.delete(key);
    else exceptions.add(key);
    return { all: current.all, exceptions };
  });
  const toggleAll = () => setFold((current) => ({ all: !current.all, exceptions: new Set() }));

  const move = (from: string, to: string) => {
    const next = moveCodexTagGroup(allKeys, from, to);
    if (next === allKeys) return;
    setOrder(next);
    persistUserMemoryList(orderId, next);
  };

  const endDrag = (event: PointerEvent) => {
    if (dragging.current?.pointerId !== event.pointerId) return;
    dragging.current = null;
    setDropTarget(null);
  };
  const startDrag = (event: PointerEvent, key: string) => {
    if (event.button !== 0 || dragging.current) return;
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    dragging.current = { key, pointerId: event.pointerId };
  };
  const groupAtPointer = (event: PointerEvent) => {
    const key = document.elementFromPoint(event.clientX, event.clientY)
      ?.closest('[data-codex-tag-group]')?.getAttribute('data-codex-tag-group');
    return key && key !== dragging.current?.key && byKey.has(key) ? key : null;
  };
  const dragOver = (event: PointerEvent) => {
    if (dragging.current?.pointerId === event.pointerId) setDropTarget(groupAtPointer(event));
  };
  const drop = (event: PointerEvent) => {
    const source = dragging.current;
    if (source?.pointerId !== event.pointerId) return;
    const target = groupAtPointer(event);
    if (target) move(source.key, target);
    endDrag(event);
  };

  return {
    visible, allCollapsed: fold.all, isCollapsed, toggle, toggleAll, move,
    dropTarget, startDrag, dragOver, drop, endDrag,
  };
}
