import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { runInNewContext } from 'node:vm';

const source = readFileSync(new URL('../src-tauri/src/modules/codex_app_injection.rs', import.meta.url), 'utf8');
type ScriptKind = 'injection_script' | 'deepseek_balance_injection_script';
const values: Record<string, unknown> = {
  provider: 'Provider', weekly: 50, five_hour: 75, account_count_value: 2,
  available_account_count_value: 2, abnormal_account_count_value: 0, cooldown_account_count_value: 0,
  plans: [], grok_lines_value: [], grok_badge_percent: 90, grok_badge_product: 'GrokBuild',
  badge_kind_value: 'codex', fallback_model_value: 'gpt-5.4',
  account_pool_label: 'Accounts', weekly_label: 'Week', five_hour_label: '5h',
  account_pool_title: 'Account pool', quota_empty_label: 'No quota', available_text: 'Available',
  issue_text: 'Issues', refresh_label: 'Refresh', close_label: 'Close', missing_label: '—',
  balance_label: 'Balance', account_balance_label: 'Account balance', total_balance_label: 'Total balance',
  quota_overview_label: 'Quota', grok_empty_label: 'No quota',
  balance_lines: [{ totalBalance: 10, currency: 'CNY' }],
  refresh_in_progress: false, handled_refresh_token: null,
  balance: { totalBalance: 10, currency: 'CNY', isAvailable: true },
  granted_balance_label: 'Granted', topped_up_balance_label: 'Top-up',
  available_label: 'Available', unavailable_label: 'Unavailable',
};

// Execute the shipped raw JS template, including Rust format! brace escaping.
// No duplicate implementation of model detection or badge rendering lives in this test.
function script(kind: ScriptKind) {
  const functionStart = source.indexOf(`fn ${kind}(`);
  assert.ok(functionStart >= 0, `missing ${kind}`);
  const start = source.indexOf('r#"(()', functionStart);
  assert.ok(start >= 0, `missing raw JS template for ${kind}`);
  const end = source.indexOf('"#', start + 3);
  assert.ok(end > start);
  return source.slice(start + 3, end).replace(/\{\{|\}\}|\{([a-z_]+)\}/g, (token, key: string | undefined) => {
    if (token === '{{') return '{';
    if (token === '}}') return '}';
    assert.ok(key && Object.hasOwn(values, key), `unhandled template argument ${key}`);
    return JSON.stringify(values[key!]);
  });
}

class Element {
  children: Element[] = [];
  parentElement: Element | null = null;
  attributes: Record<string, string> = {};
  style: Record<string, string> = {};
  innerHTML = '';
  textContent = '';
  hidden = false;
  rect = { left: 100, top: 900, width: 80, height: 24 };

