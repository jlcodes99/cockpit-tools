import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { runInNewContext } from 'node:vm';
import { getCodexQuotaClass } from '../src/types/codex';
import { getGrokQuotaClass } from '../src/types/grok';

const source = readFileSync(new URL('../src-tauri/src/modules/codex_app_injection.rs', import.meta.url), 'utf8');
const translations = JSON.parse(readFileSync(new URL('../src/locales/en.json', import.meta.url), 'utf8'));
const usageLabels = translations.common.codexUsage;
const statuslineSource = readFileSync(new URL('../src-tauri/src/modules/codex_app_injection_statusline.rs', import.meta.url), 'utf8');
type ScriptKind = 'injection_script' | 'deepseek_balance_injection_script';
const values: Record<string, unknown> = {
  provider: 'Provider', weekly: 50, five_hour: 75, account_count_value: 2,
  five_hour_capacity: 200, weekly_capacity: 200,
  available_account_count_value: 2, abnormal_account_count_value: 0, cooldown_account_count_value: 0,
  plans: [], grok_lines_value: [], grok_badge_percent: 90, grok_badge_product: 'GrokBuild',
  official_label: '官',
  badge_kind_value: 'codex', fallback_model_value: 'gpt-5.4',
  account_pool_label: 'Accounts', plan_label: 'Plan', weekly_label: 'Week', five_hour_label: '5h',
  account_pool_title: 'Account pool', quota_empty_label: 'No quota', available_text: 'Available',
  issue_text: 'Issues', refresh_label: 'Refresh', close_label: 'Close', missing_label: '—',
  balance_label: 'Balance', account_balance_label: 'Account balance', total_balance_label: 'Total balance',
  quota_overview_label: 'Quota', grok_empty_label: 'No quota',
  balance_lines: [{ totalBalance: 10, currency: 'CNY' }],
  refresh_in_progress: false, handled_refresh_token: null,
  balance: { totalBalance: 10, currency: 'CNY', isAvailable: true },
  granted_balance_label: 'Granted', topped_up_balance_label: 'Top-up',
  available_label: 'Available', unavailable_label: 'Unavailable',
  usage_state: {}, refresh_error: null,
  usage_labels: {
    ...usageLabels,
    requestTier:translations.codex.apiService.logs.requestedTier,responseTier:translations.codex.apiService.logs.responseTier,
    legacyTier:translations.codex.apiService.logs.legacyTier,tierHint:translations.codex.apiService.logs.serviceTierHint,
    tierFast:translations.codex.speed.fast,tierUltrafast:translations.codex.speed.ultrafast,tierStandard:translations.codex.speed.standard,
    tierAuto:translations.codex.contextOverride.automatic,tierUnknown:translations.common.requestProxy.unknown,
    title: 'Quota and usage', session: 'Current session', speed: 'Output speed', cache: 'Cache hits', cacheShort: 'Cache',
    speedHint: 'Current chat output tokens divided by post-first-response generation time', cacheHint: 'Cached input divided by input tokens',
    aggregateHint: 'Sum of account percentages without plan capacity weighting', waiting: 'Waiting for session tokens',
    refreshFailed: 'Quota refresh failed. Try again.', refreshPartial: 'Some account quotas failed to refresh.',
    hostUnavailable: 'Host unavailable', refreshing: 'Refreshing', recent: 'Recent sample',
    recentAccount: 'Recent account', noRecentAccount: 'No verified record', recentAccountHint: 'Verified request for this conversation',
  },
};

// Execute the shipped raw JS template, including Rust format! brace escaping.
// No duplicate implementation of model detection or badge rendering lives in this test.
function script(kind: ScriptKind, overrides: Record<string, unknown> = {}) {
  const templateSource = kind === 'injection_script' ? statuslineSource : source;
  const arguments_ = { ...values, ...overrides };
  const functionStart = templateSource.indexOf(`fn ${kind}(`);
  assert.ok(functionStart >= 0, `missing ${kind}`);
  const start = templateSource.indexOf('r#"(()', functionStart);
  assert.ok(start >= 0, `missing raw JS template for ${kind}`);
  const end = templateSource.indexOf('"#', start + 3);
  assert.ok(end > start);
  return templateSource.slice(start + 3, end).replace(/\{\{|\}\}|\{([a-z_]+)\}/g, (token, key: string | undefined) => {
    if (token === '{{') return '{';
    if (token === '}}') return '}';
    if (key === 'badge_layout') return readFileSync(new URL('../src-tauri/src/modules/codex_app_injection_layout.js', import.meta.url), 'utf8');
    if (key === 'usage_metrics') return readFileSync(new URL('../src-tauri/src/modules/codex_app_injection_usage.js', import.meta.url), 'utf8');
    if (key === 'dialog_renderer') return readFileSync(new URL('../src-tauri/src/modules/codex_app_injection_dialog.js', import.meta.url), 'utf8');
    if (key === 'dialog_styles') return JSON.stringify(readFileSync(new URL('../src-tauri/src/modules/codex_app_injection_dialog.css', import.meta.url), 'utf8'));
    if (key === 'renderer') return readFileSync(new URL('../src-tauri/src/modules/codex_app_injection_statusline.js', import.meta.url), 'utf8');
    assert.ok(key && Object.hasOwn(arguments_, key), `unhandled template argument ${key}`);
    return JSON.stringify(arguments_[key!]);
  });
}

class Style {
  [property: string]: any;
  setProperty(name: string, value: string) { this[name] = String(value); }
  getPropertyValue(name: string): string { return typeof this[name] === 'string' ? this[name] : ''; }
  removeProperty(name: string): string { const previous = this.getPropertyValue(name); delete this[name]; return previous; }
}

