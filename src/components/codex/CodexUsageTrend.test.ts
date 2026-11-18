import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../../tests/helpers/reactHookHarness';

function nodes(value: any): any[] {
  if (!value || typeof value !== 'object') return [];
  if (Array.isArray(value)) return value.flatMap(nodes);
  return [value, ...nodes(value.props?.children)];
}

function chartHarness(width: number, zoom = 1, count = 5, reasoningReported = false, days?: number[]) {
  let resize: () => void = () => {};
  let disconnected = false;
  const h = loadHookModule(new URL('./CodexUsageTrend.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({
      t: (key: string, fallback?: string) => fallback ?? key,
      i18n: { language: 'en-US' },
    }) },
  }, {
    ResizeObserver: class {
      constructor(callback: () => void) { resize = callback; }
      observe() {}
      disconnect() { disconnected = true; }
    },
  });
  const wrap = {
    clientWidth: width,
    getBoundingClientRect: () => ({ left: 40, width: wrap.clientWidth * zoom }),
  };
  const svg = {
    getBoundingClientRect: () => ({ left: 40, width: wrap.clientWidth * zoom }),
    getScreenCTM() {
      const scale = Math.min(wrap.clientWidth / 960, 292 / 286) * zoom;
      const offset = 40 + (wrap.clientWidth * zoom - 960 * scale) / 2;
      return { a: scale, e: offset, inverse: () => ({ a: 1 / scale, e: -offset / scale }) };
    },
    createSVGPoint() {
      return { x: 0, y: 0, matrixTransform(matrix: { a: number; e: number }) {
        return { x: this.x * matrix.a + matrix.e };
      } };
    },
  };
  const points = Array.from({ length: count }, (_, index) => ({
    bucketStart: Date.UTC(2026, 9, days?.[index] ?? index + 1),
    usage: {
      totalTokens: (index + 1) * 100,
      requestCount: index + 1,
      estimatedCostUsd: index,
      inputTokens: (index + 1) * 80,
      outputTokens: (index + 1) * 20,
      ...(reasoningReported ? { reasoningTokens: (index + 1) * 5 } : {}),
    },
  }));
  let tree = h.render(() => h.exports.CodexUsageTrend({ points }));
  for (const node of nodes(tree)) {
    if (!node.props?.ref) continue;
    if (node.type === 'svg') node.props.ref.current = svg;
    if (node.props.className === 'codex-usage-trend__chart-wrap') node.props.ref.current = wrap;
  }
  function find(className: string) {
    return nodes(tree).find((node) => node.props?.className === className);
  }
  function screenX(index: number) {
    const x = count === 1 ? 500 : 58 + index / (count - 1) * 884;
    const matrix = svg.getScreenCTM();
    return x * matrix.a + matrix.e;
  }
  return {
    h, wrap, svg, find, screenX,
    flush() { tree = h.flush(); },
    resize() { resize(); tree = h.flush(); },
    get disconnected() { return disconnected; },
    get tree() { return tree; },
  };
}

test('hover selects the bucket under the pointer with SVG letterboxing and zoom', () => {
  for (const width of [560, 960, 2400]) {
    for (const zoom of [0.7, 1, 1.25]) {
      const c = chartHarness(width, zoom);
      for (const index of [0, 1, 2, 3, 4]) {
        c.find('codex-usage-trend__interaction-layer').props.onMouseMove({
          currentTarget: { ownerSVGElement: c.svg }, clientX: c.screenX(index), clientY: 100,
        });
        c.flush();
        const tooltip = c.find('codex-usage-trend__tooltip');
        assert.equal(nodes(tooltip).find((node) => node.type === 'strong').props.children,
          `10/${index + 1}`, `width=${width}, zoom=${zoom}, bucket=${index}`);
      }
      c.h.unmount();
    }
  }
});

