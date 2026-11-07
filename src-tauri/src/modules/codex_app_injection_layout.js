// Shared by API quota and DeepSeek balance. Keep native composer controls in charge
// of layout: the detached badges may only occupy a measured, empty toolbar gap.
(() => {
  const padding = 8;
  const controlSelector = 'button, [role="button"], input, select, textarea, [data-composer-navigation-target], [role="status"], [role="progressbar"], canvas, svg';
  const ownSelector = '[data-cockpit-quota-footer], [data-cockpit-quota-details], [data-cockpit-deepseek-balance], [data-cockpit-deepseek-balance-details]';

  function visibleRect(node) {
    if (!node || node.getClientRects().length === 0) return null;
    const style = getComputedStyle(node);
    if (style.display === 'none' || style.visibility === 'hidden' || style.visibility === 'collapse') return null;
    const rect = node.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0 ? rect : null;
  }

  function occupiedNodes(footer) {
    return [...footer.querySelectorAll('*')].filter((node) => {
      if (node.closest(ownSelector)) return false;
      // Text-only native errors, recording timers, and status messages also need
      // room even when they are not buttons and carry no stable accessibility ID.
      return node.matches(controlSelector)
        || [...node.childNodes].some((child) => child.nodeType === 3 && child.textContent.trim());
    });
  }

  function findPlacement(footer, permissions, blockers, size, viewport) {
    if (!footer || !permissions || size.width <= 0 || size.height <= 0) return null;
    const top = permissions.top + permissions.height / 2;
    if (top - size.height / 2 < 0 || top + size.height / 2 > viewport.height) return null;
    const left = Math.max(padding, footer.left + padding, permissions.right + padding);
    const right = Math.min(viewport.width - padding, footer.right - padding);
    if (right - left < size.width) return null;
    const bandTop = top - size.height / 2 - 2;
    const bandBottom = top + size.height / 2 + 2;
    const intervals = blockers
      .filter((rect) => rect.bottom > bandTop && rect.top < bandBottom && rect.right > left && rect.left < right)
      .map((rect) => [Math.max(left, rect.left - padding), Math.min(right, rect.right + padding)])
      .sort((a, b) => a[0] - b[0]);
    let cursor = left;
    let best = null;
    for (const [start, end] of [...intervals, [right, right]]) {
      if (start - cursor >= size.width && (!best || start - cursor > best.width)) {
        best = { left: (cursor + start) / 2, top, width: start - cursor };
      }
      cursor = Math.max(cursor, end);
    }
    return best;
  }

  function observe(root, footer, permissions, nodes) {
    if (!('ResizeObserver' in window)) return;
    if (!root.layoutObserver) root.layoutObserver = new ResizeObserver(() => root.scheduleRender());
    const next = [footer, permissions, ...nodes];
    if (root.layoutNodes && root.layoutNodes.length === next.length
      && next.every((node, index) => root.layoutNodes[index] === node)) return;
    root.layoutObserver.disconnect();
    new Set(next).forEach((node) => root.layoutObserver.observe(node));
    root.layoutNodes = next;
  }

  function disconnect(root) {
    if (root.layoutObserver) root.layoutObserver.disconnect();
    root.layoutNodes = null;
  }

  function place(root, host, footer, permissions) {
    const nodes = occupiedNodes(footer);
    observe(root, footer, permissions, nodes);
    // The native footer sets this only while dictation owns the toolbar (both
    // waveform and live-transcript modes). Its idle microphone is still usable.
    if (footer.hasAttribute('data-dictation-view')) {
      host.style.display = 'none';
      return null;
    }
    // Measure the full, uncompressed badge group before deciding to show it.
    host.style.display = 'flex';
    host.style.visibility = 'hidden';
    const rect = host.getBoundingClientRect();
    const placement = findPlacement(visibleRect(footer), visibleRect(permissions),
      nodes.map(visibleRect).filter(Boolean), rect,
      { width: window.innerWidth, height: window.innerHeight });
    if (!placement) {
      host.style.display = 'none';
      return null;
    }
    host.style.left = placement.left + 'px';
    host.style.top = placement.top + 'px';
    host.style.visibility = 'visible';
    return placement;
  }

  return { place, disconnect, findPlacement };
})()
