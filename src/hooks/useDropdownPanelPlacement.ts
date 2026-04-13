import { useLayoutEffect, useRef, useState, type CSSProperties, type RefObject } from 'react';

const PANEL_GAP = 8;
const VIEWPORT_MARGIN = 12;
const PANEL_PADDING_BUFFER = 20;

export function useDropdownPanelPlacement(
  rootRef: RefObject<HTMLElement | null>,
  open: boolean,
  dependencyKey?: unknown,
) {
  const panelRef = useRef<HTMLDivElement | null>(null);
  const [panelPlacement, setPanelPlacement] = useState<'top' | 'bottom'>('bottom');
  const [panelMaxHeight, setPanelMaxHeight] = useState<number | null>(null);

  useLayoutEffect(() => {
    if (!open) {
      setPanelPlacement('bottom');
      setPanelMaxHeight(null);
      return;
    }

    const updatePanelPosition = () => {
      const rootElement = rootRef.current;
      const panelElement = panelRef.current;
      if (!rootElement || !panelElement) return;

      const rootRect = rootElement.getBoundingClientRect();
      const panelHeight = panelElement.offsetHeight;
      const availableBelow = window.innerHeight - rootRect.bottom - PANEL_GAP - VIEWPORT_MARGIN;
      const availableAbove = rootRect.top - PANEL_GAP - VIEWPORT_MARGIN;
      const nextPlacement =
        availableBelow < panelHeight && availableAbove > availableBelow ? 'top' : 'bottom';
      const availableSpace = nextPlacement === 'top' ? availableAbove : availableBelow;

      setPanelPlacement(nextPlacement);
      setPanelMaxHeight(Math.max(48, Math.floor(availableSpace - PANEL_PADDING_BUFFER)));
    };

    const frameId = window.requestAnimationFrame(updatePanelPosition);
    window.addEventListener('resize', updatePanelPosition);
    window.addEventListener('scroll', updatePanelPosition, true);

    return () => {
      window.cancelAnimationFrame(frameId);
      window.removeEventListener('resize', updatePanelPosition);
      window.removeEventListener('scroll', updatePanelPosition, true);
    };
  }, [dependencyKey, open, rootRef]);

  const scrollContainerStyle: CSSProperties | undefined =
    panelMaxHeight != null
      ? {
          maxHeight: `${panelMaxHeight}px`,
          overflowY: 'auto',
        }
      : undefined;

  return {
    panelRef,
    panelPlacement,
    panelMaxHeight,
    scrollContainerStyle,
  };
}