const escapeMarkup = (value: string) => value.replace(/[&<>"']/g, char => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[char]!));

class Element {
  constructor(readonly tagName = "div") {}
  readonly nodeType = 1;
  get childNodes() { return [...this.children, ...(this.ownText ? [{ nodeType: 3, textContent: this.ownText }] : [])]; }
  hasAttribute(name: string) { return Object.hasOwn(this.attributes, name); }
  getAttribute(name: string) { return this.attributes[name] ?? null; }
  removeAttribute(name: string) {
    delete this.attributes[name];
    if (name === 'style') for (const property of Object.keys(this.style)) delete this.style[property];
  }
  children: Element[] = [];
  insertions: Array<{ node: Element; progress: string | null; width: string; tone: string | null }> = [];
  parentElement: Element | null = null;
  attributes: Record<string, string> = {};
  style = new Style();
  private ownText = '';
  get textContent(): string { return this.ownText + this.children.map(child => child.textContent).join(''); }
  set textContent(value: string) {
    this.ownText = String(value);
    this.children.forEach(child => { child.parentElement = null; });
    this.children = [];
  }
  get innerHTML(): string { return escapeMarkup(this.ownText) + this.children.map(child => child.outerHTML).join(''); }
  get outerHTML(): string {
    const attributes = Object.entries(this.attributes).filter(([name]) => name !== 'style');
    const style = Object.entries(this.style).filter(([, value]) => typeof value === 'string')
      .map(([name, value]) => `${name}:${value}`).join(';');
    if (style) attributes.push(['style', style]);
    const opening = '<' + this.tagName + attributes.map(([name, value]) => ` ${name}="${escapeMarkup(value)}"`).join('') + '>';
    return opening + (['input', 'br', 'hr', 'img', 'col'].includes(this.tagName) ? '' : this.innerHTML + `</${this.tagName}>`);
  }
  set innerHTML(value: string) {
    this.children.forEach(child => { child.parentElement = null; });
    this.children = [];
    this.ownText = '';
    const stack: Element[] = [this];
    const decode = (text: string) => text.replace(/&(?:amp|lt|gt|quot|#39);/g, entity => ({ '&amp;': '&', '&lt;': '<', '&gt;': '>', '&quot;': '"', '&#39;': "'" }[entity]!));
    for (const token of value.match(/<[^>]+>|[^<]+/g) || []) {
      if (token.startsWith('</')) { stack.pop(); continue; }
      const tag = /^<([\w-]+)/.exec(token);
      if (!tag) { stack.at(-1)!.ownText += decode(token); continue; }
      const element = new Element(tag[1]);
      const attributes = token.slice(tag[0].length, -1);
      for (const match of attributes.matchAll(/([\w-]+)(?:="([^"]*)")?/g)) element.setAttribute(match[1], decode(match[2] || ''));
      stack.at(-1)!.appendChild(element);
      if (!/\/$/.test(token.slice(0, -1)) && !['input', 'br', 'hr', 'img', 'col'].includes(tag[1])) stack.push(element);
    }
  }
  onclick: (() => void) | null = null;
  onkeydown: ((event: any) => void) | null = null;
  activeElement: Element | null = null;
  focused = false;
  get disabled() { return this.hasAttribute('disabled'); }
  set disabled(value: boolean) { if (value) this.attributes.disabled = ''; else delete this.attributes.disabled; }
  get isConnected(): boolean { return this.tagName === 'html' || Boolean(this.parentElement?.isConnected); }
  get previousElementSibling() { const index = this.parentElement?.children.indexOf(this) ?? -1; return index > 0 ? this.parentElement!.children[index - 1] : null; }
  click() { if (!this.disabled) { if (this.onclick) this.onclick(); else this.parentElement?.click(); } }
  press(key: string) { if (this.tagName === 'button' && (key === 'Enter' || key === ' ')) this.click(); }
  focus() {
    if (this.disabled) return;
    let root: Element = this;
    while (root.parentElement) root = root.parentElement;
    if (root.activeElement) root.activeElement.focused = false;
    root.activeElement = this;
    this.focused = true;
  }
  scrollTop = 0;
  hidden = false;
  rect = { left: 100, top: 900, width: 80, height: 24 };
  measureRect: ((node: Element) => typeof this.rect | null) | null = null;

  setAttribute(key: string, value: string) {
    this.attributes[key] = String(value);
    if (key === 'style') {
      for (const property of Object.keys(this.style)) delete this.style[property];
      for (const declaration of value.split(';')) {
        const separator = declaration.indexOf(':');
        if (separator >= 0) this.style.setProperty(declaration.slice(0, separator).trim(), declaration.slice(separator + 1).trim());
      }
    }
  }
  appendChild(node: Element) {
    node.remove();
    node.parentElement = this;
    this.children.push(node);
    return node;
  }
  insertBefore(node: Element, reference: Element | null) {
    if (reference === node) return node;
    this.insertions.push({ node, progress: node.getAttribute('data-cockpit-progress'), width: node.style.getPropertyValue('--cockpit-quota-fill-width'), tone: node.getAttribute('data-cockpit-quota-tone') });
    if (reference === null) return this.appendChild(node);
    assert.equal(reference.parentElement, this, 'insertBefore reference must belong to the parent');
    node.remove();
    node.parentElement = this;
    this.children.splice(this.children.indexOf(reference), 0, node);
    return node;
  }
  insertAdjacentElement(position: string, node: Element) {
    assert.equal(position, 'afterend');
    assert.ok(this.parentElement);
    node.remove();
    node.parentElement = this.parentElement;
    this.parentElement.children.splice(this.parentElement.children.indexOf(this) + 1, 0, node);
    node.rect = { ...this.rect, top: this.rect.top + this.rect.height + 6, height: 17 };
  }
  remove() {
    if (this.parentElement) this.parentElement.children = this.parentElement.children.filter((child) => child !== this);
    this.parentElement = null;
  }
  contains(node: Element): boolean { return node === this || this.children.some((child) => child.contains(node)); }
  matches(selector: string) {
    return selector.split(',').some((part) => {
      let rest = part.trim();
      if (rest === '*') return true;
      let excluded = false;
      rest = rest.replace(/:not\(([^)]+)\)/g, (_, inner: string) => { if (this.matches(inner)) excluded = true; return ''; });
      if (excluded) return false;
      if (rest === ':disabled') return this.disabled;
      const tag = /^[a-z]+/.exec(rest);
      if (tag) { if (this.tagName !== tag[0]) return false; rest = rest.slice(tag[0].length); }
      const className = /^\.([\w-]+)/.exec(rest);
      if (className) {
        if (!(this.attributes.class || '').split(/\s+/).includes(className[1])) return false;
        rest = rest.slice(className[0].length);
      }
      while (rest) {
        const match = /^\[([^=*\]]+)(?:(\*?=)"([^"]*)"( i)?)?\]/.exec(rest);
        assert.ok(match, `unsupported mock selector ${part}`);
        let value = this.attributes[match[1]];
        if (value === undefined) return false;
        if (match[2]) {
          let expected = match[3];
          if (match[4]) { value = value.toLowerCase(); expected = expected.toLowerCase(); }
          if (match[2] === '*=' ? !value.includes(expected) : value !== expected) return false;
        }
        rest = rest.slice(match[0].length);
      }
      return true;
    });
  }
  querySelectorAll(selector: string): Element[] {
    return this.children.flatMap((child) => [...(child.matches(selector) ? [child] : []), ...child.querySelectorAll(selector)]);
  }
  querySelector(selector: string) { return this.querySelectorAll(selector)[0] ?? null; }
  closest(selector: string): Element | null {
    return this.matches(selector) ? this : this.parentElement?.closest(selector) ?? null;
  }
  getClientRects() {
    if (this.hidden || this.parentElement?.getClientRects().length === 0) return [];
    return [this.rect];
  }
  getBoundingClientRect() {
    let root: Element = this;
    while (root.parentElement) root = root.parentElement;
    const rect = this.getClientRects().length ? root.measureRect?.(this) || this.rect : { left: 0, top: 0, width: 0, height: 0 };
    const metrics = this.querySelectorAll('[data-cockpit-quota-metric]');
    const isStrip = this.hasAttribute('data-cockpit-quota-footer') && metrics.length > 0;
    // Approximate glyph widths at the shipped 11/12/10px label/value/unit sizes.
    // The shared shell adds 14px plus an 11px decorative dot; fields add 10px padding and 4px flex gaps.
    // Speed also includes a 10px icon and its 7px separator/spacing adjustment.
    const width = isStrip ? 25 + metrics.reduce((sum, metric) => {
      const parts = metric.children;
      const textWidth = parts.reduce((total, part) => total + (part.hasAttribute('data-cockpit-metric-icon')
        ? 10 : part.textContent.trim().length * (part.hasAttribute('data-cockpit-metric-unit')
          ? 5 : part.hasAttribute('data-cockpit-metric-label') ? 5.5 : 6)), 0);
      return sum + textWidth + 10 + Math.max(0, parts.length - 1) * 4
        + (metric.getAttribute('data-cockpit-metric-key') === 'speed' ? 7 : 0);
    }, 0) : rect.width;
    const height = isStrip ? 28 : rect.height;
    return { ...rect, width, height, right: rect.left + width, bottom: rect.top + height };
  }
}

const firstThread = '01a157ce-ea9c-7683-940e-db6cb1f236db';
const secondThread = 'aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee';
function composer(model: string, hidden = false, top = 900, threadId = firstThread) {
  const root = new Element();
  root.setAttribute('data-codex-composer-root', 'true');
  root.setAttribute('data-composer-placement', 'thread');
  const conversation = new Element();
  conversation.setAttribute('data-above-composer-conversation-id', threadId);
  const card = new Element();
  card.setAttribute('data-composer-surface-variant', 'default');
  card.rect = { left: 100, top: top - 100, width: 800, height: 130 };
  card.hidden = hidden;
  const editor = new Element();
  editor.setAttribute('class', 'ProseMirror');
  editor.setAttribute('contenteditable', 'true');
  const footer = new Element();
  footer.setAttribute('class', '_ComposerLayoutFooter_test');
  footer.rect = { left: 100, top, width: 800, height: 30 };
  footer.hidden = hidden;
  const permissions = new Element();
  permissions.setAttribute('data-composer-navigation-target', 'permissions');
  permissions.rect.top = top;
  const trigger = new Element();
  trigger.setAttribute('data-composer-navigation-target', 'reasoning');
  trigger.textContent = model;
  trigger.rect = { left: 850, top, width: 50, height: 24 };
  footer.appendChild(permissions);
  footer.appendChild(trigger);
  card.appendChild(editor);
  card.appendChild(footer);
  root.appendChild(conversation);
  root.appendChild(card);
  return { root, card, editor, conversation, footer, trigger, permissions };
}

function dropdown(model: string, hidden = false) {
  const dropdown = new Element();
  dropdown.setAttribute('class', '_ModelPickerDropdownContent_test');
  dropdown.hidden = hidden;
  const row = new Element();
  row.setAttribute('class', '_ViewToggleModelLabel_5d7qv_2');
  row.textContent = model;
  dropdown.appendChild(row);
  return dropdown;
}

function harness(kind: ScriptKind, composers: ReturnType<typeof composer>[]) {
  const html = new Element('html');
  const body = new Element();
  const head = new Element();
  html.appendChild(body);
  html.appendChild(head);
  composers.forEach(({ root }) => body.appendChild(root));
  const handlers = new Map<string, Set<(event: any) => void>>();
  const intervals = new Map<number, () => void>();
  const document = {
    body, head, documentElement: html, createElement: (tag: string) => new Element(tag),
    get activeElement() { return html.activeElement?.isConnected ? html.activeElement : body; },
    querySelector: (selector: string) => html.querySelector(selector),
    querySelectorAll: (selector: string) => html.querySelectorAll(selector),
  };
  class ResizeObserver {
    targets: Element[] = [];
    observe(node: Element) { this.targets.push(node); }
    disconnect() { this.targets = []; }
  }
  class MutationObserver { observe() {} disconnect() {} }
  const window: Record<string, any> = {
    innerWidth: 1280, innerHeight: 1000, ResizeObserver,
    setInterval(fn: () => void) { const id = intervals.size + 1; intervals.set(id, fn); return id; },
    clearInterval(id: number) { intervals.delete(id); },
    addEventListener(name: string, fn: (event: any) => void) { if (!handlers.has(name)) handlers.set(name, new Set()); handlers.get(name)!.add(fn); },
    removeEventListener(name: string, fn: (event: any) => void) { handlers.get(name)?.delete(fn); },
  };
  let now = Date.now();
  class Clock extends Date { static now() { return now; } }
  const context = { getComputedStyle: (node: Element) => ({ display: node.hidden ? "none" : node.style.display || "block", visibility: node.style.visibility || "visible" }), document, window, Date: Clock, ResizeObserver, MutationObserver, requestAnimationFrame: (fn: () => void) => fn() };
  const namespace = kind === 'injection_script' ? '__cockpitCodexInjection' : '__cockpitDeepSeekBalance';
  const marker = kind === 'injection_script' ? 'quota-footer' : 'deepseek-balance';
  return {
    body, head, window, handlers, intervals,
    measure: (read: NonNullable<Element['measureRect']>) => { html.measureRect = read; },
    evaluate: (overrides: Record<string, unknown> = {}) => runInNewContext(script(kind, overrides), context),
    evaluateAs: (nextKind: ScriptKind) => runInNewContext(script(nextKind), context),
    root: () => window[namespace],
    host: () => document.querySelector(`[data-cockpit-${marker}]`)!,
    details: () => document.querySelector(`[data-cockpit-${kind === 'injection_script' ? 'quota-details' : 'deepseek-balance-details'}]`)!,
    dispatch: (name: string, event: Record<string, unknown> = {}) => handlers.get(name)?.forEach(fn => fn({ preventDefault() {}, ...event })),
    now: () => now,
    advance: (milliseconds: number) => { now += milliseconds; },
  };
}

test('quota status strip uses the visible GPT toolbar gap without changing composer structure', () => {
  const active = composer('gpt-5.4');
  const h = harness('injection_script', [composer('deepseek-v4-pro', true), active]);
  assert.equal(h.evaluate().selectedModel, 'gpt-5.4');
  assert.equal(h.host().parentElement, h.body);
  assert.equal(h.host().style.top, '912px');
  assert.equal(h.host().style.left, '515px');
  assert.match(h.host().textContent, /5h 75%/);
  assert.doesNotMatch(h.host().textContent, /Balance/);
  assert.deepEqual(h.root().layoutObserver.targets, [active.footer, active.permissions, active.trigger]);
});

test('quota badge reads visible DeepSeek and updates the model, badge, and observers on conversation change', () => {
  const old = composer('deepseek-v4-pro');
  const h = harness('injection_script', [composer('gpt-5.4', true), old]);
  assert.equal(h.evaluate().selectedModel, 'deepseek-v4-pro');
  assert.match(h.host().textContent, /Balance ¥10.00/);
  old.card.hidden = true;
  const next = composer('grok-4.6', false, 500);
  h.body.appendChild(next.root);
  assert.equal(h.evaluate().selectedModel, 'grok-4.6');
  assert.match(h.host().textContent, /Build 90%/);
  assert.doesNotMatch(h.host().textContent, /Balance/);
  assert.equal(h.host().style.top, '512px');
  assert.deepEqual(h.root().layoutObserver.targets, [next.footer, next.permissions, next.trigger]);
});

test('only a visible picker dropdown can supply a model when the current trigger is a placeholder', () => {
  const active = composer('Select effort');
  active.trigger.setAttribute('aria-expanded', 'true');
  const h = harness('injection_script', [active]);
  h.body.appendChild(dropdown('deepseek-v4-pro', true));
  h.body.appendChild(dropdown('grok-4.6'));
  assert.equal(h.evaluate().selectedModel, 'grok-4.6');
  assert.match(h.host().textContent, /Build 90%/);
});

test('multiple visible composers follow focus and preserve that owner while the usage dialog is focused', () => {
  const main = composer('gpt-6-sol');
  const side = composer('gpt-6-astra', false, 500, secondThread);
  const h = harness('injection_script', [main, side]);
  h.evaluate();
  side.editor.focus();
  h.dispatch('focusin');
  assert.equal(h.root().threadId, secondThread);
  assert.equal(h.host().style.top, '512px');
  const result = h.evaluate();
  assert.equal(result.selectedModel, 'gpt-6-astra');
  h.host().click();
  assert.equal(h.root().threadId, secondThread);
  assert.equal(h.root().quotaDetailsOpen, true);
  main.editor.focus();
  h.dispatch('focusin');
  assert.equal(h.root().threadId, firstThread);
  assert.equal(h.root().quotaDetailsOpen, false);
});

test('standalone DeepSeek balance follows the active composer without touching native controls', () => {
  const main = composer('deepseek-v4-pro');
  const side = composer('deepseek-v4-pro', false, 500, secondThread);
  const h = harness('deepseek_balance_injection_script', [main, side]);
  h.evaluate();
  const original = side.card.innerHTML;
  side.editor.focus();
  h.dispatch('focusin');
  assert.equal(h.root().card, side.card);
  assert.equal(h.host().style.top, '512px');
  assert.equal(side.card.innerHTML, original);
  main.editor.focus();
  h.dispatch('focusin');
  assert.equal(h.root().card, main.card);
  assert.equal(h.host().style.top, '912px');
});

test('an open native portal selects its composer and cannot leak model or speed into a focused sibling', () => {
  const main = composer('Select effort');
  const side = composer('6 Astra', false, 500, secondThread);
  const h = harness('injection_script', [main, side]);
  side.trigger.setAttribute('data-state', 'open');
  const menu = dropdown('6 Astra');
  const fast = new Element('span');
  fast.setAttribute('data-fast-mode-enabled', 'true');
  const ultra = new Element('svg');
  ultra.setAttribute('class', '_UltrafastIcon_5d7qv_2');
  fast.appendChild(ultra);
  menu.appendChild(fast);
  h.body.appendChild(menu);
  const usage = sessionUsage(h.now());
  usage.__binding.model = 'gpt-6-astra';
  menu.focus();
  let result = h.evaluate({ usage_state: usage });
  assert.equal(result.threadId, secondThread);
  assert.equal(result.usageSelection.model, 'gpt-6-astra');
  assert.equal(result.usageSelection.serviceTier, 'ultrafast');
  main.editor.focus();
  result = h.evaluate({ usage_state: usage });
  assert.equal(result.threadId, firstThread);
  assert.equal(result.selectedModel, null);
  assert.equal(result.usageSelection.serviceTier, 'default');
});

test('ambiguous native menus require the current open trigger aria-controls identity', () => {
  const active = composer('Select effort');
  active.trigger.setAttribute('aria-expanded', 'true');
  const h = harness('injection_script', [active]);
  const first = dropdown('grok-4.6');
  first.setAttribute('id', 'other-menu');
  const current = dropdown('6 Astra');
  current.setAttribute('id', 'current-menu');
  h.body.appendChild(first);
  h.body.appendChild(current);
  assert.equal(h.evaluate().selectedModel, null);
  active.trigger.setAttribute('aria-controls', 'current-menu');
  assert.equal(h.evaluate().selectedModel, '6 Astra');
  active.trigger.setAttribute('aria-expanded', 'false');
  assert.equal(h.evaluate().selectedModel, null);
});

test('current Astra three-tier menu reports Standard, Fast and Ultrafast without changing session means', () => {
  const active = composer('6 Astra');
  active.trigger.setAttribute('aria-expanded', 'true');
  const h = harness('injection_script', [active]);
  const menu = dropdown('6 Astra');
  const toggle = new Element('span');
  toggle.setAttribute('class', '_FastModeToggleContent_5d7qv_2');
  menu.appendChild(toggle);
  h.body.appendChild(menu);
  const usage = sessionUsage(h.now());
  usage.__binding.model = 'gpt-6-astra';
  usage.__binding.serviceTier = 'priority';
  for (const tier of ['ultrafast', 'priority', null]) {
    toggle.textContent = '';
    toggle.setAttribute('data-fast-mode-enabled', String(tier !== null));
    if (tier === 'ultrafast') {
      const icon = new Element('svg');
      icon.setAttribute('class', '_UltrafastIcon_5d7qv_2');
      toggle.appendChild(icon);
    }
    const result = h.evaluate({ usage_state: usage });
    assert.equal(result.usageSelection.model, 'gpt-6-astra');
    assert.equal(result.usageSelection.serviceTier, tier);
    assert.equal(metric(h.host(), 'speed').textContent, '↗50.0 tok/s');
    assert.equal(metric(h.host(), 'cache').textContent, 'Cache 80.0%');
  }
});

test('an explicit empty composer conversation ID never inherits the selected sidebar conversation', () => {
  const active = composer('6 Astra');
  active.conversation.setAttribute('data-above-composer-conversation-id', '');
  const h = harness('injection_script', [active]);
  const sidebar = new Element();
  sidebar.setAttribute('data-app-action-sidebar-thread-selected', 'true');
  sidebar.setAttribute('data-app-action-sidebar-thread-id', secondThread);
  h.body.appendChild(sidebar);
  const result = h.evaluate();
  assert.equal(result.threadId, null);
  active.conversation.removeAttribute('data-above-composer-conversation-id');
  active.conversation.setAttribute('data-above-composer-portal', 'true');
  assert.equal(h.evaluate().threadId, null, 'native pending composer omits its unresolved ID but keeps the portal');
});

test('a reused composer with an unresolved new conversation clears the prior model and metrics', () => {
  const active = composer('deepseek-v4-pro');
  const h = harness('injection_script', [active]);
  h.evaluate({ usage_state: sessionUsage(h.now()) });
  assert.match(h.host().textContent, /Balance/);
  active.trigger.textContent = 'Select effort';
  active.conversation.setAttribute('data-above-composer-conversation-id', 'local:client-new-thread:pending');
  const result = h.evaluate({ usage_state: sessionUsage(h.now()) });
  assert.equal(result.threadId, null);
  assert.equal(result.selectedModel, null);
  assert.equal(h.root().visibleModel, null);
  assert.doesNotMatch(h.host().textContent, /Balance|50\.0/);
});

test('placeholder text retains the same composer model briefly, then returns to the host fallback', () => {
  const active = composer('deepseek-v4-pro');
  const h = harness('injection_script', [active]);
  h.evaluate();
  active.trigger.textContent = 'Select effort';
  h.body.appendChild(dropdown('grok-4.6', true));
  assert.equal(h.evaluate().selectedModel, null);
  assert.match(h.host().textContent, /Balance ¥10.00/);
  h.advance(15001);
  assert.equal(h.evaluate().selectedModel, null);
  assert.match(h.host().textContent, /5h 75%/);
});

test('a new composer with placeholder text does not reuse the previous composer model cache', () => {
  const old = composer('deepseek-v4-pro');
  const h = harness('injection_script', [old]);
  h.evaluate();
  old.card.hidden = true;
  h.body.appendChild(composer('Select effort', false, 500).root);
  assert.equal(h.evaluate().selectedModel, null);
  assert.match(h.host().textContent, /5h 75%/);
  assert.doesNotMatch(h.host().textContent, /Balance/);
});

for (const kind of ['injection_script', 'deepseek_balance_injection_script'] as const) {
  test(`${kind}: ignore hidden reasoning anchors and hide when no composer is visible`, () => {
    const active = composer('gpt-5.4');
    const retained = new Element();
    retained.setAttribute('data-composer-navigation-target', 'reasoning');
    retained.hidden = true;
    retained.textContent = 'deepseek-v4-pro';
    active.footer.children.unshift(retained);
    retained.parentElement = active.footer;
    const h = harness(kind, [composer('deepseek-v4-pro', true), active]);
    h.evaluate();
    assert.equal(h.host().style.left, '515px');
    assert.equal(h.host().style.top, '912px');
    active.card.hidden = true;
    h.root().render();
    assert.equal(h.host().style.display, 'none');
    if (kind === 'injection_script') assert.equal(h.root().visibleModelFooter, null);
  });

  test(`${kind}: separate windows keep their own composer and state`, () => {
    const first = composer('gpt-5.4', false, 500);
    const second = composer('deepseek-v4-pro');
    const a = harness(kind, [composer('deepseek-v4-pro', true), first]);
    const b = harness(kind, [composer('gpt-5.4', true), second]);
    a.evaluate();
    b.evaluate();
    assert.equal(a.host().style.top, '512px');
    assert.equal(b.host().style.top, '912px');
    assert.notEqual(a.root(), b.root());
    if (kind === 'injection_script') {
      assert.match(a.host().textContent, /5h 75%/);
      assert.match(b.host().textContent, /Balance ¥10.00/);
    }
  });
}


test('composer account status keeps uppercase plan rows and distinguishes official access without a quota', () => {
  const previous = { plans: values.plans, account_count_value: values.account_count_value };
  values.plans = [{ plan: 'TEAM', count: 2, fiveHourRemainingPercent: 200, weeklyRemainingPercent: 200 }, { plan: 'PLUS', official: true, count: 1 }];
  values.account_count_value = 3;
  try {
    const h = harness('injection_script', [composer('gpt-6-luna')]);
    h.evaluate();
    assert.doesNotMatch(h.host().textContent, /Accounts 3/);
    h.root().quotaDetailsOpen = true; h.root().render();
    const details = h.body.querySelector('[data-cockpit-quota-details]')!;
    assert.match(details.innerHTML, /TEAM/);
    assert.match(details.textContent, /PLUS官/);
    assert.match(details.textContent, /TEAM2/);
    assert.match(details.textContent, /PLUS官1——/);
    assert.doesNotMatch(details.innerHTML, /API_KEY/);
  } finally { Object.assign(values, previous); }
});

function sessionUsage(now: number, threadId = firstThread): Record<string, any> {
  return {
    __binding:{binding:'fixture-service-key-instance',model:'gpt-6-sol',serviceTier:'default'},
    [threadId]: {
      threadId, currentTurnId: 'active-turn',
      outputSpeed: { threadId,binding:'fixture-service-key-instance', tokensPerSecond:50, samples:2, observedAtMs:now - 10000,avgCacheHitRate:.8,totalInputTokens:1000,totalCachedTokens:800,
        latestTokensPerSecond:50,latestObservedAtMs:now - 10000,requestId:'latest-'+threadId,model:'gpt-6-sol',
        selection:{model:'gpt-6-sol',serviceTier:'default'},latestInputTokens:1000,latestCachedTokens:800 },
      events: [
        { t: new Date(now - 20000).toISOString(), turn_id: 'active-turn', turn_output_tokens: 100, input_tokens: 100, cached_input_tokens: 80 },
        { t: new Date(now - 10000).toISOString(), turn_id: 'active-turn', turn_output_tokens: 600, input_tokens: 900, cached_input_tokens: 720 },
      ],
    },
  };
}

test('one native button presents a shared status strip with distinct labels, values and speed unit', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  h.evaluate({ usage_state: sessionUsage(h.now()) });
  assert.equal(h.host().tagName, 'button');
  assert.equal(h.host().getAttribute('type'), 'button');
  assert.equal(h.host().getAttribute('aria-haspopup'), 'dialog');
  assert.equal(h.host().querySelectorAll('button').length, 0);
  const metrics = h.host().querySelectorAll('[data-cockpit-quota-metric]');
  assert.deepEqual(metrics.map(metric => metric.textContent), ['5h 75%', 'Week 50%', '↗50.0 tok/s', 'Cache 80.0%']);
  assert.ok(metrics.every(metric => metric.tagName === 'span' && metric.getAttribute('title')));
  assert.deepEqual(metrics.map(metric => metric.querySelector('[data-cockpit-metric-value]')?.textContent), ['75%', '50%', '50.0', '80.0%']);
  assert.deepEqual(metrics.map(metric => metric.querySelector('[data-cockpit-metric-label]')?.textContent.trim() || null), ['5h', 'Week', null, 'Cache']);
  const speed = metrics[2];
  assert.equal(speed.querySelector('[data-cockpit-metric-unit]')!.textContent.trim(), 'tok/s');
  assert.equal(speed.querySelector('[data-cockpit-metric-icon]')!.getAttribute('aria-hidden'), 'true');
  assert.equal(speed.getAttribute('aria-label'), '50.0 tok/s');
  assert.equal(h.host().querySelectorAll('[data-cockpit-quota-refresh]').length, 0);
  assert.doesNotMatch(h.host().textContent, /Accounts|·/);
  const css = h.head.querySelector('[data-cockpit-quota-style]')!.textContent;
  assert.match(css, /\[data-cockpit-quota-footer\]\{[^}]*background:[^;}]+;[^}]*border:1px solid/);
  assert.match(css, /\[data-cockpit-quota-footer\]\{[^}]*gap:0;[^}]*height:28px;[^}]*padding:0 6px/);
  assert.match(css, /\[data-cockpit-quota-metric\]\{[^}]*border:0/);
  assert.match(css, /\[data-cockpit-quota-metric\]\{[^}]*background:transparent/);
  assert.match(css, /\[data-cockpit-quota-metric\]\{[^}]*gap:4px;[^}]*padding:0 5px;[^}]*color:var\(--color-token-text-primary,inherit\)/);
  assert.match(css, /\[data-cockpit-metric-label\]\{[^}]*font-size:11px;[^}]*color:var\(--color-token-text-secondary/);
  assert.match(css, /\[data-cockpit-metric-value\]\{font-size:12px;[^}]*font-weight:500/);
  assert.match(css, /\[data-cockpit-metric-unit\]\{[^}]*font-size:10px;[^}]*color:var\(--color-token-text-secondary/);
  assert.match(css, /\[data-cockpit-metric-key="speed"\]\{[^}]*border-inline-start:1px solid/);
  assert.match(css, /\[data-cockpit-metric-key="speed"\] \[data-cockpit-metric-value\],[^{]+\{color:#3f729f;/);
  assert.match(css, /\[data-cockpit-quota-tone="medium"\] \[data-cockpit-metric-value\]\{color:/);
  assert.match(css, /\[data-cockpit-quota-tone="critical"\] \[data-cockpit-metric-value\]\{color:/);
  assert.doesNotMatch(css, /\[data-cockpit-quota-tone=[^}]+background:/);
  assert.doesNotMatch(css, /data-cockpit-progress|cockpit-quota-fill/);
  const stripRules = [...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)].filter(match => match[1].includes('[data-cockpit-quota-footer]'));
  assert.ok(stripRules.length > 0);
  for (const rule of stripRules) assert.doesNotMatch(rule[2], /linear-gradient/);
  assert.equal(h.details().style.display, 'none');
});

test('split metric content escapes labels and retains literal translated text', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  const label = '<img src=x onerror=alert(1)>&"';
  h.evaluate({ five_hour_label: label });
  const first = h.host().querySelector('[data-cockpit-metric-key="five-hour"]')!;
  assert.equal(first.querySelector('[data-cockpit-metric-label]')!.textContent.trim(), label);
  assert.equal(first.querySelector('[data-cockpit-metric-value]')!.textContent, '75%');
  assert.equal(h.host().querySelectorAll('img, svg').length, 0);
});

test('the complete status strip supports native keyboard activation and clicks on any metric', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  h.evaluate({ usage_state: sessionUsage(h.now()) });
  assert.equal(h.host().tagName, 'button');
  for (const key of ['Enter', ' ']) {
    h.host().focus();
    h.host().press(key);
    assert.equal(h.details().style.display, 'flex');
    h.dispatch('keydown', { key: 'Escape' });
    assert.equal(h.details().style.display, 'none');
    assert.equal(h.host().focused, true);
  }
  for (const node of h.host().querySelectorAll('[data-cockpit-quota-metric], [data-cockpit-metric-label], [data-cockpit-metric-value], [data-cockpit-metric-unit], [data-cockpit-metric-icon]')) {
    node.click();
    assert.equal(h.details().style.display, 'flex');
    h.details().querySelector('[data-cockpit-quota-close]')!.click();
  }
});

