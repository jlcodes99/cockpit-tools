import { OverlayScrollbars, type OverlayScrollbars as Scrollbars } from 'overlayscrollbars';
import { registerScrollbarOwner } from './scrollbarContainment';

interface Entry {
  element: HTMLElement;
  slot: HTMLDivElement;
  instance: Scrollbars;
  overflow: string;
}

/** Enhance native scroll owners without moving React children or changing refs. */
export function installAppScrollbars(doc: Document = document): () => void {
  const win = doc.defaultView!;
  const entries = new Map<HTMLElement, Entry>();
  const candidates = new Set<HTMLElement>();
  const pending = new Set<HTMLElement>();
  const layer = doc.createElement('div');
  layer.className = 'app-scrollbars';
  layer.setAttribute('aria-hidden', 'true');
  doc.body.append(layer);
  let frame = 0;
  let scanFrame = 0;
  let disposed = false;

  function isOwned(node: Node): boolean {
    return node === layer || layer.contains(node);
  }

  function schedulePosition(): void {
    if (!frame && !disposed) frame = win.requestAnimationFrame(positionAll);
  }

  function visibleAt(element: HTMLElement, x: number, y: number): boolean {
    const hit = doc.elementsFromPoint(x, y).find(node => !isOwned(node));
    return !!hit && (hit === element || element.contains(hit));
  }

  function isTextarea(element: HTMLElement): boolean {
    return element.tagName === 'TEXTAREA';
  }

  function detach(element: HTMLElement): void {
    const entry = entries.get(element);
    if (!entry) return;
    entry.instance.destroy();
    entry.slot.remove();
    entries.delete(element);
  }

  function positionAll(): void {
    frame = 0;
    // Textareas are never enhanced (see `enhance`); drop any stale instance that
    // survived a hot reload so we never measure their content here.
    let textareas: HTMLElement[] | null = null;
    for (const { element, slot, instance } of entries.values()) {
      if (isTextarea(element)) {
        (textareas ??= []).push(element);
        continue;
      }
      const rect = element.getBoundingClientRect();
      const style = win.getComputedStyle(element);
      const overflow = `${style.overflowX}/${style.overflowY}`;
      const entry = entries.get(element)!;
      if (entry.overflow !== overflow) {
        entry.overflow = overflow;
        instance.options({ overflow: {
          x: /^(auto|scroll)$/.test(style.overflowX) ? 'scroll' : 'hidden',
          y: /^(auto|scroll)$/.test(style.overflowY) ? 'scroll' : 'hidden',
        } });
      }
      const left = rect.left + element.clientLeft;
      const top = rect.top + element.clientTop;
      const width = element.clientWidth;
      const height = element.clientHeight;
      let clipLeft = Math.max(0, left);
      let clipTop = Math.max(0, top);
      let clipRight = Math.min(win.innerWidth, left + width);
      let clipBottom = Math.min(win.innerHeight, top + height);
      for (let parent = element.parentElement; parent && parent !== doc.body; parent = parent.parentElement) {
        const parentStyle = win.getComputedStyle(parent);
        if (/(auto|scroll|hidden|clip)/.test(parentStyle.overflowX + parentStyle.overflowY)) {
          const bounds = parent.getBoundingClientRect();
          if (parentStyle.overflowX !== 'visible') {
            clipLeft = Math.max(clipLeft, bounds.left + parent.clientLeft);
            clipRight = Math.min(clipRight, bounds.left + parent.clientLeft + parent.clientWidth);
          }
          if (parentStyle.overflowY !== 'visible') {
            clipTop = Math.max(clipTop, bounds.top + parent.clientTop);
            clipBottom = Math.min(clipBottom, bounds.top + parent.clientTop + parent.clientHeight);
          }
        }
      }
      const visible = width > 0 && height > 0 && clipRight > clipLeft && clipBottom > clipTop
        && style.visibility !== 'hidden' && style.display !== 'none';
      slot.style.display = visible ? '' : 'none';
      if (!visible) continue;
      Object.assign(slot.style, {
        left: `${left}px`, top: `${top}px`, width: `${width}px`, height: `${height}px`,
        clipPath: `inset(${clipTop - top}px ${left + width - clipRight}px ${top + height - clipBottom}px ${clipLeft - left}px)`,
      });
      const bars = instance.elements();
      // Hit testing prevents a background scrollbar from appearing above a modal
      // or a parent scrollbar from covering an overlapping child panel.
      const vertical = /^(auto|scroll)$/.test(style.overflowY)
        && visibleAt(element, style.direction === 'rtl' ? left + 3 : left + width - 3, (clipTop + clipBottom) / 2);
      const horizontal = /^(auto|scroll)$/.test(style.overflowX)
        && visibleAt(element, (clipLeft + clipRight) / 2, top + height - 3);
      bars.scrollbarVertical.scrollbar.style.visibility = vertical ? '' : 'hidden';
      bars.scrollbarHorizontal.scrollbar.style.visibility = horizontal ? '' : 'hidden';
    }
    if (textareas) for (const element of textareas) detach(element);
  }

  const resize = new ResizeObserver(records => {
    // Only rescan the element that actually resized. Rescanning every candidate
    // subtree here turns a single layout change into a document-wide re-measure
    // loop (getComputedStyle + scrollWidth forces layout), which can starve the
    // main thread and blow up renderer memory on large content.
    let added = false;
    for (const record of records) {
      const target = record.target;
      if (target instanceof win.HTMLElement && !isOwned(target)) {
        pending.add(target);
        added = true;
      }
    }
    if (added) scheduleScan();
    schedulePosition();
  });

  function enhance(element: HTMLElement): void {
    if ((element === doc.documentElement || element === doc.body)
      && element !== doc.scrollingElement) return;
    if (isOwned(element)) return;
    // Textareas hold unbounded text (users paste multi-megabyte JSON) and every
    // overlay-scrollbar measurement walks all of it. Keep their native scrollbar.
    if (isTextarea(element)) {
      detach(element);
      return;
    }
    const style = win.getComputedStyle(element);
    const scrollX = /^(auto|scroll)$/.test(style.overflowX);
    const scrollY = /^(auto|scroll)$/.test(style.overflowY);
    if (!scrollX && !scrollY) return;
    if (!candidates.has(element)) {
      candidates.add(element);
      resize.observe(element);
    }
    const existing = entries.get(element);
    if (existing) {
      existing.instance.update();
      return;
    }
    if (!element.clientWidth || !element.clientHeight) return;
    if (!(scrollX && element.scrollWidth > element.clientWidth)
      && !(scrollY && element.scrollHeight > element.clientHeight)) return;
    const slot = doc.createElement('div');
    slot.className = 'app-scrollbar-slot';
    layer.append(slot);
    registerScrollbarOwner(slot, element);
    const instance = OverlayScrollbars({
      target: element,
      elements: { viewport: element, content: false, padding: false },
      scrollbars: { slot },
    }, {
      overflow: { x: scrollX ? 'scroll' : 'hidden', y: scrollY ? 'scroll' : 'hidden' },
      scrollbars: {
        theme: 'os-theme-cockpit', autoHide: 'leave', autoHideDelay: 600,
        dragScroll: true, clickScroll: 'instant',
      },
    }, { updated: schedulePosition, scroll: schedulePosition });
    entries.set(element, { element, slot, instance, overflow: `${style.overflowX}/${style.overflowY}` });
  }

  function scan(): void {
    scanFrame = 0;
    const roots = [...pending];
    pending.clear();
    for (const root of roots) {
      if (!root.isConnected || isOwned(root)) continue;
      if (roots.some(parent => parent !== root && parent.contains(root))) continue;
      enhance(root);
      root.querySelectorAll<HTMLElement>('*').forEach(enhance);
    }
    for (const element of candidates) {
      if (element.isConnected) continue;
      const entry = entries.get(element);
      entry?.instance.destroy();
      entry?.slot.remove();
      entries.delete(element);
      candidates.delete(element);
      resize.unobserve(element);
    }
    schedulePosition();
  }

  function scheduleScan(): void {
    if (!scanFrame && !disposed) scanFrame = win.requestAnimationFrame(scan);
  }

  const observer = new MutationObserver(records => {
    for (const record of records) {
      if (isOwned(record.target)) continue;
      if (record.type === 'attributes' && record.attributeName === 'style'
        && entries.has(record.target as HTMLElement)) {
        // OverlayScrollbars updates its CSS variables on the viewport. Observe
        // overflow locks too, but avoid scanning the whole subtree for these.
        schedulePosition();
        continue;
      }
      const target = record.target instanceof HTMLElement ? record.target : record.target.parentElement;
      if (target) pending.add(target);
    }
    if (pending.size) scheduleScan();
    schedulePosition();
  });
  observer.observe(doc.documentElement, {
    subtree: true, childList: true, characterData: true,
    attributes: true, attributeFilter: ['class', 'style', 'hidden', 'open'],
  });
  const rescan = () => { pending.add(doc.documentElement); scheduleScan(); };
  const input = (event: Event) => {
    if (event.target instanceof HTMLElement && !isOwned(event.target)) {
      if (isTextarea(event.target)) {
        detach(event.target);
        return;
      }
      entries.get(event.target)?.instance.update(true);
      pending.add(event.target);
      scheduleScan();
    }
  };
  win.addEventListener('resize', rescan);
  doc.addEventListener('scroll', schedulePosition, true);
  doc.addEventListener('pointerover', schedulePosition, true);
  doc.addEventListener('input', input, true);
  doc.addEventListener('load', rescan, true); // lazy-loaded page styles
  doc.addEventListener('transitionend', rescan, true);
  doc.addEventListener('animationend', rescan, true);
  doc.addEventListener('animationcancel', rescan, true);
  doc.fonts?.addEventListener('loadingdone', rescan);
  rescan();
  return () => {
    disposed = true;
    win.cancelAnimationFrame(frame);
    win.cancelAnimationFrame(scanFrame);
    observer.disconnect();
    resize.disconnect();
    win.removeEventListener('resize', rescan);
    doc.removeEventListener('scroll', schedulePosition, true);
    doc.removeEventListener('pointerover', schedulePosition, true);
    doc.removeEventListener('input', input, true);
    doc.removeEventListener('load', rescan, true);
    doc.removeEventListener('transitionend', rescan, true);
    doc.removeEventListener('animationend', rescan, true);
    doc.removeEventListener('animationcancel', rescan, true);
    doc.fonts?.removeEventListener('loadingdone', rescan);
    entries.forEach(({ instance }) => instance.destroy());
    layer.remove();
  };
}
