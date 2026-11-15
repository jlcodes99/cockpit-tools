import {
  DndContext, DragOverlay, KeyboardSensor, PointerSensor, closestCenter, pointerWithin,
  defaultDropAnimationSideEffects, useSensor, useSensors,
  type CollisionDetection, type Modifier,
} from '@dnd-kit/core';
import { SortableContext, sortableKeyboardCoordinates, useSortable, verticalListSortingStrategy } from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import { useTranslation } from 'react-i18next';
import { createPortal } from 'react-dom';
import { useEffect, useLayoutEffect, useRef, useState, type HTMLAttributes, type ReactNode } from 'react';
import { isReducedMotionEnabled } from '../utils/reducedMotion';
import './SortableLayoutList.css';

const verticalOnly: Modifier = ({ transform }) => ({ ...transform, x: 0 });
const collisionDetection: CollisionDetection = (args) => (
  args.pointerCoordinates ? pointerWithin(args) : closestCenter(args)
);

// A visual snapshot keeps expanded groups and custom icons identical to the
// source without mounting a second set of sortable controls or React effects.
function DragPreview({ source }: { source: HTMLElement }) {
  const host = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const clone = source.cloneNode(true) as HTMLElement;
    clone.classList.remove('is-sorting-source');
    Object.assign(clone.style, { transform: 'none', transition: 'none', opacity: '1' });
    for (const node of [clone, ...clone.querySelectorAll<HTMLElement>('*')]) {
      node.removeAttribute('id');
      node.removeAttribute('data-sortable-id');
    }
    host.current?.replaceChildren(clone);
    return () => host.current?.replaceChildren();
  }, [source]);
  return <div ref={host} className="sortable-layout-preview" aria-hidden="true" inert />;
}

interface ListProps {
  items: string[];
  className?: string;
  children: ReactNode;
  onReorder: (activeId: string, overId: string) => void;
  onSortingChange: (sorting: boolean) => void;
}

export function SortableLayoutList({ items, className = '', children, onReorder, onSortingChange }: ListProps) {
  const { t } = useTranslation();
  const list = useRef<HTMLDivElement>(null);
  const [source, setSource] = useState<HTMLElement | null>(null);
  const sorting = useRef(false);
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 5 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  );
  useEffect(() => {
    if (!source) return;
    document.documentElement.classList.add('platform-layout-sorting');
    return () => document.documentElement.classList.remove('platform-layout-sorting');
  }, [source]);
  useEffect(() => () => onSortingChange(false), [onSortingChange]);
  useEffect(() => {
    const consumeEscape = (event: KeyboardEvent) => {
      // Let the sensor cancel, while preventing the same key from closing the
      // dialog after React synchronously installs its close handler again.
      if (sorting.current && event.key === 'Escape') event.preventDefault();
    };
    document.addEventListener('keydown', consumeEscape, true);
    return () => document.removeEventListener('keydown', consumeEscape, true);
  }, []);
  const announcePosition = (id: string | number) => {
    const index = items.indexOf(String(id));
    return index < 0 ? undefined : t('platformLayout.sortPosition', { position: index + 1, total: items.length });
  };
  const finish = () => { sorting.current = false; setSource(null); onSortingChange(false); };

  return (
    <DndContext
      sensors={sensors}
      accessibility={{
        screenReaderInstructions: { draggable: t('platformLayout.sortKeyboardInstructions') },
        announcements: {
          onDragStart: ({ active }) => announcePosition(active.id),
          onDragOver: ({ over }) => over ? announcePosition(over.id) : undefined,
          onDragEnd: ({ active, over }) => {
            const index = over ? items.indexOf(String(over.id)) : -1;
            if (index < 0) return t('platformLayout.sortCancelled');
            return active.id === over?.id
              ? announcePosition(active.id)
              : t('platformLayout.sortPlaced', { position: index + 1 });
          },
          onDragCancel: () => t('platformLayout.sortCancelled'),
        },
      }}
      collisionDetection={collisionDetection}
      modifiers={[verticalOnly]}
      autoScroll={{ threshold: { x: 0.1, y: 0.18 }, acceleration: 8 }}
      onDragStart={({ active }) => {
        sorting.current = true;
        const node = Array.from(list.current?.children ?? []).find(
          child => (child as HTMLElement).dataset.sortableId === String(active.id),
        ) as HTMLElement | undefined;
        setSource(node ?? null);
        onSortingChange(true);
      }}
      onDragCancel={finish}
      onDragEnd={({ active, over }) => {
        finish();
        if (!over || active.id === over.id) return;
        const activeId = String(active.id);
        const overId = String(over.id);
        if (items.includes(activeId) && items.includes(overId)) onReorder(activeId, overId);
      }}
    >
      <SortableContext items={items} strategy={verticalListSortingStrategy}>
        <div ref={list} className={`sortable-layout-list ${className}${source ? ' is-sorting' : ''}`}>
          {children}
        </div>
      </SortableContext>
      {createPortal(
        <DragOverlay
          zIndex={2147483647}
          adjustScale={false}
          dropAnimation={isReducedMotionEnabled() ? null : {
            duration: 180, easing: 'ease-out',
            sideEffects: defaultDropAnimationSideEffects({ styles: { active: { opacity: '0' } } }),
          }}
        >
          {source ? <DragPreview source={source} /> : null}
        </DragOverlay>, document.body,
      )}
    </DndContext>
  );
}

interface HandleProps extends HTMLAttributes<HTMLButtonElement> {
  ref: (element: HTMLElement | null) => void;
}

export function SortableLayoutItem({ id, className, children }: {
  id: string;
  className: string;
  children: (handle: HandleProps) => ReactNode;
}) {
  const { attributes, listeners, setNodeRef, setActivatorNodeRef, transform, transition, isDragging } = useSortable({
    id, transition: { duration: isReducedMotionEnabled() ? 0 : 180, easing: 'ease-out' },
  });
  return (
    <div
      ref={setNodeRef}
      data-sortable-id={id}
      className={`${className}${isDragging ? ' is-sorting-source' : ''}`}
      style={{ transform: CSS.Transform.toString(transform), transition }}
    >
      {children({ ...attributes, ...listeners, ref: setActivatorNodeRef })}
    </div>
  );
}