test('native dictation and recording controls take precedence over the API status strip', () => {
  const active = composer('gpt-6-sol');
  const h = harness('injection_script', [active]);
  h.evaluate({ usage_state: sessionUsage(h.now()) });
  for (const mode of ['waveform', 'text']) {
    active.footer.setAttribute('data-dictation-view', mode);
    h.root().render();
    assert.equal(h.host().style.display, 'none');
  }
  delete active.footer.attributes['data-dictation-view'];
  h.root().render();
  assert.equal(h.host().style.display, 'flex');
  const recorder = new Element('canvas');
  recorder.rect = { left: 188, top: 900, width: 650, height: 24 };
  active.footer.appendChild(recorder);
  h.root().render();
  assert.equal(h.host().style.display, 'none');
  recorder.remove();
  h.root().render();
  assert.equal(h.host().style.display, 'flex');
});

type DialogHarness = ReturnType<typeof harness>;
function dialogAction(h: DialogHarness, action: string, value?: string) {
  const button = h.details().querySelector(`[data-cockpit-dialog-action="${action}"]${value ? `[data-value="${value}"]` : ''}`);
  assert.ok(button, `missing ${action}:${value ?? ''} control`);
  button.click();
  return button;
}
function dialogQuery(h: DialogHarness) {
  const query = h.root().dialogState?.pending;
  assert.ok(query?.requestToken, 'open dialog should request real host data');
  return { ...query };
}
function dialogResponse(h: DialogHarness, payload: Record<string, unknown>, query = dialogQuery(h)) {
  h.root().usageState = { ...h.root().usageState, __dialog: {
    ...h.root().usageState?.__dialog, [query.requestToken]: { ...query, available: true, ...payload },
  } };
  h.root().render();
}
function requestRecord(id: string, overrides: Record<string, unknown> = {}) {
  return {
    id, timestamp: Date.UTC(2026, 9, 21, 0, 16, 46), accountId: 'account-' + id, accountEmail: id + '@example.com',
    plan: 'TEAM_CUSTOM', model: 'gpt-real-model', reasoningEffort: 'high', transport: 'official', success: true,
    inputTokens: 1234, outputTokens: 250, cachedTokens: 1000, totalTokens: 1484,
    firstResponseMs: 1200, latencyMs: 6200, outputGenerationMs: 5000, httpStatus: 200, ...overrides,
  };
}
function openRequests(h: DialogHarness) {
  h.host().click();
  dialogAction(h, 'tab', 'logs');
}

test('request account identity comes from the current chat page and preserves its raw plan', () => {
  const active = composer('gpt-6-sol');
  const h = harness('injection_script', [active]);
  const usage = sessionUsage(h.now());
  h.evaluate({ usage_state: { ...usage, [firstThread]: { ...usage[firstThread],
    recentAccount: { displayName: 'obsolete@example.com', plan: 'OLD_PLAN', lastUsedAtMs: h.now() } } } });
  openRequests(h);
  const query = dialogQuery(h);
  assert.equal(query.threadId, firstThread);
  assert.equal(query.scope, 'chat');
  dialogResponse(h, { requests: { rows: [requestRecord('alice')], nextCursor: null } });
  assert.match(h.details().textContent, /alice@example\.comTEAM_CUSTOM/);
  assert.doesNotMatch(h.details().textContent, /obsolete@example\.com|OLD_PLAN/);
  assert.doesNotMatch(h.host().textContent, /alice@example\.com|TEAM_CUSTOM/);
  active.conversation.setAttribute('data-above-composer-conversation-id', secondThread);
  h.root().render();
  assert.equal(h.details().style.display, 'none');
  openRequests(h);
  assert.equal(dialogQuery(h).threadId, secondThread);
  dialogResponse(h, { requests: { rows: [requestRecord('alice')], nextCursor: null } }, query);
  assert.doesNotMatch(h.details().textContent, /alice@example\.com|TEAM_CUSTOM/);
});

test('unlinked current chat metrics stay unknown and request records can still use service scope', () => {
  const active = composer('gpt-6-sol', false, 900, '');
  active.conversation.removeAttribute('data-above-composer-conversation-id');
  const h = harness('injection_script', [active]);
  h.evaluate({ usage_state: sessionUsage(h.now(), secondThread) });
  h.host().click();
  assert.match(h.details().querySelector('.session-metrics')!.textContent, /Average output speed—.*Average cache hits—/);
  dialogAction(h, 'tab', 'logs');
  assert.equal(dialogQuery(h).threadId, null);
  dialogResponse(h, { requests: { rows: [], nextCursor: null } });
  assert.match(h.details().textContent, /No requests are linked to this chat yet/);
  dialogAction(h, 'scope', 'service');
  assert.equal(dialogQuery(h).scope, 'service');
  dialogResponse(h, { requests: { rows: [requestRecord('service')], nextCursor: null } });
  assert.match(h.details().textContent, /service@example\.com/);
});

test('request account, plan, model and error strings render as text rather than executable markup', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  h.evaluate();
  openRequests(h);
  const accountEmail = '<img src=x onerror=alert(1)>alice&"';
  const plan = '<svg onload=alert(1)>CUSTOM';
  const model = '<script>alert(1)</script>';
  const errorCategory = '<iframe src=x>';
  const errorMessage = '<svg onload=alert(2)>"';
  dialogResponse(h, { requests: { rows: [requestRecord('unsafe', { accountEmail, plan, model, errorCategory, errorMessage, success: false, httpStatus: 429 })], nextCursor: null } });
  assert.equal(h.details().querySelectorAll('img, svg, script, iframe').length, 0);
  for (const text of [accountEmail, plan, model, errorCategory]) assert.ok(h.details().textContent.includes(text));
  assert.equal(h.details().querySelector('.cell-email')!.getAttribute('title'), accountEmail);
  assert.equal(h.details().querySelector('.cell-error')!.getAttribute('title'), errorMessage);
  assert.match(h.details().innerHTML, /&lt;img/);
});

test('overview uses real quotas, current chat metrics and service totals without a duplicated recent request', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  h.evaluate({ usage_state: sessionUsage(h.now()), plans: [{ plan: 'lowercase-custom', count: 3, fiveHourRemainingPercent: 143, weeklyRemainingPercent: 211 }] });
  assert.equal(h.root().dialogState?.pending ?? null, null, 'closed dialog must not start its data query');
  h.host().click();
  assert.match(h.details().textContent, /Loading/);
  assert.equal(dialogQuery(h).view, 'overview');
  dialogResponse(h, {
    session: { threadId: firstThread, model: 'verified-current-model', reasoningEffort: 'xhigh', firstResponseMs: 900 },
    summary: { requests: 6, successRequests: 5, totalTokens: 4567, estimatedCostUsd: 1.28, averageLatencyMs: 1500,
      sinceMs: Date.UTC(2026, 9, 20), untilMs: Date.UTC(2026, 9, 22) },
    requests: { rows: [requestRecord('not-an-overview-card')], nextCursor: null },
  });
  assert.equal(h.details().querySelectorAll('.business-section').length, 3);
  assert.match(h.details().querySelector('.quota-block')!.textContent, /lowercase-custom3143%211%/);
  assert.match(h.details().querySelector('.session-metrics')!.textContent, /verified-current-modelxhigh.*50\.0\s?tok\/s.*80\.0%.*900\s?ms/);
  assert.match(h.details().querySelector('.stats-grid')!.textContent, /Requests6Success rate 83%Total tokens4\.6KEst\. value\$1\.28Avg\. latency1\.50\s?s/);
  assert.equal(h.details().querySelectorAll('.request-row').length, 0);
  assert.doesNotMatch(h.details().textContent, /not-an-overview-card|Recent account|Loading/);
  assert.equal(h.root().dialogState.summary.requests, 6);
});

test('the shipped wrapper reports a dialog request only while open and ingests the host response', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  const usage = sessionUsage(h.now());
  assert.equal(h.evaluate({ usage_state: usage }).dialogRequest, null);
  h.host().click();
  const requested = h.evaluate({ usage_state: usage }).dialogRequest;
  assert.equal(requested.threadId, firstThread);
  assert.equal(requested.view, 'overview');
  h.evaluate({ usage_state: { ...usage, __dialog: { [requested.requestToken]: { ...requested, available: true,
    session: { threadId: firstThread, model: 'model-from-host', firstResponseMs: 1500 },
    summary: { requests: 29 } } } } });
  assert.match(h.details().textContent, /model-from-host.*1\.50\s?s.*Requests29/);
  h.details().querySelector('[data-cockpit-quota-close]')!.click();
  assert.equal(h.evaluate().dialogRequest, null);
});

test('quota details follow the selected model without mixing GPT, DeepSeek balances and Grok quotas', () => {
  const active = composer('gpt-6-sol');
  const h = harness('injection_script', [active]);
  const quota = {
    plans: [{plan:'API_KEY',count:7}, {plan:'ChatGPTTeamPlan',count:1,fiveHourRemainingPercent:97,weeklyRemainingPercent:100}],
    balance_lines: [{totalBalance:91.36,currency:'CNY',accountCount:2}, {totalBalance:12.50,currency:'USD',accountCount:1}],
    grok_lines_value: [{product:'GrokBuild',count:2,remainingPercent:75}],
    usage_state: {__accountPool:{binding:'fixture-scope',gptApiKeyCount:1}},
  };
  h.evaluate(quota);
  h.host().click();
  dialogResponse(h,{gptApiKeyCount:1});
  let block = h.details().querySelector('.quota-block')!;
  assert.equal(block.getAttribute('data-resource-kind'),'codex');
  assert.match(block.textContent,/API1.*TEAM197%100%/);
  assert.doesNotMatch(block.textContent,/91\.36|12\.50|Grok|Build/);
  assert.ok(block.querySelectorAll('.badge').some(node => node.textContent === 'TEAM' && node.getAttribute('title') === 'ChatGPTTeamPlan' && node.getAttribute('class')!.split(' ').includes('team')), 'ChatGPTTeamPlan uses the API Service TEAM badge and retains the raw tooltip');
  active.trigger.textContent = 'deepseek-flash';
  h.root().render();
  block = h.details().querySelector('.quota-block')!;
  assert.equal(block.getAttribute('data-resource-kind'),'deepseek');
  assert.match(block.textContent,/ProviderAccountsBalanceDeepSeek · CNY2¥91\.36.*DeepSeek · USD1\$12\.50/);
  assert.doesNotMatch(block.textContent,/API_KEY|ChatGPTTeamPlan|Build|97%/);
  active.trigger.textContent = 'grok-4.6';
  h.root().render();
  block = h.details().querySelector('.quota-block')!;
  assert.equal(block.getAttribute('data-resource-kind'),'grok');
  assert.match(block.textContent,/ProviderAccountsQuotaGrok · Build275%/);
  assert.doesNotMatch(block.textContent,/DeepSeek|91\.36|API_KEY|ChatGPTTeamPlan|97%/);
  active.trigger.textContent = 'gpt-6-sol';
  h.root().render();
  assert.equal(h.details().querySelector('.quota-block')!.getAttribute('data-resource-kind'),'codex');
});

