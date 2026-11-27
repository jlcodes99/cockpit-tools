(dictionary, templates) => {
  if (window.top !== window || location.protocol !== 'https:' || location.hostname !== '127.0.0.1') return false;
  if (!document.body || (!window.__APP_CONFIG__ && !document.querySelector('[data-agent-composer-root]'))) return false;
  if (window.__cockpitAntigravityLocalization) return window.__cockpitAntigravityLocalization.setEnabled(true);

  const chromeSelector = [
    'button', 'label', 'a[href]', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'summary',
    '[role="button"]', '[role="heading"]', '[role="tab"]', '[role="menuitem"]',
    '[role="menuitemcheckbox"]', '[role="menuitemradio"]', '[role="option"]', '[role="tooltip"]',
    '[role="switch"]', '[role="checkbox"]', '[role="radio"]', '[role="combobox"]',
    '[role="status"]', '[role="alert"]',
    '[data-testid="menu-option-label"]',
    '[data-agent-composer-root] p.pointer-events-none.select-none',
  ].join(',');
  const protectedSelector = [
    'script', 'style', 'pre', 'code', 'kbd', 'svg', 'iframe', 'webview',
    '[contenteditable]:not([contenteditable="false"])', '[role="textbox"]', '[role="code"]', '[role="treeitem"]',
    '.monaco-editor', '.xterm', '.markdown-body', '.prose', '.chat-message',
    '.tool-output', '.terminal-output', '[data-markdown-root]', '[data-quotable]',
    '[data-message-id]', '[data-cos-artifact]', '[data-diff-type]',
    '[data-role="assistant"]', '[data-role="user"]',
    '[data-testid*="message-content"]', '[data-testid*="user-message"]', '[data-testid*="assistant-message"]',
    '[data-testid*="thinking-content"]', '[data-testid*="tool-output"]', '[data-testid*="terminal-output"]',
    '[data-testid*="conversation-list"]', '[data-testid*="conversation-title"]', '[data-testid*="conversation-item"]',
    '[data-project-name]', '[data-testid*="project-name"]', '[data-testid*="project-item"]', '[data-testid*="workspace-name"]',
    '[data-testid*="workspace-item"]', '[data-testid*="file-tree"]',
    '[data-testid*="profile-name"]', '[data-testid*="user-name"]',
    '[data-testid="automation-name"]', '[data-testid="automation-instructions-text"]',
    '[data-testid*="account-menu"]',
    '[data-testid="model-selector-orchestrator-model"]',
    '[data-testid="model-selector-effort-group"]', '[data-testid="orchestrator-picker-row"] span.truncate',
    '[data-testid="orchestrator-picker-default"] span.truncate',
    '[data-testid="model-selector-item"] span.truncate', '[data-testid="model-selector-custom-item"] span.truncate',
    '[data-cockpit-ag-capsule]', '[data-cockpit-ag-details]',
  ].join(',');
  const settingsSidebar = '[data-testid="settings-sidebar"]';
  const settingsNav = '[id^="settings-nav-item-"],[data-testid^="settings-nav-item-"]';
  const namedSelectorTrigger = '[data-testid="model-selector-trigger"],[data-testid="project-selector-trigger"]';
  const uiSurfaces = [
    '.settings-content-pane', '[data-settings-content-pane="true"]', '.settings-modal-container',
    '[role="dialog"]:has([data-testid="settings-sidebar"])',
    '[data-testid="declared-permissions-modal"]', '[data-testid="post-onboarding-welcome-modal"]',
    '[data-testid="computer-permissions-card"]', '[data-testid="browser-cert-interstitial"]',
    '[data-testid="browser-error-overlay"]', '[data-testid="workspace-mismatch-banner"]',
    '[data-testid="automation-settings"]', '[data-testid="automation-schedule-editor"]',
    '[data-testid="project-folder-selection-step"]', '[data-testid="cloud-resource-disclosure-modal"]',
    '[data-testid="terminal-empty"]', '[data-testid="bottom-terminal-panel-empty"]',
    '[data-testid="queued-messages-status"]', '[data-testid="toast-notification"]',
    '[data-testid="model-selector-header"]', '[data-testid="model-selector-empty"]',
    '[data-testid="model-selector-loading"]', '[data-testid="model-selector-effort-header"]',
    '[data-testid="artifact-update-notice"]',
    '[data-sidebar-section-drop-id] > span.select-none.opacity-30',
    '[data-queued-msg-id] > div:first-child > span.line-clamp-2',
  ].join(',');
  const settingsSidebarLabels = new Set([
    'Settings', 'General', 'Application', 'Appearance', 'Shortcuts', 'Keyboard Shortcuts',
    'Notifications', 'Models', 'Browser', 'Projects', 'Show All', 'Show Less', 'Not in Project',
    'Conversations', 'Give Feedback', 'Account',
  ]);
  const controlSelector = `${chromeSelector},input,textarea,select`;
  const attributes = ['title', 'aria-label', 'aria-description', 'placeholder'];
  const placeholder = /\{(\d+)\}/g;
  const templateRules = Object.entries(templates).map(([source, translation]) => {
    const parts = source.split(placeholder);
    const groups = new Map();
    const pattern = parts.map((part, index) => {
      if (index % 2 === 0) return part.replace(/[.*+?^${}()|[\]\\]/g, '\\$&').replaceAll(' ', '\\s+');
      if (groups.has(part)) return `\\k<slot${part}>`;
      groups.set(part, true);
      return `(?<slot${part}>[\\s\\S]+?)`;
    }).join('');
    const literalLength = parts.reduce((length, part, index) => length + (index % 2 === 0 ? part.length : 0), 0);
    return { prefix: parts[0], suffix: parts.at(-1), literalLength, expression: new RegExp(`^${pattern}$`, 'u'), translation };
  }).sort((a, b) => b.literalLength - a.literalLength);
  const textEdits = new Map();
  const attributeEdits = new Map();
  const added = new Set();
  let nodesWereRemoved = false;
  let frame = null;
  let enabled = false;

  function restore(edits, key, current, write) {
    const edit = edits.get(key);
    if (edit && current === edit.translated) write(edit.original);
    edits.delete(key);
  }

  function resolveTranslation(label) {
    const normalized = label.replace(/\s+/gu, ' ');
    if (Object.hasOwn(dictionary, normalized)) return dictionary[normalized];
    for (const { prefix, suffix, expression, translation } of templateRules) {
      if (!normalized.startsWith(prefix) || !normalized.endsWith(suffix)) continue;
      const match = expression.exec(label);
      if (match) return translation.replace(placeholder, (_, index) => match.groups[`slot${index}`]);
    }
    return null;
  }

  function translate(edits, key, current, write) {
    if (edits.get(key)?.translated === current) return;
    edits.delete(key);
    const label = current?.trim();
    if (!label) return;
    const replacement = resolveTranslation(label);
    if (!replacement || replacement === label) return;
    const translated = current.replace(label, () => replacement);
    edits.set(key, { original: current, translated });
    write(translated);
  }

  function canTranslateText(element, value) {
    if (!element || element.closest('textarea') || element.closest(namedSelectorTrigger)) return false;
    if (element.closest(settingsSidebar)) {
      return Boolean(element.closest(settingsNav)) || settingsSidebarLabels.has(value?.trim());
    }
    return Boolean(element.closest(chromeSelector) || element.closest(uiSurfaces));
  }

  function canTranslateAttribute(element, attr, value) {
    if (element.closest(namedSelectorTrigger)) {
      return attr === 'aria-label' && element.matches(namedSelectorTrigger);
    }
    if (element.closest(settingsSidebar) && !canTranslateText(element, value)) return false;
    return element.matches(controlSelector) || Boolean(element.closest(uiSurfaces));
  }

  function translateNode(node) {
    if (node.nodeType === Node.TEXT_NODE) {
      if (canTranslateText(node.parentElement, node.nodeValue)) {
        translate(textEdits, node, node.nodeValue, (value) => { node.nodeValue = value; });
      }
      return;
    }

    let edits = attributeEdits.get(node);
    for (const attr of attributes) {
      const value = node.getAttribute(attr);
      if (value === null && !edits?.has(attr)) continue;
      if (!canTranslateAttribute(node, attr, value)) continue;
      if (!edits) edits = new Map();
      translate(edits, attr, value, (next) => node.setAttribute(attr, next));
    }
    if (edits?.size) attributeEdits.set(node, edits);
    else attributeEdits.delete(node);
  }

  function restoreNode(node) {
    if (node.nodeType === Node.TEXT_NODE) {
      restore(textEdits, node, node.nodeValue, (value) => { node.nodeValue = value; });
      return;
    }
    const edits = attributeEdits.get(node);
    if (!edits) return;
    for (const attr of edits.keys()) {
      restore(edits, attr, node.getAttribute(attr), (value) => node.setAttribute(attr, value));
    }
    attributeEdits.delete(node);
  }

  function translateSubtree(root) {
    if (root.nodeType === Node.TEXT_NODE) {
      if (!root.parentElement?.closest(protectedSelector)) translateNode(root);
      return;
    }
    if (root.nodeType !== Node.ELEMENT_NODE || root.closest(protectedSelector)) return;
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT, {
      acceptNode: (node) => node.nodeType === Node.ELEMENT_NODE && node.matches(protectedSelector)
        ? NodeFilter.FILTER_REJECT
        : NodeFilter.FILTER_ACCEPT,
    });
    translateNode(root);
    while (walker.nextNode()) translateNode(walker.currentNode);
  }

  function restoreRemovedOrMovedNodes() {
    for (const node of textEdits.keys()) {
      const element = node.parentElement;
      if (!node.isConnected || !canTranslateText(element, textEdits.get(node).original) || element.closest(protectedSelector)) {
        restoreNode(node);
      }
    }
    for (const [element, edits] of attributeEdits) {
      if (!element.isConnected || element.closest(protectedSelector)) {
        restoreNode(element);
        continue;
      }
      for (const [attr, { original }] of edits) {
        if (!canTranslateAttribute(element, attr, original)) {
          restore(edits, attr, element.getAttribute(attr), (value) => element.setAttribute(attr, value));
        }
      }
      if (!edits.size) attributeEdits.delete(element);
    }
  }

  function schedule() {
    if (frame !== null) return;
    frame = window.requestAnimationFrame(() => {
      frame = null;
      if (nodesWereRemoved) restoreRemovedOrMovedNodes();
      nodesWereRemoved = false;
      for (const root of added) {
        if (root.isConnected) translateSubtree(root);
      }
      added.clear();
    });
  }

  const observer = new MutationObserver((records) => {
    for (const record of records) {
      const target = record.target;
      const element = target.nodeType === Node.ELEMENT_NODE ? target : target.parentElement;
      if (!element || element.closest(protectedSelector)) continue;
      if (record.type === 'childList') {
        nodesWereRemoved ||= record.removedNodes.length > 0;
        for (const node of record.addedNodes) added.add(node);
      } else if (record.type === 'characterData') {
        if (target.nodeValue !== textEdits.get(target)?.translated && canTranslateText(element, target.nodeValue)) added.add(target);
      } else if (element.getAttribute(record.attributeName) !== attributeEdits.get(element)?.get(record.attributeName)?.translated) {
        translateNode(element);
      }
    }
    if (added.size || nodesWereRemoved) schedule();
  });

  function setEnabled(next) {
    if (enabled === next) return true;
    enabled = next;
    if (enabled) {
      observer.observe(document.body, {
        subtree: true, childList: true, characterData: true, attributeFilter: attributes,
      });
      added.add(document.body);
      schedule();
    } else {
      observer.disconnect();
      if (frame !== null) window.cancelAnimationFrame(frame);
      frame = null;
      added.clear();
      nodesWereRemoved = false;
      for (const node of textEdits.keys()) restoreNode(node);
      for (const node of attributeEdits.keys()) restoreNode(node);
    }
    return true;
  }

  window.__cockpitAntigravityLocalization = { setEnabled };
  return setEnabled(true);
}