  setAttribute(key: string, value: string) { this.attributes[key] = value; }
  appendChild(node: Element) {
    node.remove();
    node.parentElement = this;
    this.children.push(node);
  }
  remove() {
    if (this.parentElement) this.parentElement.children = this.parentElement.children.filter((child) => child !== this);
    this.parentElement = null;
  }
  contains(node: Element): boolean { return node === this || this.children.some((child) => child.contains(node)); }
  matches(selector: string) {
    return selector.split(',').some((part) => {
      const match = /^\[([^=*\]]+)(?:(\*?=)"([^"]*)"( i)?)?\]$/.exec(part.trim());
      assert.ok(match, `unsupported mock selector ${part}`);
      let value = this.attributes[match[1]];
      if (!match[2]) return value !== undefined;
      if (value === undefined) return false;
      let expected = match[3];
      if (match[4]) { value = value.toLowerCase(); expected = expected.toLowerCase(); }
      return match[2] === '*=' ? value.includes(expected) : value === expected;
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
    return this.getClientRects().length ? this.rect : { left: 0, top: 0, width: 0, height: 0 };
  }
}

function composer(model: string, hidden = false, top = 900) {
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
  return { footer, trigger, permissions };
}

function dropdown(model: string, hidden = false) {
  const dropdown = new Element();
  dropdown.setAttribute('class', '_ModelPickerDropdownContent_test');
  dropdown.hidden = hidden;
  const row = new Element();
  row.setAttribute('data-model-picker-model-row', 'true');
  row.textContent = model;
  dropdown.appendChild(row);
  return dropdown;
}

function harness(kind: ScriptKind, composers: ReturnType<typeof composer>[]) {
  const html = new Element();
  const body = new Element();
  const head = new Element();
  html.appendChild(body);
  html.appendChild(head);
  composers.forEach(({ footer }) => body.appendChild(footer));
  const document = {
    body, head, documentElement: html, createElement: () => new Element(),
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
    ResizeObserver, setInterval: () => 1, clearInterval() {}, addEventListener() {}, removeEventListener() {},
  };
  let now = Date.now();
  class Clock extends Date { static now() { return now; } }
  const context = { document, window, Date: Clock, ResizeObserver, MutationObserver, requestAnimationFrame: (fn: () => void) => fn() };
  const namespace = kind === 'injection_script' ? '__cockpitCodexInjection' : '__cockpitDeepSeekBalance';
  const marker = kind === 'injection_script' ? 'quota-footer' : 'deepseek-balance';
  return {
    body, window,
    evaluate: () => runInNewContext(script(kind), context),
    root: () => window[namespace],
    host: () => document.querySelector(`[data-cockpit-${marker}]`)!,
    advance: (milliseconds: number) => { now += milliseconds; },
  };
}

test('quota badge reads the visible GPT composer rather than a retained hidden DeepSeek composer', () => {
  const active = composer('gpt-5.4');
  const h = harness('injection_script', [composer('deepseek-v4-pro', true), active]);
  assert.equal(h.evaluate().selectedModel, 'gpt-5.4');
  assert.equal(h.host().style.top, '912px');
  assert.equal(h.host().style.left, '515px');
  assert.match(h.host().innerHTML, /5h 75%/);
  assert.doesNotMatch(h.host().innerHTML, /Balance/);
  assert.deepEqual(h.root().layoutObserver.targets, [active.footer, active.permissions]);
});

test('quota badge reads visible DeepSeek and updates the model, badge, and observers on conversation change', () => {
  const old = composer('deepseek-v4-pro');
  const h = harness('injection_script', [composer('gpt-5.4', true), old]);
  assert.equal(h.evaluate().selectedModel, 'deepseek-v4-pro');
  assert.match(h.host().innerHTML, /Balance ¥10.00/);
  old.footer.hidden = true;
  const next = composer('grok-4.6', false, 500);
  h.body.appendChild(next.footer);
  assert.equal(h.evaluate().selectedModel, 'grok-4.6');
  assert.match(h.host().innerHTML, /Build 90%/);
  assert.doesNotMatch(h.host().innerHTML, /Balance/);
  assert.equal(h.host().style.top, '512px');
  assert.deepEqual(h.root().layoutObserver.targets, [next.footer, next.permissions]);
});

test('only a visible picker dropdown can supply a model when the current trigger is a placeholder', () => {
  const h = harness('injection_script', [composer('Select effort')]);
  h.body.appendChild(dropdown('deepseek-v4-pro', true));
  h.body.appendChild(dropdown('grok-4.6'));
  assert.equal(h.evaluate().selectedModel, 'grok-4.6');
  assert.match(h.host().innerHTML, /Build 90%/);
});

test('placeholder text retains the same composer model briefly, then returns to the host fallback', () => {
  const active = composer('deepseek-v4-pro');
  const h = harness('injection_script', [active]);
  h.evaluate();
  active.trigger.textContent = 'Select effort';
  h.body.appendChild(dropdown('grok-4.6', true));
  assert.equal(h.evaluate().selectedModel, null);
  assert.match(h.host().innerHTML, /Balance ¥10.00/);
  h.advance(15001);
  assert.equal(h.evaluate().selectedModel, null);
  assert.match(h.host().innerHTML, /5h 75%/);
});

test('a new composer with placeholder text does not reuse the previous composer model cache', () => {
  const old = composer('deepseek-v4-pro');
  const h = harness('injection_script', [old]);
  h.evaluate();
  old.footer.hidden = true;
  h.body.appendChild(composer('Select effort', false, 500).footer);
  assert.equal(h.evaluate().selectedModel, null);
  assert.match(h.host().innerHTML, /5h 75%/);
  assert.doesNotMatch(h.host().innerHTML, /Balance/);
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
    active.footer.hidden = true;
    h.root().render();
    assert.equal(h.host().style.display, 'none');
    if (kind === 'injection_script') assert.equal(h.root().visibleModelFooter, null);
  });

  test(`${kind}: separate windows keep their own composer and state`, () => {
    const a = harness(kind, [composer('deepseek-v4-pro', true), composer('gpt-5.4', false, 500)]);
    const b = harness(kind, [composer('gpt-5.4', true), composer('deepseek-v4-pro')]);
    a.evaluate();
    b.evaluate();
    assert.equal(a.host().style.top, '512px');
    assert.equal(b.host().style.top, '912px');
    assert.notEqual(a.root(), b.root());
    if (kind === 'injection_script') {
      assert.match(a.host().innerHTML, /5h 75%/);
      assert.match(b.host().innerHTML, /Balance ¥10.00/);
    }
  });
}