test('GPT API key counts never inherit the unfiltered provider total', () => {
  for (const count of [undefined,null,0,2]) {
    const h = harness('injection_script',[composer('gpt-6-sol')]);
    h.evaluate({plans:[{plan:'API_KEY',count:17},{plan:'plus',count:1}],
      usage_state:{__accountPool:{binding:'fixture-scope',gptApiKeyCount:count}}});
    h.host().click();
    dialogResponse(h,{gptApiKeyCount:count});
    const text = h.details().querySelector('.quota-block')!.textContent;
    assert.doesNotMatch(text,/17/);
    if (count === 0) assert.doesNotMatch(text,/API/);
    else assert.match(text,count === 2 ? /API2/ : /API—/);
  }
});

test('API key counts are independent of statistics, chat changes and host disconnection', () => {
  const active = composer('gpt-6-sol');
  const h = harness('injection_script',[active]);
  h.evaluate({plans:[{plan:'API_KEY',count:17}],
    usage_state:{__accountPool:{binding:'scope-a',gptApiKeyCount:2}}});
  h.host().click();
  assert.match(h.details().querySelector('.quota-block')!.textContent,/API2——/);
  dialogResponse(h,{available:false,gptApiKeyCount:999,summary:null});
  assert.match(h.details().querySelector('.quota-block')!.textContent,/API2——/);
  active.conversation.setAttribute('data-above-composer-conversation-id',secondThread);
  h.root().render();
  h.host().click();
  assert.match(h.details().querySelector('.quota-block')!.textContent,/API2——/);
  h.advance(9000);
  h.intervals.forEach(tick => tick());
  assert.equal(h.root().hostAvailable,false);
  assert.match(h.details().querySelector('.quota-block')!.textContent,/API2——/);
});

test('pool counts retain a confirmed value while reconnecting only within the same service and key', () => {
  const h = harness('injection_script',[composer('gpt-6-sol')]);
  const show = (binding:string,count:number|null,pending=false) => h.evaluate({
    plans:[{plan:'API_KEY',count:17}],
    usage_state:{__accountPool:{binding,gptApiKeyCount:count,pending}},
  });
  show('scope-a',2); h.host().click();
  show('scope-a',null,true);
  assert.match(h.details().querySelector('.quota-block')!.textContent,/API2/);
  show('scope-b',null,true);
  assert.match(h.details().querySelector('.quota-block')!.textContent,/API—/);
  show('scope-b',3);
  assert.match(h.details().querySelector('.quota-block')!.textContent,/API3/);
  show('scope-b',null,false);
  assert.match(h.details().querySelector('.quota-block')!.textContent,/API—/);
  show('scope-b',0);
  assert.doesNotMatch(h.details().querySelector('.quota-block')!.textContent,/API/);
});

test('balance rows align provider, real contributor count and currency amount without a half-width card', () => {
  const h = harness('injection_script',[composer('deepseek-flash')]);
  const currency = '<img src=x onerror=alert(1)>';
  h.evaluate({balance_lines:[
    {currency:'CNY',accountCount:2,totalBalance:89.77},
    {currency:'USD',accountCount:1,totalBalance:0},
    {currency,totalBalance:1},
  ]});
  h.host().click();
  const table = h.details().querySelector('.resource-table')!;
  assert.deepEqual(table.querySelectorAll('th').map(node => node.textContent),['Provider','Accounts','Balance']);
  assert.deepEqual(table.querySelector('tbody')!.querySelectorAll('tr').map(row => row.querySelectorAll('td').map(node => node.textContent)),[
    ['DeepSeek · CNY','2','¥89.77'],['DeepSeek · USD','1','$0.00'],['DeepSeek · ' + currency,'—','1.00 ' + currency],
  ]);
  assert.equal(h.details().querySelectorAll('.resource-metric, img').length,0);
  const css = h.head.querySelector('[data-cockpit-quota-style]')!.textContent;
  assert.match(css,/\.resource-table :is\(th,td\):first-child\{[^}]*text-align:start/);
  assert.match(css,/\.resource-table :is\(th,td\):nth-child\(2\)\{[^}]*text-align:center/);
  assert.match(css,/\.resource-table :is\(th,td\):last-child\{[^}]*text-align:end/);
});

test('overview collapses identical model names, separates metric units and preserves request model details', () => {
  const h = harness('injection_script',[composer('gpt-6-sol')]);
  h.evaluate({usage_state:sessionUsage(h.now())});
  h.host().click();
  const session = {threadId:firstThread,model:'gpt-6-sol',requestedModel:'gpt-6-sol',upstreamModel:'gpt-6-sol',reasoningEffort:'high',firstResponseMs:7640};
  dialogResponse(h,{session,summary:{requests:1,averageLatencyMs:11040}});
  let block = h.details().querySelector('.session-block')!;
  assert.equal(block.querySelector('.subtle'),null);
  assert.deepEqual(block.querySelector('.model')!.querySelectorAll('dd').map(node => node.textContent),['gpt-6-sol','high']);
  assert.deepEqual(block.querySelectorAll('.metric-unit').map(node => node.textContent),[' tok/s','%',' s']);
  assert.equal(h.details().querySelector('.stats-grid')!.querySelector('.metric-unit')!.textContent,' s');
  // A fresh context response with a genuinely different upstream remains visible.
  dialogAction(h,'period','today');
  dialogResponse(h,{session:{...session,upstreamModel:'gpt-upstream'}});
  block = h.details().querySelector('.session-block')!;
  assert.deepEqual(block.querySelector('.model')!.querySelectorAll('dd').map(node => node.textContent),['gpt-6-sol','↳ gpt-upstream','high']);
  dialogAction(h,'tab','logs');
  dialogResponse(h,{requests:{rows:[requestRecord('same-model',{requestedModel:'gpt-6-sol',upstreamModel:'gpt-6-sol'})],nextCursor:null}});
  assert.equal(h.details().querySelector('.cell-upstream')!.textContent,'↳ gpt-6-sol');
});

test('request account names match API service labels and never fall back to internal identifiers', () => {
  const h = harness('injection_script',[composer('gpt-6-sol')]);
  h.evaluate();
  openRequests(h);
  dialogResponse(h,{requests:{rows:[
    requestRecord('provider',{accountId:'codex_0c881bdd4e810dcfd5edfae96bb470bc',accountEmail:null,accountDisplayName:'DeepSeek'}),
    requestRecord('oauth',{accountId:'codex_internal',accountDisplayName:'724550799@qq.com'}),
    requestRecord('deleted',{accountId:'codex_deleted',accountEmail:null,accountDisplayName:null}),
    requestRecord('generated',{accountId:'codex_gone',accountEmail:'api-key-1234abcd'}),
  ],nextCursor:null}});
  const names = h.details().querySelectorAll('.cell-email').map(node => node.textContent);
  assert.deepEqual(names,['DeepSeek','724550799@qq.com','—','—']);
  assert.doesNotMatch(h.details().textContent,/codex_|api-key-1234abcd/);
});

test('historical totals with missing dispatch evidence are visibly identified as a verified subset', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  h.evaluate();
  h.host().click();
  dialogResponse(h, {
    summary: { requests: 12, successRequests: 9, totalTokens: 4500, estimatedCostUsd: 0.72,
      averageLatencyMs: 1200, incomplete: true, excludedUnknownRequests: 49 },
  });
  const heading = h.details().querySelector('.stats-block')!.querySelector('.section-title')!;
  assert.equal(heading.textContent, usageLabels.verifiedStatistics);
  assert.equal(heading.getAttribute('title'),
    usageLabels.excludedHistory.replace('{{count}}', '49'));
  assert.match(h.details().querySelector('.stats-grid')!.textContent, /Requests12Success rate 75%.*4\.5K/);
  dialogAction(h, 'period', 'today');
  dialogResponse(h, { summary: { requests: 2, incomplete: false, excludedUnknownRequests: 0 } });
  assert.equal(h.details().querySelector('.stats-block')!.querySelector('.section-title')!.getAttribute('title'), '');
});

test('known subscription aliases use API service categories while custom values retain their source', () => {
  const h = harness('injection_script',[composer('gpt-6-sol')]);
  h.evaluate({plans:['ChatGPTTeamPlan','ChatGPTPlusPlan','pro','enterprise','custom-tier','__proto__'].map(plan => ({plan,count:1}))});
  h.host().click();
  const badges = h.details().querySelectorAll('.badge');
  assert.deepEqual(badges.map(node => [node.textContent,node.getAttribute('title')]),[
    ['TEAM','ChatGPTTeamPlan'],['PLUS','ChatGPTPlusPlan'],['PRO','pro'],['ENTERPRISE','enterprise'],['custom-tier','custom-tier'],['__proto__','__proto__'],
  ]);
});

test('request model aliases, authorization route, zero timing and cancellation use API service evidence', () => {
  const h = harness('injection_script',[composer('gpt-6-sol')]);
  h.evaluate();
  openRequests(h);
  dialogResponse(h,{requests:{rows:[
    requestRecord('alias',{requestedModel:'gpt-client-alias',upstreamModel:'deepseek-flash',authorizationRoute:'official',transport:'http',firstResponseMs:0,latencyMs:0}),
    requestRecord('old-official',{accountId:'chatgpt_plan_legacy',transport:'sse'}),
    requestRecord('unknown-route',{accountId:null,accountEmail:null,transport:'http'}),
    requestRecord('canceled',{httpStatus:499,success:false,errorCategory:' CLIENT_CANCELLED '}),
  ],nextCursor:null}});
  const rows = h.details().querySelectorAll('.request-row');
  assert.equal(rows[0].querySelector('.cell-model')!.textContent,'gpt-client-alias');
  assert.equal(rows[0].querySelector('.cell-upstream')!.textContent,'↳ deepseek-flash');
  assert.equal(rows[0].querySelector('.cell-route')!.textContent,usageLabels.official);
  assert.match(rows[0].querySelectorAll('.cell-metrics')[1].textContent,/0ms.*0ms/);
  assert.equal(rows[1].querySelector('.cell-route')!.textContent,usageLabels.official);
  assert.equal(rows[2].querySelector('.cell-route')!.textContent,'—');
  assert.equal(rows[3].querySelector('.cell-result')!.textContent,usageLabels.cancelled);
});

test('compact dialog dimensions and readable theme fallbacks apply to data and labels', () => {
  const h = harness('injection_script',[composer('gpt-6-sol')]);
  h.evaluate();
  h.host().click();
  assert.equal(h.details().style.width,'660px');
  const css = h.head.querySelector('[data-cockpit-quota-style]')!.textContent;
  const contrastOnWhite = (hex:string) => {
    const rgb = hex.match(/\w\w/g)!.map(channel => Number.parseInt(channel,16) / 255)
      .map(value => value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4);
    return 1.05 / (rgb[0]*0.2126 + rgb[1]*0.7152 + rgb[2]*0.0722 + 0.05);
  };
  for (const name of ['text','muted']) {
    const fallback = css.match(new RegExp('--cockpit-' + name + ':var\\([^,]+,#([a-f0-9]{6})\\)'))![1];
    assert.ok(contrastOnWhite(fallback) >= 4.5, name + ' must meet readable contrast');
  }
  assert.match(css,/\.cell-metrics dd\{[^}]*color:var\(--cockpit-text\)/);
  assert.match(css,/\.cell-email[^}]*color:var\(--cockpit-text\)/);
  assert.match(css,/font-size:11px;line-height:1\.45/);
  assert.match(css,/max-height:min\(700px,/);
});

test('small nonzero estimated values retain API service precision instead of becoming zero', () => {
  for (const [value,expected] of [[0,'$0.00'],[0.0000001,'<$0.000001'],[0.000012,'$0.000012'],[0.0012,'$0.001200'],[0.12,'$0.1200'],[12.3,'$12.30']] as const) {
    const h = harness('injection_script',[composer('gpt-6-sol')]);
    h.evaluate();
    h.host().click();
    dialogResponse(h,{summary:{requests:1,estimatedCostUsd:value}});
    assert.equal(h.details().querySelector('.value')!.textContent,expected);
  }
});

test('a failed read shows retry feedback, retries finitely and still permits explicit retry', () => {
  const h = harness('injection_script',[composer('gpt-6-sol')]);
  h.evaluate();
  h.host().click();
  let failed = dialogQuery(h);
  dialogResponse(h,{error:true},failed);
  assert.match(h.details().textContent,/Could not load data/);
  for (const delay of [2000,5000,10000]) {
    h.advance(delay - 1);
    h.root().render();
    assert.equal(dialogQuery(h).requestToken,failed.requestToken);
    h.advance(1);
    h.root().render();
    const retry = dialogQuery(h);
    assert.notEqual(retry.requestToken,failed.requestToken);
    assert.doesNotMatch(h.details().textContent,/Could not load data/);
    dialogResponse(h,{error:true},retry);
    failed = retry;
  }
  h.advance(60000);
  h.root().render();
  assert.equal(dialogQuery(h).requestToken,failed.requestToken);
  assert.ok(h.details().querySelector('[data-cockpit-dialog-action="retry"]'));
  dialogAction(h,'retry');
  assert.notEqual(dialogQuery(h).requestToken,failed.requestToken);
  assert.doesNotMatch(h.details().textContent,/Could not load data/);
});

test('host heartbeat recovery refreshes failed dialog reads and rejects late former replies', () => {
  const h = harness('injection_script',[composer('gpt-6-sol')]);
  h.evaluate({usage_state:sessionUsage(h.now())});
  h.host().click();
  dialogResponse(h,{summary:{requests:37}});
  h.details().querySelector('[data-cockpit-quota-refresh]')!.click();
  const failed = dialogQuery(h);
  dialogResponse(h,{error:true},failed);
  h.advance(9000);
  h.intervals.forEach(tick => tick());
  assert.equal(h.root().hostAvailable,false);
  assert.match(h.details().textContent,/Host unavailable/);
  h.evaluate({usage_state:sessionUsage(h.now())});
  const retry = dialogQuery(h);
  assert.notEqual(retry.requestToken,failed.requestToken);
  assert.equal(h.root().dialogState.summary.requests,37);
  dialogResponse(h,{summary:{requests:999}},failed);
  assert.equal(h.root().dialogState.summary.requests,37);
  dialogResponse(h,{summary:{requests:38}},retry);
  assert.equal(h.root().dialogState.summary.requests,38);
});

test('request tabs support keyboard navigation with stable dialog size and retain verified rows during refresh on return', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  h.evaluate();
  h.host().click();
  const details = h.details();
  let prevented = false;
  const overview = details.querySelector('[data-cockpit-dialog-action="tab"][data-value="overview"]')!;
  overview.onkeydown!({ key: 'ArrowRight', preventDefault() { prevented = true; } });
  assert.equal(prevented, true);
  assert.equal(h.details(), details);
  const logs = details.querySelector('[data-cockpit-dialog-action="tab"][data-value="logs"]')!;
  assert.equal(logs.getAttribute('aria-selected'), 'true');
  assert.equal(logs.getAttribute('tabindex'), '0');
  assert.equal(logs.focused, true);
  dialogResponse(h, { requests: { rows: [requestRecord('paged-row')], nextCursor: 'next' } });
  h.details().querySelector('.request-scroll')!.scrollTop = 300;
  dialogAction(h, 'more');
  dialogResponse(h, { requests: { rows: [requestRecord('older-row')], nextCursor: 'next2' } });
  dialogAction(h, 'tab', 'overview');
  dialogAction(h, 'tab', 'logs');
  assert.equal(h.details(), details);
  assert.equal(dialogQuery(h).cursor, null);
  assert.equal(h.root().dialogState.paged, false);
  assert.equal(h.details().querySelectorAll('.request-row').length, 2);
  assert.equal(h.details().querySelector('.request-scroll')!.scrollTop, 0);
});

test('session information rejects another thread even in an otherwise matching host response', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  h.evaluate({ usage_state: sessionUsage(h.now()) });
  h.host().click();
  dialogResponse(h, { session: { threadId: secondThread, model: 'wrong-conversation-model', firstResponseMs: 45 } });
  const session = h.details().querySelector('.session-metrics')!;
  assert.doesNotMatch(session.textContent, /wrong-conversation-model|0\.05s/);
  assert.match(session.textContent, /Model—.*Average output speed50\.0\s?tok\/s.*Average cache hits80\.0%.*First response—/);
});