test('tooltip follows a focused bucket and updates after resizing without mouse movement', () => {
  const c = chartHarness(2400, 1.25);
  nodes(c.tree).filter((node) => node.type === 'circle')[1].props.onFocus();
  c.flush();
  const assertAligned = () => {
    const tooltip = c.find('codex-usage-trend__tooltip');
    const left = tooltip.props.style.left;
    const localX = typeof left === 'number' ? left : parseFloat(left) / 100 * c.wrap.clientWidth;
    assert.ok(Math.abs(localX - (c.screenX(1) - 40) / 1.25) < 0.01,
      `tooltip left ${left} must match the rendered bucket`);
  };
  assertAligned();
  c.wrap.clientWidth = 1200;
  c.resize();
  assertAligned();
  c.h.unmount();
  assert.equal(c.disconnected, true);
});

test('single buckets remain selectable and detached SVGs ignore mouse movement', () => {
  const c = chartHarness(2400, 1, 1);
  const clientX = c.screenX(0);
  const move = () => c.find('codex-usage-trend__interaction-layer').props.onMouseMove({
    currentTarget: { ownerSVGElement: c.svg }, clientX, clientY: 100,
  });
  move();
  c.flush();
  assert.ok(c.find('codex-usage-trend__tooltip'));
  c.find('codex-usage-trend__interaction-layer').props.onMouseLeave();
  c.flush();
  c.svg.getScreenCTM = () => null as any;
  move();
  c.flush();
  assert.equal(c.find('codex-usage-trend__tooltip'), undefined);
  c.h.unmount();
});

test('token bars, running totals, and the ring use the same selected buckets', () => {
  const c = chartHarness(960, 1, 3, true);
  const bars = nodes(c.tree).filter((node) => node.props.className === 'codex-usage-trend__period-bar');
  assert.deepEqual(bars.map((bar) => Math.round(bar.props.height)), [67, 133, 200]);
  const line = c.find('codex-usage-trend__cumulative-line');
  assert.deepEqual(String(line.props.d).split(' ').filter((_, index) => [2, 6, 10].includes(index)).map(Number).map(Math.round), [195, 128, 28]);
  const composition = c.find('codex-usage-trend__composition');
  const portions = nodes(composition).filter((node) => node.type === 'b');
  assert.deepEqual(portions.map((node) => [node.props.title, node.props.children]), [['480', '80%'], ['90', '15%'], ['30', '5%']]);
  nodes(c.tree).filter((node) => node.type === 'circle')[1].props.onFocus();
  c.flush();
  const tooltip = c.find('codex-usage-trend__tooltip');
  assert.deepEqual(nodes(tooltip).filter((node) => node.type === 'b').slice(0, 2).map((node) => node.props.children), ['200', '300']);
  c.h.unmount();
});

test('days without records retain their time on the horizontal axis and do not raise the running total', () => {
  const c = chartHarness(960, 1, 3, true, [1, 2, 5]);
  const bars = nodes(c.tree).filter((node) => node.props.className === 'codex-usage-trend__period-bar');
  assert.deepEqual(bars.map((bar) => bar.props.x + bar.props.width / 2), [58, 279, 942]);
  assert.match(c.find('codex-usage-trend__cumulative-line').props.d, /H 279 V 128 H 942 V 28$/);
  c.find('codex-usage-trend__interaction-layer').props.onMouseMove({
    currentTarget: { ownerSVGElement: c.svg }, clientX: 40 + 279, clientY: 100,
  });
  c.flush();
  assert.equal(nodes(c.find('codex-usage-trend__tooltip')).find((node) => node.type === 'strong').props.children, '10/2');
  c.h.unmount();
});

test('sources without reasoning use full output and request mode keeps its own trend', () => {
  const c = chartHarness(960, 1, 3);
  const composition = c.find('codex-usage-trend__composition');
  assert.deepEqual(nodes(composition).filter((node) => node.type === 'b').map((node) => node.props.children), ['80%', '20%']);
  nodes(c.tree).find((node) => node.type === 'button' && node.props.children === '总请求数').props.onClick();
  c.flush();
  assert.equal(c.find('codex-usage-trend__composition'), undefined);
  assert.equal(c.find('codex-usage-trend__cumulative-line'), undefined);
  assert.equal(c.find('codex-usage-trend__total').props.children, '6');
  assert.ok(c.find('codex-usage-trend__line'));
  c.h.unmount();
});