test('dialog rejects success and error responses with mismatched token, chat, scope, period, cursor or view', () => {
  for (const key of ['requestToken', 'threadId', 'scope', 'period', 'cursor', 'view']) {
    for (const error of [false, true]) {
      const h = harness('injection_script', [composer('gpt-6-sol')]);
      h.evaluate();
      openRequests(h);
      const q = dialogQuery(h);
      dialogResponse(h, { [key]: 'not-the-pending-' + key, error,
        requests: { rows: [requestRecord('wrong-context')], nextCursor: 'wrong-cursor' } }, q);
      assert.equal(h.root().dialogState.handled, null, `${key}, error=${error}`);
      assert.equal(h.root().dialogState.error, false, `${key}, error=${error}`);
      assert.doesNotMatch(h.details().textContent, /wrong-context|Could not load data/);
      dialogResponse(h, { requests: { rows: [requestRecord('correct')], nextCursor: null } }, q);
      assert.match(h.details().textContent, /correct@example\.com/);
    }
  }
});

test('scope and period switches replace pending requests and never accept a late former response', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  h.evaluate();
  openRequests(h);
  const chat = dialogQuery(h);
  dialogResponse(h, { requests: { rows: [requestRecord('chat')], nextCursor: 'chat-more' } });
  h.details().querySelector('.request-scroll')!.scrollTop = 320;
  dialogAction(h, 'scope', 'service');
  assert.equal(h.details().querySelector('.request-scroll')!.scrollTop, 0);
  const service = dialogQuery(h);
  assert.notEqual(service.requestToken, chat.requestToken);
  assert.equal(service.cursor, null);
  assert.doesNotMatch(h.details().textContent, /chat@example\.com/);
  dialogResponse(h, { requests: { rows: [requestRecord('late-chat')], nextCursor: null } }, chat);
  assert.doesNotMatch(h.details().textContent, /late-chat/);
  dialogResponse(h, { requests: { rows: [requestRecord('service')], nextCursor: null } }, service);
  assert.match(h.details().textContent, /service@example\.com/);
  dialogAction(h, 'tab', 'overview');
  const week = dialogQuery(h);
  dialogResponse(h, { summary: { requests: 345 } });
  assert.match(h.details().querySelector('.stats-grid')!.textContent, /Requests345/);
  dialogAction(h, 'period', 'today');
  const today = dialogQuery(h);
  assert.notEqual(today.requestToken, week.requestToken);
  assert.equal(today.period, 'today');
  assert.doesNotMatch(h.details().textContent, /345/);
  dialogResponse(h, { summary: { requests: 876 } }, week);
  assert.doesNotMatch(h.details().textContent, /876/);
  dialogResponse(h, { summary: { requests: 9 } }, today);
  assert.match(h.details().querySelector('.stats-grid')!.textContent, /Requests9/);
});

test('more appends pages, deduplicates request IDs and preserves list scroll and focus', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  h.evaluate();
  openRequests(h);
  dialogResponse(h, { requests: { rows: Array.from({ length: 10 }, (_, i) => requestRecord('r' + i)), nextCursor: 'opaque-page-2' } });
  assert.equal(h.details().querySelectorAll('.request-row').length, 10);
  h.details().querySelector('.request-scroll')!.scrollTop = 540;
  h.details().querySelector('[data-cockpit-dialog-action="more"]')!.focus();
  dialogAction(h, 'more');
  const page2 = dialogQuery(h);
  assert.equal(page2.cursor, 'opaque-page-2');
  assert.equal(h.details().querySelectorAll('.request-row').length, 10);
  assert.equal(h.details().querySelector('.request-scroll')!.scrollTop, 540);
  assert.equal(h.details().querySelector('.request-scroll')!.focused, true, 'disabled loading button transfers focus to its readable request region');
  dialogResponse(h, { requests: { rows: [requestRecord('r9'), requestRecord('r10'), requestRecord('r11')], nextCursor: 'opaque-page-3' } });
  assert.equal(h.details().querySelectorAll('.request-row').length, 12);
  assert.equal(h.details().querySelector('.request-scroll')!.scrollTop, 540);
  assert.equal(h.details().querySelector('.request-scroll')!.focused, true, 'completion keeps focus inside the same request region');
  assert.match(h.details().querySelector('.records-count')!.textContent, /12 shown/);
  h.advance(6000);
  h.root().render();
  assert.equal(dialogQuery(h).requestToken, page2.requestToken, 'polling must not replace paginated results');
  dialogAction(h, 'more');
  dialogResponse(h, { requests: { rows: [requestRecord('r12')], nextCursor: null } });
  assert.equal(h.details().querySelectorAll('.request-row').length, 13);
  assert.equal(h.details().querySelector('[data-cockpit-dialog-action="more"]')!.disabled, true);
  assert.match(h.details().textContent, /All loaded/);
});

test('pagination failures retain existing records, expose one in-dialog error and retry the same cursor', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  h.evaluate();
  openRequests(h);
  dialogResponse(h, { requests: { rows: [requestRecord('retained')], nextCursor: 'retry-page' } });
  dialogAction(h, 'more');
  const failed = dialogQuery(h);
  dialogResponse(h, { error: true });
  assert.equal(h.details().style.display, 'flex');
  assert.match(h.details().textContent, /retained@example\.com/);
  assert.equal(h.details().querySelectorAll('[data-cockpit-quota-error]').length, 1);
  assert.match(h.details().querySelector('[data-cockpit-quota-error]')!.textContent, /Could not load data/);
  assert.equal(h.host().querySelector('[data-cockpit-quota-error]'), null);
  dialogAction(h, 'retry');
  const retry = dialogQuery(h);
  assert.equal(retry.cursor, failed.cursor);
  assert.notEqual(retry.requestToken, failed.requestToken);
  assert.equal(h.details().querySelector('[data-cockpit-quota-error]'), null);
  assert.match(h.details().textContent, /Loading/);
  dialogResponse(h, { requests: { rows: [requestRecord('after-retry')], nextCursor: null } });
  assert.equal(h.details().querySelectorAll('.request-row').length, 2);
  assert.match(h.details().textContent, /retained@example\.com.*after-retry@example\.com/);
});

test('only the first request page refreshes automatically and a manual refresh replaces a paginated snapshot', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  h.evaluate();
  openRequests(h);
  dialogResponse(h, { requests: { rows: [requestRecord('first')], nextCursor: 'next' } });
  const first = dialogQuery(h);
  h.advance(5001);
  h.root().render();
  const refreshed = dialogQuery(h);
  assert.notEqual(refreshed.requestToken, first.requestToken);
  assert.equal(refreshed.cursor, null);
  assert.match(h.details().textContent, /first@example\.com/);
  dialogResponse(h, { requests: { rows: [requestRecord('updated')], nextCursor: 'next' } });
  assert.doesNotMatch(h.details().textContent, /first@example\.com/);
  dialogAction(h, 'more');
  dialogResponse(h, { requests: { rows: [requestRecord('older')], nextCursor: null } });
  h.details().querySelector('[data-cockpit-quota-refresh]')!.click();
  assert.equal(dialogQuery(h).cursor, null);
  dialogResponse(h, { requests: { rows: [requestRecord('latest')], nextCursor: null } });
  assert.match(h.details().textContent, /latest@example\.com/);
  assert.doesNotMatch(h.details().textContent, /updated@example\.com|older@example\.com/);
});

test('request performance uses actual output-generation timing and excludes short, failed or unmeasured samples', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  h.evaluate();
  openRequests(h);
  const cases = [
    { outputTokens: 100, outputGenerationMs: 100 },
    { outputTokens: 99, outputGenerationMs: 100 },
    { outputTokens: 100, outputGenerationMs: 99 },
    { outputTokens: 100, outputGenerationMs: null },
    { outputTokens: null, outputGenerationMs: 100 },
    { outputTokens: 200, outputGenerationMs: 200, success: false },
    { outputTokens: 200, outputGenerationMs: 200, errorCategory: 'stream_incomplete' },
    { outputTokens: 200, outputGenerationMs: 200, errorCategory: 'client_canceled' },
  ];
  dialogResponse(h, { requests: { rows: cases.map((data, i) => requestRecord('sample-' + i, data)), nextCursor: null } });
  const rows = h.details().querySelectorAll('.request-row');
  assert.equal(rows.length, cases.length);
  assert.match(rows[0].children[4].textContent, /First response1\.20sDuration6\.20sOutput speed1000 tok\/s/);
  for (const row of rows.slice(1)) assert.match(row.children[4].textContent, /Output speed—/);
  assert.match(rows[6].children[5].textContent, /Incomplete stream/);
  assert.match(rows[7].children[5].textContent, /Cancelled/);
  assert.deepEqual(h.details().querySelector('.request-table')!.querySelectorAll('th').map(el => el.textContent),
    ['Time', 'Account', 'Model', 'Token', 'Performance', 'Result']);
});

test('click opens shared details, close and Escape close them and restore trigger focus', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  h.evaluate({ usage_state: sessionUsage(h.now()) });
  h.host().click();
  assert.equal(h.details().style.display, 'flex');
  assert.equal(h.details().getAttribute('role'), 'dialog');
  assert.match(h.details().textContent, /Current session.*Average output speed50\.0\s?tok\/s.*Average cache hits80\.0%/);
  assert.equal(h.details().querySelector('[data-cockpit-quota-close]')!.focused, true);
  h.details().querySelector('[data-cockpit-quota-close]')!.click();
  assert.equal(h.details().style.display, 'none');
  assert.equal(h.host().focused, true);
  h.host().click();
  let prevented = false;
  h.dispatch('keydown', { key: 'Escape', preventDefault() { prevented = true; } });
  assert.equal(prevented, true);
  assert.equal(h.details().style.display, 'none');
  assert.equal(h.host().focused, true);
});

test('outside mouse, touch and pen presses close the dialog without consuming the outside action or stealing focus', () => {
  for (const pointerType of ['mouse','touch','pen']) {
    const h = harness('injection_script',[composer('gpt-6-sol')]);
    h.evaluate();
    h.host().click();
    const pending = dialogQuery(h);
    const outside = new Element('button');
    h.body.appendChild(outside);
    let clicks = 0;
    outside.onclick = () => { clicks++; };
    outside.focus();
    h.dispatch('pointerdown',{target:outside,pointerType,
      preventDefault(){assert.fail('outside action must remain available');},
      stopPropagation(){assert.fail('outside event must propagate');}});
    assert.equal(h.details().style.display,'none');
    assert.equal(outside.focused,true);
    assert.equal(h.host().focused,false);
    outside.click();
    assert.equal(clicks,1);
    assert.equal(h.root().dialogState.pending,null,'closing releases the in-flight query');
    h.host().click();
    assert.equal(h.details().style.display,'flex');
    assert.notEqual(dialogQuery(h).requestToken,pending.requestToken);
  }
});

test('inside presses and retargeted inner events stay open, while the status strip toggles exactly once', () => {
  const h = harness('injection_script',[composer('gpt-6-sol')]);
  h.evaluate();
  h.host().click();
  for (const target of [h.details(),h.details().querySelector('.business-content')!,h.details().querySelector('[data-cockpit-quota-close]')!]) {
    h.dispatch('pointerdown',{target});
    assert.equal(h.details().style.display,'flex');
  }
  h.dispatch('pointerdown',{target:h.body,composedPath:()=>[h.details(),h.body]});
  assert.equal(h.details().style.display,'flex');
  h.dispatch('pointerdown',{target:null});
  assert.equal(h.details().style.display,'flex');
  const logs = h.details().querySelector('[data-value="logs"]')!;
  h.dispatch('pointerdown',{target:logs});
  logs.click();
  assert.equal(h.root().dialogState.tab,'logs');
  h.dispatch('pointerdown',{target:h.details().querySelector('.request-scroll')!});
  assert.equal(h.details().style.display,'flex');
  h.dispatch('pointerdown',{target:h.host().querySelector('[data-cockpit-metric-value]')!});
  assert.equal(h.details().style.display,'flex');
  h.host().click();
  assert.equal(h.details().style.display,'none');
  h.dispatch('pointerdown',{target:h.host()});
  h.host().click();
  assert.equal(h.details().style.display,'flex');
});

test('usage details retain breakdowns and controls without repeated totals or explanatory paragraphs', () => {
  for (const hasUsage of [false, true]) {
    const h = harness('injection_script', [composer('gpt-6-sol')]);
    h.evaluate({ usage_state: hasUsage ? sessionUsage(h.now()) : {},
      plans: [{ plan: 'plus', count: 2, fiveHourRemainingPercent: 75, weeklyRemainingPercent: 50 }] });
    h.host().click();
    const details = h.details();
    assert.match(details.textContent, /Account quotas.*Plan.*PLUS/);
    const css = h.head.querySelector('[data-cockpit-quota-style]')!.textContent;
    assert.match(css, /\.plan-table[^}]*text-align:center/);
    assert.doesNotMatch(details.textContent, /5h75%|Week50%/);
    assert.match(h.host().textContent, /5h 75%Week 50%/);
    assert.match(details.textContent, /Current session.*Average output speed.*Average cache hits/);
    assert.ok(details.querySelector('[data-cockpit-quota-close]'));
    assert.ok(details.querySelector('[data-cockpit-quota-refresh]'));
    for (const key of ['aggregateHint', 'speedHint', 'cacheHint', 'waiting'] as const) {
      assert.equal(details.textContent.includes(values.usage_labels[key]), false, key);
    }
    assert.ok(metric(h.host(), 'speed').getAttribute('title')?.includes(values.usage_labels.speedHint));
  }
});

test('switching the current composer thread immediately clears previous metrics and closes details', () => {
  const active = composer('gpt-6-sol');
  const h = harness('injection_script', [active]);
  h.evaluate({ usage_state: sessionUsage(h.now()) });
  h.host().click();
  active.conversation.setAttribute('data-above-composer-conversation-id', secondThread);
  h.root().render();
  assert.equal(h.root().threadId, secondThread);
  assert.equal(h.details().style.display, 'none');
  assert.match(h.host().textContent, /— tok\/sCache —/);
  assert.doesNotMatch(h.host().textContent, /50\.0\s?tok\/s|Cache 80.0%/);
  active.conversation.setAttribute('data-above-composer-conversation-id', `local:client-new-thread:${secondThread}`);
  h.root().render();
  assert.equal(h.root().threadId, '');
  assert.match(h.host().textContent, /— tok\/sCache —/);
});

test('a new or stopped turn retains verified session average speed and cache', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  const usage = sessionUsage(h.now());
  h.evaluate({ usage_state: usage });
  usage[firstThread].currentTurnId = 'new-turn';
  h.evaluate({ usage_state: usage });
  assert.match(h.host().textContent, /50\.0 tok\/sCache 80.0%/);
});

test('toolbar gap shrinks by removing cache before speed and restores all metrics when widened', () => {
  const active = composer('gpt-6-sol');
  const h = harness('injection_script', [active]);
  h.evaluate({ usage_state: sessionUsage(h.now()) });
  const shrinkGap = (width: number) => {
    active.trigger.rect.left = 196 + width;
    active.trigger.rect.width = 900 - active.trigger.rect.left;
    h.root().render();
  };
  assert.equal(h.host().querySelectorAll('[data-cockpit-quota-metric]').length, 4);
  shrinkGap(220);
  assert.deepEqual(h.host().querySelectorAll('[data-cockpit-quota-metric]').map(node => node.textContent), ['5h 75%', 'Week 50%', '↗50.0 tok/s']);
  shrinkGap(140);
  assert.deepEqual(h.host().querySelectorAll('[data-cockpit-quota-metric]').map(node => node.textContent), ['5h 75%', 'Week 50%']);
  h.host().click();
  assert.match(h.details().textContent, /Average output speed50\.0\s?tok\/s.*Average cache hits80\.0%/);
  shrinkGap(50);
  assert.equal(h.host().style.display, 'none');
  assert.equal(h.details().style.display, 'none');
  shrinkGap(400);
  assert.equal(h.host().style.display, 'flex');
  assert.equal(h.host().querySelectorAll('[data-cockpit-quota-metric]').length, 4);
});

test('details refresh issues a token, keeps failure inside the dialog and clears it on retry', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  h.evaluate();
  h.host().click();
  h.details().querySelector('[data-cockpit-quota-refresh]')!.click();
  const token = h.root().refreshRequestToken;
  assert.equal(typeof token, 'string');
  assert.ok(token.length > 0);
  assert.equal(h.details().querySelector('[data-cockpit-quota-refresh]')!.disabled, true);
  assert.equal(h.evaluate().refreshRequestToken, token);
  h.evaluate({ handled_refresh_token: token, refresh_error: 'refreshFailed', refresh_in_progress: true });
  assert.equal(h.details().querySelector('[data-cockpit-quota-error]'), null);
  h.evaluate({ handled_refresh_token: token, refresh_error: 'refreshFailed' });
  assert.equal(h.root().refreshRequestToken, null);
  assert.equal(h.details().style.display, 'flex');
  assert.match(h.details().querySelector('[data-cockpit-quota-error]')!.textContent, /Quota refresh failed/);
  assert.equal(h.host().querySelector('[data-cockpit-quota-error]'), null);
  assert.equal(h.details().querySelectorAll('[data-cockpit-quota-error]').length, 1);
  h.details().querySelector('[data-cockpit-quota-refresh]')!.click();
  assert.equal(h.details().querySelector('[data-cockpit-quota-error]'), null);
  assert.notEqual(h.root().refreshRequestToken, token);
  assert.equal(h.details().style.display, 'flex');
});

test('actual status strip width rather than composer width decides placement for large quota totals', () => {
  const active = composer('gpt-6-sol');
  const h = harness('injection_script', [active]);
  h.evaluate({ usage_state: sessionUsage(h.now()), five_hour: 123456 });
  active.trigger.rect.left = 346;
  active.trigger.rect.width = 554;
  h.root().render();
  assert.equal(h.host().querySelectorAll('[data-cockpit-quota-metric]').length, 2);
  assert.match(h.host().textContent, /5h 123456%/);
  h.host().click();
  assert.match(h.details().textContent, /Average output speed50\.0\s?tok\/s.*Average cache hits80\.0%/);
});

test('host disconnect ends refresh loading and keeps the dialog usable until reconnect', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  h.evaluate({ usage_state: sessionUsage(h.now()) });
  h.host().click();
  h.details().querySelector('[data-cockpit-quota-refresh]')!.click();
  h.advance(8001);
  h.intervals.forEach(callback => callback());
  assert.equal(h.root().refreshing, false);
  assert.equal(h.root().refreshRequestToken, null);
  assert.match(h.details().textContent, /Host unavailable/);
  assert.equal(h.details().querySelector('[data-cockpit-quota-refresh]')!.disabled, true);
  h.details().querySelector('[data-cockpit-quota-close]')!.click();
  assert.equal(h.details().style.display, 'none');
  h.evaluate();
  h.host().click();
  assert.equal(h.details().querySelector('[data-cockpit-quota-refresh]')!.disabled, false);
  assert.doesNotMatch(h.details().textContent, /Host unavailable/);
});

test('repeat injection reuses nodes, observers and handlers, and Escape renders the newest data', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  h.evaluate();
  const host = h.host();
  const details = h.details();
  const observer = h.root().observer;
  for (let index = 0; index < 3; index++) h.evaluate({ five_hour: 42, usage_state: sessionUsage(h.now()),
    plans: [{ plan: 'PLUS', count: 1, fiveHourRemainingPercent: 42, weeklyRemainingPercent: 50 }] });
  assert.equal(h.host(), host);
  assert.equal(h.details(), details);
  assert.equal(h.body.querySelectorAll('[data-cockpit-quota-footer]').length, 1);
  assert.equal(h.body.querySelectorAll('[data-cockpit-quota-details]').length, 1);
  assert.equal(h.head.querySelectorAll('[data-cockpit-quota-style]').length, 1);
  assert.equal(h.root().observer, observer);
  for (const name of ['resize', 'scroll', 'keydown', 'pointerdown', 'focusin']) assert.equal(h.handlers.get(name)!.size, 1);
  assert.equal(h.intervals.size, 1);
  h.host().click();
  h.dispatch('keydown', { key: 'Escape' });
  assert.match(h.host().textContent, /5h 42%.*50\.0\s?tok\/s.*Cache 80.0%/);
  h.host().click();
  assert.match(h.details().textContent, /PLUS142%50%/);
  assert.doesNotMatch(h.details().textContent, /5h42%/);
});

test('automatic metric updates preserve current detail focus and body scroll position', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  h.evaluate({ usage_state: sessionUsage(h.now()) });
  h.host().click();
  h.details().querySelector('.cockpit-usage-body')!.scrollTop = 240;
  h.details().querySelector('[data-cockpit-quota-close]')!.focus();
  h.evaluate({ five_hour: 42, usage_state: sessionUsage(h.now()),
    plans: [{ plan: 'PLUS', count: 1, fiveHourRemainingPercent: 42, weeklyRemainingPercent: 50 }] });
  assert.equal(h.details().querySelector('.cockpit-usage-body')!.scrollTop, 240);
  assert.equal(h.details().querySelector('[data-cockpit-quota-close]')!.focused, true);
  assert.match(h.details().textContent, /PLUS142%50%/);
  assert.doesNotMatch(h.details().textContent, /5h42%/);
});

test('switching to standalone DeepSeek and back cleans up status line handlers and restores observation', () => {
  const active = composer('deepseek-v4-pro');
  const h = harness('injection_script', [active]);
  h.evaluate();
  assert.equal(h.handlers.get('keydown')!.size, 1);
  assert.equal(h.handlers.get('pointerdown')!.size, 1);
  assert.equal(h.handlers.get('focusin')!.size, 1);
  h.evaluateAs('deepseek_balance_injection_script');
  assert.equal(h.handlers.get('keydown')!.size, 0);
  assert.equal(h.handlers.get('pointerdown')!.size, 0);
  assert.equal(h.handlers.get('focusin')!.size, 1, 'only the active DeepSeek renderer retains a focus listener');
  assert.equal(h.root().keyHandler, undefined);
  assert.equal(h.root().outsidePointerHandler, undefined);
  assert.equal(h.root().focusHandler, undefined);
  assert.equal(h.body.querySelector('[data-cockpit-quota-footer]'), null);
  h.evaluate();
  assert.equal(h.handlers.get('keydown')!.size, 1);
  assert.equal(h.handlers.get('focusin')!.size, 1);
  assert.deepEqual(h.root().layoutObserver.targets, [active.footer, active.permissions, active.trigger]);
  assert.equal(h.host().parentElement, h.body);
  assert.equal(h.body.querySelector('[data-cockpit-deepseek-balance]'), null);
});

test('measured overview height removes unused bottom space and stays identical across request tabs', () => {
  for (const [model,naturalHeight] of [['deepseek-flash',214],['grok-4.6',238],['gpt-6-sol',264]] as const) {
    const h = harness('injection_script',[composer(model)]);
    let probes = 0;
    h.measure(node => {
      const rect = {...node.rect};
      if (node.hasAttribute('data-cockpit-quota-details')) return {...rect,width:660,height:Number.parseFloat(node.style.height) || 700};
      if (node.matches('.cockpit-usage-measure')) {
        probes++;
        assert.equal(node.getAttribute('aria-hidden'),'true');
        assert.equal(node.hasAttribute('inert'),true);
        assert.equal(node.querySelectorAll('.business-section').length,3);
        return {...rect,height:naturalHeight};
      }
      if (node.matches('.topbar')) return {...rect,height:44};
      if (node.matches('.tabs')) return {...rect,height:34};
      if (node.matches('.footer')) return {...rect,height:40};
      return null;
    });
    h.evaluate(); h.host().click();
    assert.equal(h.details().style.height,(naturalHeight + 120) + 'px');
    assert.equal(h.details().querySelector('.cockpit-usage-measure'),null,'measurement nodes never remain in the visible/focus tree');
    const initialHeight = h.details().style.height;
    const initialProbes = probes;
    h.root().render();
    assert.equal(probes,initialProbes,'unchanged markup and width do not force repeated layout measurements');
    dialogAction(h,'tab','logs');
    assert.equal(h.details().style.height,initialHeight);
    dialogResponse(h,{requests:{rows:Array.from({length:10},(_,index)=>requestRecord(String(index))),nextCursor:'more'}});
    h.details().querySelector('.request-scroll')!.scrollTop = 180;
    dialogAction(h,'more');
    dialogResponse(h,{requests:{rows:[requestRecord('older')],nextCursor:null}});
    assert.equal(h.details().style.height,initialHeight,'loading more records only expands the internal list');
    assert.equal(h.details().querySelector('.request-scroll')!.scrollTop,180);
    dialogAction(h,'tab','overview');
    assert.equal(h.details().style.height,initialHeight);
  }
});

test('viewport changes, wrapped errors and long quotas keep header and actions inside the adaptive frame', () => {
  const h = harness('injection_script',[composer('gpt-6-sol',false,220)]);
  let naturalHeight = 220;
  h.measure(node => {
    const rect = {...node.rect};
    if (node.hasAttribute('data-cockpit-quota-details')) return {...rect,width:Number.parseFloat(node.style.width) || 660,height:Number.parseFloat(node.style.height) || 700};
    if (node.matches('.cockpit-usage-measure')) return {...rect,height:naturalHeight};
    if (node.matches('.topbar')) return {...rect,height:44};
    if (node.matches('.tabs')) return {...rect,height:34};
    if (node.matches('.footer')) return {...rect,height:node.querySelector('[data-cockpit-quota-error]') ? 76 : 40};
    return null;
  });
  h.window.innerHeight = 600;
  h.evaluate(); h.host().click();
  assert.equal(h.details().style.height,'340px');
  dialogResponse(h,{error:true});
  assert.equal(h.details().style.height,'376px','wrapped feedback is measured in the fixed action area');
  dialogAction(h,'tab','logs');
  h.window.innerWidth = 500;
  naturalHeight = 340;
  h.dispatch('resize');
  assert.equal(h.details().style.width,'476px');
  assert.equal(h.details().style.height,'460px');
  naturalHeight = 1400;
  h.window.innerWidth = 450;
  h.window.innerHeight = 1200;
  h.dispatch('resize');
  assert.equal(h.details().style.height,'700px');
  h.window.innerHeight = 300;
  h.dispatch('resize');
  assert.equal(h.details().style.height,'255px');
  assert.ok(Number.parseFloat(h.details().style.top) >= 12);
  assert.ok(Number.parseFloat(h.details().style.top) + 255 <= h.window.innerHeight - 12);
  assert.ok(h.details().querySelector('header')!.querySelector('[data-cockpit-quota-close]'));
  assert.ok(h.details().querySelector('footer')!.querySelector('[data-cockpit-quota-refresh]'));
});

test('600px viewport and long plan lists keep a bounded dialog with a scrollable body and fixed actions', () => {
  const h = harness('injection_script', [composer('gpt-6-sol', false, 500)]);
  h.window.innerHeight = 600;
  h.evaluate({ plans: Array.from({ length: 80 }, (_, index) => ({ plan: `PLAN_${index}`, count: 1 })) });
  h.details().rect.height = 560;
  h.host().click();
  assert.equal(h.details().querySelectorAll('tbody').length, 1);
  assert.equal(h.details().querySelectorAll('tr').length, 81);
  assert.ok(Number.parseFloat(h.details().style.top) >= 12);
  assert.ok(Number.parseFloat(h.details().style.top) + h.details().rect.height <= 588);
  assert.ok(h.details().querySelector('header')!.querySelector('[data-cockpit-quota-close]'));
  assert.ok(h.details().querySelector('footer')!.querySelector('[data-cockpit-quota-refresh]'));
  const css = h.head.querySelector('[data-cockpit-quota-style]')!.textContent;
  assert.match(css, /\[data-cockpit-quota-details\]\{[^}]*height:auto;max-height:min\(700px,85dvh,calc\(100dvh - 32px\)\)/);
  assert.match(css, /\.panel\{[^}]*min-height:0;overflow-y:auto/);
  assert.match(css, /\.topbar\{[^}]*flex-shrink:0/);
  assert.match(css, /\.footer\{[^}]*flex-shrink:0/);
  assert.match(css, /\.request-scroll\{[^}]*min-height:0;overflow:auto/);
  assert.match(css, /\.request-table th\{position:sticky;top:0/);
  assert.doesNotMatch(css, /\[data-view=[^}]*height:/);
  assert.doesNotMatch(css, /max-height:\s*none/);
});

const metric = (host: Element, key: string) => host.querySelector(`[data-cockpit-metric-key="${key}"]`)!;
function assertNoFill(node: Element) {
  assert.ok(node, 'expected a mounted metric');
  assert.equal(node.getAttribute('data-cockpit-progress'), null);
  assert.equal(node.getAttribute('data-cockpit-fill-tone'), null);
  assert.equal(node.style.getPropertyValue('--cockpit-quota-fill-width'), '');
}
function assertTone(node: Element, tone: 'high' | 'medium' | 'low' | 'critical' | null = null) {
  assertNoFill(node);
  assert.equal(node.getAttribute('data-cockpit-quota-tone'), tone);
}

test('aged throughput retains its speed styling and marks its age only in the hint', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  const usage = sessionUsage(h.now());
  h.evaluate({ usage_state: usage });
  const speed = metric(h.host(), 'speed');
  assert.equal(speed.textContent, '↗50.0 tok/s');
  assert.equal(speed.getAttribute('data-stale'), null);
  h.advance(121000);
  h.evaluate({ usage_state: usage });
  assert.equal(metric(h.host(), 'speed'), speed);
  assert.equal(speed.textContent, '↗50.0 tok/s');
  assert.equal(speed.getAttribute('data-stale'), 'true');
  assert.match(speed.getAttribute('title')!, /Recent sample/);
  assertTone(speed);
  const css = h.head.querySelector('[data-cockpit-quota-style]')!.textContent;
  assert.doesNotMatch(css, /\[data-stale[^\]]*\][^{]*\{[^}]*opacity/);
  h.evaluate({ usage_state: sessionUsage(h.now()) });
  assert.equal(metric(h.host(), 'speed'), speed);
  assert.equal(speed.getAttribute('data-stale'), null);
  assert.doesNotMatch(speed.getAttribute('title')!, /Recent sample/);
});

test('normal quota values keep raw pool totals with account overview colors and no progress fills', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  h.evaluate({ five_hour: 295, weekly: 270, five_hour_capacity: 300, weekly_capacity: 300,
    account_count_value: 12, usage_state: sessionUsage(h.now()) });
  assert.equal(metric(h.host(), 'five-hour').textContent, '5h 295%');
  assert.equal(metric(h.host(), 'weekly').textContent, 'Week 270%');
  for (const key of ['five-hour', 'weekly']) assertTone(metric(h.host(), key),'high');
  for (const key of ['cache','speed']) assertTone(metric(h.host(),key));
  h.host().click();
  assert.equal(h.details().style.display, 'flex');
  assert.doesNotMatch(h.details().textContent, /5h295%|Week270%/);
  assert.match(h.details().textContent, /Current session.*Average output speed/);
});

test('quota colors use the displayed totals independently of pool capacity or account count', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  for (const account_count_value of [null, 0, 1, 12, 100]) {
    h.evaluate({ five_hour: 30, weekly: 15, five_hour_capacity: 300, weekly_capacity: 100,
      account_count_value, plans: [{ plan: 'PLUS', count: 1 }, { plan: 'TEAM', count: 11 }] });
    assertTone(metric(h.host(), 'five-hour'), 'low');
    assertTone(metric(h.host(), 'weekly'), 'low');
    assert.equal(metric(h.host(), 'five-hour').textContent, '5h 30%');
    assert.equal(metric(h.host(), 'weekly').textContent, 'Week 15%');
  }
});

test('missing or invalid capacities never change the account overview color of a known percentage', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  for (const five_hour_capacity of [null, undefined, 0, -100, '300', NaN, Infinity]) {
    h.evaluate({ five_hour: 30, weekly: 30, five_hour_capacity, weekly_capacity: 300, account_count_value: 3 });
    assert.equal(metric(h.host(), 'five-hour').textContent, '5h 30%');
    assertTone(metric(h.host(), 'five-hour'),'low');
    assertTone(metric(h.host(), 'weekly'), 'low');
  }
  for (const weekly_capacity of [null, 0, -1, '300']) {
    h.evaluate({ five_hour: 30, weekly: 30, five_hour_capacity: 300, weekly_capacity, account_count_value: 3 });
    assertTone(metric(h.host(), 'five-hour'), 'low');
    assertTone(metric(h.host(), 'weekly'),'low');
  }
});

test('unknown remaining values stay uncolored, totals above 100 are green, and zero is critical', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  for (const five_hour of [null, undefined, -1, NaN, Infinity, '295']) {
    h.evaluate({ five_hour, weekly: null, five_hour_capacity: 300, weekly_capacity: null });
    assertTone(metric(h.host(), 'five-hour'));
    assert.doesNotMatch(h.host().textContent, /NaN|Infinity/);
  }
  h.evaluate({ five_hour: 450, weekly: 0, five_hour_capacity: 300, weekly_capacity: 300 });
  assertTone(metric(h.host(), 'five-hour'),'high');
  assertTone(metric(h.host(), 'weekly'), 'critical');
  assert.equal(metric(h.host(), 'five-hour').textContent, '5h 450%');
});

test('quota colors match the actual account overview helper at every threshold, including 46 and 809 percent', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  for (const remaining of [0,9.999,10,10.001,39.999,40,40.001,46,79.999,80,80.001,97,100,809]) {
    h.evaluate({ five_hour: remaining, five_hour_capacity: 300, weekly: remaining, weekly_capacity: 300 });
    const tone = getCodexQuotaClass(remaining) as 'high' | 'medium' | 'low' | 'critical';
    assertTone(metric(h.host(), 'five-hour'), tone);
    assertTone(metric(h.host(), 'weekly'), tone);
    h.host().click();
    h.evaluate({five_hour:remaining,weekly:remaining,plans:[{plan:'plus',count:1,fiveHourRemainingPercent:remaining,weeklyRemainingPercent:remaining}]});
    const cells = h.details().querySelectorAll('.quota-percent');
    assert.equal(cells.length,2);
    cells.forEach(node => assert.equal(node.getAttribute('data-cockpit-quota-tone'),tone));
    h.details().querySelector('[data-cockpit-quota-close]')!.click();
  }
});

test('cache values stay ordinary numbers for low, high and missing hit rates', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  for (const cached of [0, 10, 20, 80, 150]) {
    const usage = sessionUsage(h.now());
    usage[firstThread].outputSpeed.totalCachedTokens = 1000 * cached / 100;
    h.evaluate({ usage_state: usage });
    const cache = metric(h.host(), 'cache');
    assert.equal(cache.querySelector('[data-cockpit-metric-value]')!.textContent, cached > 100 ? '—' : `${cached}.0%`);
    assertTone(cache);
    assertTone(metric(h.host(), 'speed'));
  }
  h.evaluate({ usage_state: {} });
  assert.equal(metric(h.host(), 'cache').textContent, 'Cache —');
  assertTone(metric(h.host(), 'cache'));
});

test('footer and overview cache percentages truncate to exactly one decimal without rounding', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  for (const [input, cached, expected] of [
    [1000, 997, '99.7%'],
    [10000, 9999, '99.9%'],
    [100000, 99999, '99.9%'],
    [300, 299, '99.6%'],
    [3, 1, '33.3%'],
    [100, 29, '29.0%'],
    [100000, 1, '0.0%'],
    [1000, 0, '0.0%'],
    [1000, 1000, '100.0%'],
    [Number.MAX_SAFE_INTEGER, Number.MAX_SAFE_INTEGER - 1, '99.9%'],
  ] as const) {
    const usage = sessionUsage(h.now());
    Object.assign(usage[firstThread].outputSpeed, { totalInputTokens: input, totalCachedTokens: cached });
    h.evaluate({ usage_state: usage });
    assert.equal(metric(h.host(), 'cache').querySelector('[data-cockpit-metric-value]')!.textContent, expected);
    h.host().click();
    assert.ok(h.details().textContent.includes('Average cache hits' + expected));
    h.details().querySelector('[data-cockpit-quota-close]')!.click();
  }
});

test('footer and overview output speeds truncate to exactly one decimal without rounding', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  for (const [rate, expected] of [
    [35, '35.0'], [35.79, '35.7'], [99.99, '99.9'], [0.09, '0.0'], [28.7, '28.7'],
  ] as const) {
    const usage = sessionUsage(h.now());
    usage[firstThread].outputSpeed.tokensPerSecond = rate;
    h.evaluate({ usage_state: usage });
    assert.equal(metric(h.host(), 'speed').textContent, `↗${expected} tok/s`);
    h.host().click();
    assert.ok(h.details().textContent.replace(/\s/g, '').includes('Averageoutputspeed' + expected + 'tok/s'));
    h.details().querySelector('[data-cockpit-quota-close]')!.click();
  }
});

test('Grok uses its shared window for warning colors while DeepSeek balance stays uncolored', () => {
  const active = composer('grok-4.6');
  const h = harness('injection_script', [active]);
  h.evaluate({ grok_badge_percent: 15, account_count_value: 8, five_hour_capacity: 800, usage_state: sessionUsage(h.now()) });
  assert.equal(metric(h.host(), 'grok').textContent, 'Build 15%');
  assertTone(metric(h.host(), 'grok'), 'low');
  assertTone(metric(h.host(), 'speed'));
  active.trigger.textContent = 'deepseek-v4-pro';
  h.evaluate({ usage_state: sessionUsage(h.now()) });
  assert.equal(metric(h.host(), 'deepseek').textContent, 'Balance ¥10.00');
  assertTone(metric(h.host(), 'deepseek'));
  assert.equal(metric(h.host(), 'grok'), null);
});

test('Grok status strip and details retain the thresholds of its own account overview', () => {
  const h = harness('injection_script',[composer('grok-4.6')]);
  for (const remaining of [0,9.999,10,10.001,30,30.001,60,60.001,100]) {
    h.evaluate({grok_badge_percent:remaining,grok_lines_value:[{product:'GrokBuild',count:1,remainingPercent:remaining}]});
    const tone = getGrokQuotaClass(100 - remaining);
    assertTone(metric(h.host(),'grok'),tone);
    h.host().click();
    assert.equal(h.details().querySelector('.quota-percent')!.getAttribute('data-cockpit-quota-tone'),tone);
    h.details().querySelector('[data-cockpit-quota-close]')!.click();
  }
});

test('injected quota palette uses exactly the account overview text colors', () => {
  const h = harness('injection_script',[composer('gpt-6-sol')]); h.evaluate();
  const injected = h.head.querySelector('[data-cockpit-quota-style]')!.textContent;
  const overview = readFileSync(new URL('../src/styles/pages/codex-account-cards.css',import.meta.url),'utf8');
  const base = readFileSync(new URL('../src/styles/base.css',import.meta.url),'utf8');
  for (const tone of ['high','medium','low','critical']) {
    const raw = overview.match(new RegExp('\\.codex-quota-section \\.quota-pct\\.' + tone + '\\s*\\{\\s*color:\\s*([^;]+)'))![1];
    const token = raw.match(/^var\((--[^)]+)\)$/)?.[1];
    const color = token ? base.match(new RegExp(token + ':\\s*(#[a-f0-9]+)'))![1] : raw;
    assert.equal(injected.match(new RegExp('--cockpit-quota-' + tone + ':(#[a-f0-9]+)'))![1],color);
  }
});

test('reinjecting the same renderer version refreshes changed CSS without replacing focused controls', () => {
  const h = harness('injection_script',[composer('gpt-6-sol')]); h.evaluate();
  const host = h.host(), style = h.head.querySelector('[data-cockpit-quota-style]')!;
  host.focus();
  style.textContent = 'old fixed size and old quota palette';
  h.evaluate();
  assert.equal(h.host(),host);
  assert.equal(h.host().focused,true);
  assert.equal(h.head.querySelector('[data-cockpit-quota-style]'),style);
  assert.match(style.textContent,/height:auto;max-height:min\(700px/);
  assert.match(style.textContent,/--cockpit-quota-high:#22c55e/);
  assert.doesNotMatch(style.textContent,/old fixed size/);
});

test('changed styles invalidate cached overview measurements without a version bump', () => {
  const h = harness('injection_script',[composer('deepseek-flash')]);
  let naturalHeight = 220, probes = 0;
  h.measure(node => {
    if (!node.matches('.cockpit-usage-measure')) return null;
    probes++;
    return {...node.rect,height:naturalHeight};
  });
  h.evaluate(); h.host().click();
  const firstHeight = Number.parseFloat(h.details().style.height), firstProbes = probes;
  const details = h.details();
  naturalHeight += 25;
  h.head.querySelector('[data-cockpit-quota-style]')!.textContent = 'old layout rules';
  h.evaluate();
  assert.equal(h.details(),details);
  assert.equal(h.details().style.height,(firstHeight + 25) + 'px');
  assert.equal(probes,firstProbes + 1);
});

test('live value and warning updates preserve metric nodes and summary focus', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  h.evaluate({ five_hour: 295, weekly: 270, five_hour_capacity: 300, weekly_capacity: 300, usage_state: sessionUsage(h.now()) });
  const host = h.host();
  const nodes = [...host.children];
  const initialInsertionCount = host.insertions.length;
  host.focus();
  h.evaluate({ five_hour: 150, weekly: 30, five_hour_capacity: 300, weekly_capacity: 300, usage_state: sessionUsage(h.now()) });
  assert.equal(h.host(), host);
  assert.deepEqual(host.children, nodes);
  assert.equal(host.insertions.length, initialInsertionCount, 'unchanged order does not detach or reinsert visible metrics');
  assert.equal(host.focused, true);
  assertTone(metric(host, 'five-hour'),'high');
  assertTone(metric(host, 'weekly'), 'low');
  assert.match(host.textContent, /5h 150%/);
  assert.doesNotMatch(host.textContent, /5h 295%/);
  host.click();
  assert.equal(h.details().style.display, 'flex');
});

test('known-to-unknown quotas clear warnings and later samples update the same metric', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  h.evaluate({ five_hour: 30, five_hour_capacity: 300 });
  const node = metric(h.host(), 'five-hour');
  assertTone(node, 'low');
  h.evaluate({ five_hour: null, weekly:null, five_hour_capacity: null });
  assert.equal(metric(h.host(), 'five-hour'), node);
  assertTone(node);
  assert.equal(node.textContent, '5h —');
  h.evaluate({ five_hour: 150, five_hour_capacity: 300 });
  assert.equal(metric(h.host(), 'five-hour'), node);
  assertTone(node,'high');
});

test('narrow toolbar hides cached nodes and restores the same objects without disturbing visible quota nodes', () => {
  const active = composer('gpt-6-sol');
  const h = harness('injection_script', [active]);
  h.evaluate({ usage_state: sessionUsage(h.now()) });
  const keys = ['five-hour', 'weekly', 'speed', 'cache'];
  const nodes = keys.map(key => metric(h.host(), key));
  active.trigger.rect.left = 416;
  active.trigger.rect.width = 484;
  h.root().render();
  assert.deepEqual(h.host().children, nodes.slice(0, 3));
  assert.equal(nodes[3].parentElement, null);
  assert.equal(h.root().metricNodes.get('cache'), nodes[3]);
  active.trigger.rect.left = 336;
  active.trigger.rect.width = 564;
  h.root().render();
  assert.deepEqual(h.host().children, nodes.slice(0, 2));
  assert.equal(nodes[2].parentElement, null);
  assert.equal(h.root().metricNodes.get('speed'), nodes[2]);
  active.trigger.rect.left = 850;
  active.trigger.rect.width = 50;
  h.root().render();
  assert.deepEqual(h.host().children, nodes);
  keys.forEach((key, index) => { assert.equal(metric(h.host(), key), nodes[index]); });
});

test('first metric mount has its warning state and no progress properties or animation', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  h.evaluate({ five_hour: 30, five_hour_capacity: 300 });
  const node = metric(h.host(), 'five-hour');
  const firstMount = h.host().insertions.find(insertion => insertion.node === node)!;
  assert.ok(firstMount);
  assert.equal(firstMount.tone, 'low');
  assert.equal(firstMount.progress, null);
  assert.equal(firstMount.width, '');
  const css = h.head.querySelector('[data-cockpit-quota-style]')!.textContent;
  assert.doesNotMatch(css, /data-cockpit-progress|cockpit-quota-fill|transition:\s*width/);
});

for (const oldVersion of [3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13]) test(`upgrading v${oldVersion} injection replaces old styles and listeners with v14 exactly once`, () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  h.evaluate();
  const oldHost = h.host();
  const oldStyle = h.head.querySelector('[data-cockpit-quota-style]')!;
  oldStyle.textContent = '[data-cockpit-progress]::before{width:var(--cockpit-quota-fill-width)}';
  h.root().statuslineVersion = oldVersion;
  h.evaluate({ five_hour: 30, five_hour_capacity: 300 });
  assert.equal(h.root().statuslineVersion, 14);
  assert.notEqual(h.host(), oldHost);
  assert.equal(oldHost.parentElement, null);
  assert.equal(oldStyle.parentElement, null);
  assert.equal(h.head.querySelectorAll('[data-cockpit-quota-style]').length, 1);
  assertTone(metric(h.host(), 'five-hour'), 'low');
  assert.doesNotMatch(h.head.querySelector('[data-cockpit-quota-style]')!.textContent, /data-cockpit-progress|cockpit-quota-fill/);
  const host = h.host();
  const style = h.head.querySelector('[data-cockpit-quota-style]')!;
  h.evaluate();
  assert.equal(h.host(), host);
  assert.equal(h.head.querySelector('[data-cockpit-quota-style]'), style);
  for (const name of ['resize', 'scroll', 'keydown', 'pointerdown', 'focusin']) assert.equal(h.handlers.get(name)!.size, 1);
  assert.equal(h.intervals.size, 1);
});


test('statusline speed uses only verified measurements for the selected conversation', () => {
  const active = composer('gpt-6-sol');
  const h = harness('injection_script', [active]);
  const usage = { ...sessionUsage(h.now()), ...sessionUsage(h.now(), secondThread) };
  Object.assign(usage[firstThread].outputSpeed,{tokensPerSecond:20,latestTokensPerSecond:259 / 7.372,latestObservedAtMs:h.now()});
  Object.assign(usage[secondThread].outputSpeed,{tokensPerSecond:25,latestTokensPerSecond:80,latestObservedAtMs:h.now()});
  h.evaluate({ usage_state: usage });
  assert.equal(metric(h.host(), 'speed').textContent, '↗20.0 tok/s', 'the session mean is independent of the last request');
  active.conversation.setAttribute('data-above-composer-conversation-id', secondThread);
  h.root().render();
  assert.equal(metric(h.host(), 'speed').textContent, '↗25.0 tok/s');
  active.conversation.setAttribute('data-above-composer-conversation-id', firstThread);
  h.root().render();
  assert.equal(metric(h.host(), 'speed').textContent, '↗20.0 tok/s');
});

test('missing or mismatched API timing never falls back to rollout or other conversation speeds', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  const usage = sessionUsage(h.now());
  for (const measured of [undefined, null,
    { threadId: secondThread, tokensPerSecond: 90, samples: 1, observedAtMs: h.now() },
    { threadId: firstThread, tokensPerSecond: null, samples: 1, observedAtMs: h.now() },
    { threadId: firstThread, tokensPerSecond: 90, samples: 0, observedAtMs: h.now() },
    { threadId: firstThread, tokensPerSecond: 90, samples: 1, observedAtMs: 0 }]) {
    h.evaluate({ usage_state: {
      ...sessionUsage(h.now(), secondThread),
      [firstThread]: { ...usage[firstThread], outputSpeed: measured },
    } });
    assert.equal(metric(h.host(), 'speed').textContent, '↗— tok/s');
    assert.equal(metric(h.host(), 'cache').textContent, 'Cache —', 'missing or mismatched request identity cannot inherit rollout cache');
  }
  h.evaluate({ usage_state: {} });
  assert.equal(metric(h.host(), 'speed').textContent, '↗— tok/s');
});

test('footer and overview share session means after generation stops, ignoring the last request', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  const usage = sessionUsage(h.now());
  Object.assign(usage[firstThread].outputSpeed, { tokensPerSecond: 29, latestTokensPerSecond: 66,
    latestInputTokens: 1000, latestCachedTokens: 920, avgCacheHitRate: .74, totalCachedTokens: 740 });
  h.evaluate({ usage_state: usage });
  assert.equal(metric(h.host(), 'speed').textContent, '↗29.0 tok/s');
  assert.equal(metric(h.host(), 'cache').textContent, 'Cache 74.0%');
  usage[firstThread].events = [];
  usage[firstThread].currentTurnId = 'stopped-or-new-turn';
  h.advance(600000);
  for (let index = 0; index < 20; index++) h.evaluate({ usage_state: usage });
  assert.equal(metric(h.host(), 'speed').textContent, '↗29.0 tok/s');
  assert.equal(metric(h.host(), 'cache').textContent, 'Cache 74.0%');
  h.host().click();
  assert.match(h.details().querySelector('.session-metrics')!.textContent,
    /Average output speed29\.0\s?tok\/sAverage cache hits74\.0%/);
});

test('a cache-only session shows a verified zero cache ratio without inventing speed', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  const usage = sessionUsage(h.now());
  h.evaluate({ usage_state: usage });
  Object.assign(usage[firstThread].outputSpeed, { requestId: 'new-cache-only-request', latestObservedAtMs: h.now(),
    latestTokensPerSecond: null, samples: 0, tokensPerSecond: 0, latestInputTokens: 80, latestCachedTokens: 0, totalInputTokens: 80, totalCachedTokens: 0 });
  h.evaluate({ usage_state: usage });
  assert.equal(metric(h.host(), 'speed').textContent, '↗— tok/s');
  assert.equal(metric(h.host(), 'cache').textContent, 'Cache 0.0%');
  assert.equal(metric(h.host(), 'cache').hasAttribute('data-cockpit-metric-unknown'), false);
});

test('session cache totals preserve missing versus zero and never use rollout data to fill gaps', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  for (const cached of [undefined, null, '', '0', -1, NaN, Infinity, 1001, Number.MAX_SAFE_INTEGER + 1]) {
    const usage = sessionUsage(h.now());
    Object.assign(usage[firstThread].outputSpeed, { totalCachedTokens: cached, latestCacheHitRate: .8 });
    h.evaluate({ usage_state: usage });
    assert.equal(metric(h.host(), 'cache').textContent, 'Cache —', String(cached));
    assert.equal(metric(h.host(), 'speed').textContent, '↗50.0 tok/s');
  }
  for (const input of [undefined, null, 0, -1, '1000', Number.MAX_SAFE_INTEGER + 1]) {
    const usage = sessionUsage(h.now());
    usage[firstThread].outputSpeed.totalInputTokens = input;
    h.evaluate({ usage_state: usage });
    assert.equal(metric(h.host(), 'cache').textContent, 'Cache —', String(input));
  }
  const zero = sessionUsage(h.now());
  zero[firstThread].outputSpeed.totalCachedTokens = 0;
  h.evaluate({ usage_state: zero });
  assert.equal(metric(h.host(), 'cache').textContent, 'Cache 0.0%');
});

test('model changes preserve verified session means, including switching back', () => {
  const active = composer('gpt-6-sol');
  const h = harness('injection_script', [active]);
  const usage = sessionUsage(h.now());
  h.evaluate({ usage_state: usage });
  h.advance(1000);
  active.trigger.textContent = '6.1 Sol';
  let result = h.evaluate({ usage_state: usage });
  assert.equal(result.usageSelection.model, null);
  assert.match(h.host().textContent, /50\.0 tok\/sCache 80.0%/);
  const selected = sessionUsage(h.now());
  selected.__binding.model = 'gpt-6.1-sol';
  Object.assign(selected[firstThread].outputSpeed, { model: 'gpt-6.1-sol', selection: { model: 'gpt-6.1-sol', serviceTier: 'default' },
    latestObservedAtMs: h.now(), latestTokensPerSecond: 70, latestCachedTokens: 900 });
  result = h.evaluate({ usage_state: selected });
  assert.equal(result.usageSelection.model, 'gpt-6.1-sol');
  assert.equal(metric(h.host(), 'speed').textContent, '↗50.0 tok/s');
  assert.equal(metric(h.host(), 'cache').textContent, 'Cache 80.0%');
  h.advance(1000);
  active.trigger.textContent = 'gpt-6-sol';
  h.evaluate({ usage_state: usage });
  assert.match(h.host().textContent, /50\.0 tok\/sCache 80.0%/, 'returning to a previous model keeps the session mean');
});

test('native fast metadata follows explicit choices while footer means remain stable', () => {
  const active = composer('gpt-6-sol');
  const h = harness('injection_script', [active]);
  const usage = sessionUsage(h.now());
  h.evaluate({ usage_state: usage });
  const toggle = new Element('span');
  toggle.setAttribute('data-fast-mode-enabled', 'true');
  active.footer.appendChild(toggle);
  h.advance(1000);
  let result = h.evaluate({ usage_state: usage });
  assert.equal(result.usageSelection.serviceTier, 'priority');
  assert.match(h.host().textContent, /50\.0 tok\/sCache 80.0%/);
  const fast = sessionUsage(h.now());
  Object.assign(fast[firstThread].outputSpeed, { requestId: 'fast-request', latestObservedAtMs: h.now(),
    selection: { model: 'gpt-6-sol', serviceTier: 'priority' }, latestTokensPerSecond: 66, latestCachedTokens: 920 });
  result = h.evaluate({ usage_state: fast });
  assert.equal(result.usageSelection.serviceTier, 'priority');
  assert.equal(metric(h.host(), 'speed').textContent, '↗50.0 tok/s');
  assert.equal(metric(h.host(), 'cache').textContent, 'Cache 80.0%');
  h.advance(1000);
  toggle.setAttribute('data-fast-mode-enabled', 'false');
  h.evaluate({ usage_state: usage });
  assert.match(h.host().textContent, /50\.0 tok\/sCache 80.0%/);
  const unknown = sessionUsage(h.now());
  unknown.__binding.serviceTier = 'priority';
  result = h.evaluate({ usage_state: unknown });
  assert.equal(result.usageSelection.serviceTier, null, 'false does not distinguish default from auto and must not guess');
});

test('same-conversation composer replacement preserves cached rows and open state', () => {
  const active = composer('gpt-6-sol');
  const h = harness('injection_script', [active]);
  h.evaluate({ usage_state: sessionUsage(h.now()) });
  openRequests(h);
  dialogResponse(h, { requests: { rows: [requestRecord('kept')], nextCursor: null } });
  const replacement = composer('gpt-6-sol');
  active.root.remove();
  h.body.appendChild(replacement.root);
  h.root().render();
  assert.equal(h.details().style.display, 'flex');
  assert.match(h.details().textContent, /kept@example.com/);
});

test('request tier rendering follows API service evidence and keeps returned raw tier separate', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  h.evaluate();
  openRequests(h);
  dialogResponse(h, { requests: { rows: [
    requestRecord('fast', { requestedServiceTier: 'FAST', responseServiceTier: 'default' }),
    requestRecord('legacy', { serviceTier: 'priority' }),
    requestRecord('adjusted', { requestedServiceTier: 'auto', serviceTierAdaptation:
      { requestedTier: 'priority', sentTier: 'default', reason: 'official_default_only' } }),
  ], nextCursor: null } });
  const rows = h.details().querySelectorAll('.request-row');
  assert.match(rows[0].textContent, /highRequest: FastReported: default/);
  assert.doesNotMatch(rows[0].textContent, /Adjusted to Standard/);
  assert.match(rows[1].textContent, /Request: Not recordedReported: Not recordedLegacy: priority/);
  assert.match(rows[2].textContent, /Request: FastReported: Not recordedAdjusted to Standard/);
});

test('a different API service binding cannot reuse latest or mean measurements from the former binding', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  const usage = sessionUsage(h.now());
  h.evaluate({ usage_state: usage });
  assert.equal(metric(h.host(), 'cache').textContent, 'Cache 80.0%');
  usage.__binding.binding = 'different-service-key-instance';
  h.evaluate({ usage_state: usage });
  assert.match(h.host().textContent, /— tok\/sCache —/);
  h.host().click();
  assert.match(h.details().querySelector('.session-metrics')!.textContent, /Average output speed—Average cache hits—/);
  usage[firstThread].outputSpeed.binding = usage.__binding.binding;
  h.evaluate({ usage_state: usage });
  assert.equal(metric(h.host(), 'cache').textContent, 'Cache 80.0%');
});

test('only exact known native service-tier SVGs establish Fast or Ultrafast with a closed picker', () => {
  const icons = JSON.parse(readFileSync(new URL('./fixtures/codex-native-tier-icons.json', import.meta.url), 'utf8')).icons;
  for (const icon of icons) for (const variant of ['native','other-viewBox','modified-path']) {
    const active = composer('gpt-6-sol');
    const h = harness('injection_script', [active]);
    const svg = new Element('svg');
    svg.setAttribute('viewBox', variant === 'other-viewBox' ? '0 0 24 24' : icon.viewBox);
    const path = new Element('path');
    path.setAttribute('d', variant === 'modified-path' ? icon.path + ' M0 0' : icon.path);
    svg.appendChild(path);
    active.trigger.appendChild(svg);
    const usage = sessionUsage(h.now());
    usage.__binding.serviceTier = null;
    usage[firstThread].outputSpeed.selection.serviceTier = icon.tier;
    const result = h.evaluate({ usage_state: usage });
    if (variant === 'native') {
      assert.equal(result.usageSelection.serviceTier, icon.tier);
      assert.equal(metric(h.host(), 'speed').textContent, '↗50.0 tok/s');
    } else {
      assert.equal(result.usageSelection.serviceTier, null, variant + ' is not confirmed native tier evidence');
      assert.match(h.host().textContent, /50\.0 tok\/sCache 80.0%/);
    }
  }
});

test('native tier changes retain session averages without waiting for a new request', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  const usage = sessionUsage(h.now());
  usage.__binding.serviceTier = null;
  usage[firstThread].outputSpeed.selection.serviceTier = null;
  usage[firstThread].outputSpeed.requestedServiceTier = null;
  h.evaluate({ usage_state: usage });
  assert.equal(metric(h.host(), 'speed').textContent, '↗50.0 tok/s');
  assert.equal(metric(h.host(), 'cache').textContent, 'Cache 80.0%');
  h.advance(1000);
  const fast = sessionUsage(h.now());
  fast.__binding.serviceTier = 'priority';
  fast[firstThread].outputSpeed.selection.serviceTier = 'priority';
  fast[firstThread].outputSpeed.latestObservedAtMs = h.now();
  h.evaluate({ usage_state: fast });
  assert.equal(metric(h.host(), 'speed').textContent, '↗50.0 tok/s');
  h.advance(1000);
  h.evaluate({ usage_state: usage });
  assert.match(h.host().textContent, /50\.0 tok\/sCache 80.0%/, 'session means do not depend on the selected speed mode');
  usage[firstThread].outputSpeed.latestObservedAtMs = h.now();
  h.evaluate({ usage_state: usage });
  assert.equal(metric(h.host(), 'cache').textContent, 'Cache 80.0%');
});

test('overview response default empty request payload cannot erase a cached request list on tab roundtrip', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  h.evaluate({ usage_state: sessionUsage(h.now()) });
  openRequests(h);
  dialogResponse(h, { requests: { rows: [requestRecord('cached')], nextCursor: 'next-page' } });
  dialogAction(h, 'tab', 'overview');
  dialogResponse(h, { summary: { requests: 37 }, requests: { rows: [], nextCursor: null } });
  dialogAction(h, 'tab', 'logs');
  assert.match(h.details().textContent, /cached@example.com/);
  assert.equal(h.root().dialogState.nextCursor, 'next-page');
});

test('live request binding receives cancellation on close and a new token on reopen while rows stay cached', () => {
  const h = harness('injection_script', [composer('gpt-6-sol')]);
  const sent: unknown[] = [];
  h.window.__cockpitUsageRequest = (query: string) => sent.push(JSON.parse(query));
  h.evaluate({ usage_state: sessionUsage(h.now()) });
  openRequests(h);
  const before = dialogQuery(h).requestToken;
  dialogResponse(h, { requests: { rows: [requestRecord('kept')], nextCursor: null } });
  h.details().querySelector('[data-cockpit-quota-close]')!.click();
  assert.equal(sent.at(-1), null);
  h.host().click();
  assert.notEqual(dialogQuery(h).requestToken, before);
  assert.match(h.details().textContent, /kept@example.com/);
});

test('closing a menu after disabling Fast retains its explicit native choice instead of reverting to stale profile settings', () => {
  const active = composer('gpt-6-sol');
  const h = harness('injection_script', [active]);
  const usage = sessionUsage(h.now());
  usage.__binding.serviceTier = 'priority';
  usage[firstThread].outputSpeed.selection.serviceTier = 'priority';
  h.evaluate({ usage_state: usage });
  active.trigger.setAttribute('aria-expanded', 'true');
  const menu = dropdown('gpt-6-sol');
  const toggle = new Element('span');
  toggle.setAttribute('data-fast-mode-enabled', 'false');
  menu.appendChild(toggle);
  h.body.appendChild(menu);
  h.advance(1000);
  let result = h.evaluate({ usage_state: usage });
  assert.equal(result.usageSelection.serviceTier, null, 'explicit false only rejects Fast; it cannot name default or auto');
  assert.match(h.host().textContent, /50\.0 tok\/sCache 80.0%/);
  menu.remove();
  h.advance(1000);
  result = h.evaluate({ usage_state: usage });
  assert.equal(result.usageSelection.serviceTier, null);
  const normal = sessionUsage(h.now());
  normal.__binding.serviceTier = 'priority';
  Object.assign(normal[firstThread].outputSpeed, { selection: { model:'gpt-6-sol',serviceTier:null },
    tokensPerSecond:30,totalCachedTokens:700,latestObservedAtMs:h.now(),latestTokensPerSecond:66,latestCachedTokens:920,requestId:'after-fast-disabled' });
  h.evaluate({ usage_state: normal });
  assert.equal(metric(h.host(), 'speed').textContent, '↗30.0 tok/s');
  assert.equal(metric(h.host(), 'cache').textContent, 'Cache 70.0%');
  const newMenu = dropdown('gpt-6-sol');
  const newToggle = new Element('span');
  newToggle.setAttribute('data-fast-mode-enabled','true');
  newMenu.appendChild(newToggle);
  h.body.appendChild(newMenu);
  h.advance(1000);
  result = h.evaluate({ usage_state: normal });
  assert.equal(result.usageSelection.serviceTier, 'priority', 'a new explicit native observation replaces the cached one');
});

test('cached native per-chat tier observations never carry across conversations, models or service bindings', () => {
  for (const change of ['thread','model','binding']) {
    const active = composer('gpt-6-sol');
    const h = harness('injection_script', [active]);
    const usage = { ...sessionUsage(h.now()), ...sessionUsage(h.now(), secondThread) };
    usage.__binding.serviceTier = 'priority';
    usage[firstThread].outputSpeed.selection.serviceTier = 'priority';
    usage[secondThread].outputSpeed.selection.serviceTier = 'priority';
    const toggle = new Element('span');
    toggle.setAttribute('data-fast-mode-enabled','false');
    active.footer.appendChild(toggle);
    let result = h.evaluate({ usage_state: usage });
    assert.equal(result.usageSelection.serviceTier, null);
    toggle.remove();
    h.evaluate({ usage_state: usage });
    assert.equal(h.root().nativeSpeedState.enabled, false);
    if (change === 'thread') active.conversation.setAttribute('data-above-composer-conversation-id',secondThread);
    else if (change === 'model') {
      active.trigger.textContent = 'gpt-6.1-sol';
      usage.__binding.model = 'gpt-6.1-sol';
      usage[firstThread].outputSpeed.model = 'gpt-6.1-sol';
      usage[firstThread].outputSpeed.selection.model = 'gpt-6.1-sol';
    } else {
      usage.__binding.binding = 'new-service-binding';
      usage[firstThread].outputSpeed.binding = usage.__binding.binding;
    }
    result = h.evaluate({ usage_state: usage });
    assert.equal(result.usageSelection.serviceTier, 'priority', change + ' reads its own verified configuration');
    assert.equal(h.root().nativeSpeedState, null, change + ' clears the former native observation');
    if (change === 'thread') assert.equal(metric(h.host(), 'speed').textContent, '↗50.0 tok/s');
  }
});
