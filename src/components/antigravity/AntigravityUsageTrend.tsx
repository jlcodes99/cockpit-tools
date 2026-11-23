import {
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type MouseEvent,
} from 'react';
import { useTranslation } from 'react-i18next';
import type { TimeBucketPoint } from '../../types/antigravityTokenUsage';
import {
  formatSessionUsageCount,
  formatSessionUsageTokensShort,
} from '../../utils/codexSessionUsageFormat';
import '../codex/CodexUsageTrend.css';

export type AntigravityTrendMetric =
  | 'totalTokens'
  | 'inputTokens'
  | 'thinkingTokens'
  | 'outputTokens'
  | 'cacheTokens'
  | 'turnCount';

type TrendValueLabel = {
  text: string;
  x: number;
};

const CHART_WIDTH = 960;
const CHART_HEIGHT = 286;
const PLOT = { left: 58, right: 18, top: 28, bottom: 58 };

function extractMetricValue(
  bucket: TimeBucketPoint,
  metric: AntigravityTrendMetric,
): number {
  switch (metric) {
    case 'inputTokens':
      return bucket.exact.inputTokens;
    case 'thinkingTokens':
      return bucket.exact.thinkingTokens;
    case 'outputTokens':
      return bucket.exact.outputTokens;
    case 'cacheTokens':
      return bucket.exact.cacheReadTokens + bucket.exact.cacheWriteTokens;
    case 'turnCount':
      return bucket.turnCount;
    case 'totalTokens':
    default:
      return bucket.exact.totalTokens;
  }
}

function formatMetricValue(
  value: number,
  metric: AntigravityTrendMetric,
  lang: string,
): string {
  if (metric === 'turnCount') {
    return formatSessionUsageCount(value);
  }
  return formatSessionUsageTokensShort(value, lang, 2);
}

function buildSmoothPath(points: Array<{ x: number; y: number }>): string {
  if (points.length === 0) return '';
  if (points.length === 1) {
    return `M ${points[0].x.toFixed(2)} ${points[0].y.toFixed(2)}`;
  }
  return points.slice(0, -1).reduce((path, current, index) => {
    const next = points[index + 1];
    const previous = points[Math.max(0, index - 1)];
    const afterNext = points[Math.min(points.length - 1, index + 2)];
    const controlOne = {
      x: current.x + (next.x - previous.x) / 6,
      y: current.y + (next.y - previous.y) / 6,
    };
    const controlTwo = {
      x: next.x - (afterNext.x - current.x) / 6,
      y: next.y - (afterNext.y - current.y) / 6,
    };
    return `${path} C ${controlOne.x.toFixed(2)} ${controlOne.y.toFixed(2)}, ${controlTwo.x.toFixed(2)} ${controlTwo.y.toFixed(2)}, ${next.x.toFixed(2)} ${next.y.toFixed(2)}`;
  }, `M ${points[0].x.toFixed(2)} ${points[0].y.toFixed(2)}`);
}

export function AntigravityUsageTrend({
  series,
  title,
  emptyLabel,
}: {
  series: TimeBucketPoint[];
  title?: string;
  emptyLabel?: string;
}) {
  const { t, i18n } = useTranslation();
  const lang = i18n.resolvedLanguage || i18n.language || 'en';
  const chartId = useId().replace(/:/g, '');
  const [metric, setMetric] = useState<AntigravityTrendMetric>('totalTokens');
  const [hoveredIndex, setHoveredIndex] = useState<number | null>(null);
  const svgRef = useRef<SVGSVGElement>(null);
  const chartWrapRef = useRef<HTMLDivElement>(null);
  const tooltipRef = useRef<HTMLDivElement>(null);
  const [tooltipLeft, setTooltipLeft] = useState(0);

  const metricTabs = useMemo<Array<[AntigravityTrendMetric, string]>>(
    () => [
      ['totalTokens', t('antigravityTokenUsage.cards.total', 'Total Tokens')],
      ['inputTokens', t('antigravityTokenUsage.cards.input', 'Input Tokens')],
      ['thinkingTokens', t('antigravityTokenUsage.cards.thinking', 'Thinking Tokens')],
      ['outputTokens', t('antigravityTokenUsage.cards.output', 'Output Tokens')],
      ['cacheTokens', t('antigravityTokenUsage.cards.cache', 'Cache Tokens')],
      ['turnCount', t('antigravityTokenUsage.cards.turns', 'Turns')],
    ],
    [t],
  );

  const points = useMemo(() => {
    if (series.length === 0) return [];
    const values = series.map((bucket) => ({
      bucket,
      value: extractMetricValue(bucket, metric),
    }));
    const min = Math.min(...values.map((item) => item.value));
    const max = Math.max(...values.map((item) => item.value));
    const span = Math.max(max - min, max * 0.08, 1);
    const xSpan = CHART_WIDTH - PLOT.left - PLOT.right;
    const ySpan = CHART_HEIGHT - PLOT.top - PLOT.bottom;
    return values.map((item, index) => ({
      ...item,
      index,
      x:
        PLOT.left +
        (values.length === 1
          ? xSpan / 2
          : (index / (values.length - 1)) * xSpan),
      y:
        PLOT.top +
        (1 - (item.value - min + span * 0.03) / (span * 1.06)) * ySpan,
    }));
  }, [metric, series]);

  const linePath = buildSmoothPath(points);
  const areaPath =
    points.length > 0
      ? `${linePath} L ${points[points.length - 1].x.toFixed(2)} ${(CHART_HEIGHT - PLOT.bottom).toFixed(2)} L ${points[0].x.toFixed(2)} ${(CHART_HEIGHT - PLOT.bottom).toFixed(2)} Z`
      : '';
  const maxValue =
    points.length > 0 ? Math.max(...points.map((point) => point.value)) : 0;
  const totalValue = points.reduce((sum, point) => sum + point.value, 0);
  const axisStep = Math.max(1, Math.ceil(points.length / 7));
  const axisIndexes = new Set(
    points
      .filter(
        (point) =>
          point.index % axisStep === 0 || point.index === points.length - 1,
      )
      .map((point) => point.index),
  );

  const valueLabels = useMemo<Map<number, TrendValueLabel>>(() => {
    const occupiedRanges: Array<{ start: number; end: number }> = [];
    const labels = new Map<number, TrendValueLabel>();
    const candidates = points
      .filter((point) => point.value > 0)
      .sort(
        (left, right) => right.value - left.value || left.index - right.index,
      );

    candidates.forEach((point) => {
      const text = formatMetricValue(point.value, metric, lang);
      const width = Math.max(42, Math.min(112, text.length * 7 + 16));
      const x = Math.min(
        CHART_WIDTH - PLOT.right - width / 2,
        Math.max(PLOT.left + width / 2, point.x),
      );
      const start = x - width / 2 - 4;
      const end = x + width / 2 + 4;
      if (
        occupiedRanges.some(
          (range) => start < range.end && end > range.start,
        )
      ) {
        return;
      }
      occupiedRanges.push({ start, end });
      labels.set(point.index, { text, x });
    });
    return labels;
  }, [lang, metric, points]);

  const gridYs = [0, 1, 2].map(
    (index) => PLOT.top + (index / 2) * (CHART_HEIGHT - PLOT.top - PLOT.bottom),
  );
  const hoveredPoint =
    hoveredIndex === null ? null : points[hoveredIndex] ?? null;
  const hoveredX = hoveredPoint?.x;

  useLayoutEffect(() => {
    const svg = svgRef.current;
    const wrap = chartWrapRef.current;
    if (!svg || !wrap || hoveredX === undefined) return;
    const updateTooltipPosition = () => {
      const matrix = svg.getScreenCTM();
      const bounds = wrap.getBoundingClientRect();
      if (!matrix || bounds.width === 0) return;
      const point = svg.createSVGPoint();
      point.x = hoveredX;
      const localX =
        ((point.matrixTransform(matrix).x - bounds.left) * wrap.clientWidth) /
        bounds.width;
      const halfWidth = (tooltipRef.current?.offsetWidth ?? 0) / 2 + 8;
      setTooltipLeft(
        Math.max(halfWidth, Math.min(wrap.clientWidth - halfWidth, localX)),
      );
    };
    updateTooltipPosition();
    const observer = new ResizeObserver(updateTooltipPosition);
    observer.observe(wrap);
    return () => observer.disconnect();
  }, [hoveredX]);

  const handleChartMove = (event: MouseEvent<SVGRectElement>) => {
    const svg = event.currentTarget.ownerSVGElement;
    if (!svg || points.length === 0) return;
    const matrix = svg.getScreenCTM();
    if (!matrix) return;
    const pointer = svg.createSVGPoint();
    pointer.x = event.clientX;
    pointer.y = event.clientY;
    const relativeX = pointer.matrixTransform(matrix.inverse()).x;
    const fraction =
      (relativeX - PLOT.left) / (CHART_WIDTH - PLOT.left - PLOT.right);
    setHoveredIndex(
      Math.round(Math.max(0, Math.min(1, fraction)) * (points.length - 1)),
    );
  };

  const chartTitle =
    title ?? t('antigravityTokenUsage.trend.title', 'Token Usage Trend');

  return (
    <section
      className="codex-usage-trend"
      aria-labelledby="ag-usage-trend-title"
    >
      <div className="codex-usage-trend__header">
        <div>
          <h2 id="ag-usage-trend-title">{chartTitle}</h2>
        </div>
        <div className="codex-usage-trend__header-right">
          <strong className="codex-usage-trend__total">
            {formatMetricValue(totalValue, metric, lang)}
          </strong>
          <div
            className="codex-usage-trend__metrics"
            role="group"
            aria-label={chartTitle}
          >
            {metricTabs.map(([value, label]) => (
              <button
                key={value}
                type="button"
                className={`btn btn-secondary btn-sm${metric === value ? ' is-active' : ''}`}
                aria-pressed={metric === value}
                onClick={() => {
                  setMetric(value);
                  setHoveredIndex(null);
                }}
              >
                {label}
              </button>
            ))}
          </div>
        </div>
      </div>

      {points.length === 0 ? (
        <div className="codex-usage-trend__empty" role="status">
          {emptyLabel ??
            t(
              'antigravityTokenUsage.trend.empty',
              'No token usage recorded in the selected time range',
            )}
        </div>
      ) : (
        <div ref={chartWrapRef} className="codex-usage-trend__chart-wrap">
          <svg
            ref={svgRef}
            className="codex-usage-trend__chart"
            viewBox={`0 0 ${CHART_WIDTH} ${CHART_HEIGHT}`}
            role="img"
            aria-label={`${chartTitle} · ${formatMetricValue(maxValue, metric, lang)}`}
          >
            <title>{chartTitle}</title>
            <defs>
              <linearGradient
                id={`${chartId}-area`}
                x1="0"
                y1="0"
                x2="0"
                y2="1"
              >
                <stop
                  offset="0%"
                  stopColor="var(--primary)"
                  stopOpacity="0.26"
                />
                <stop
                  offset="100%"
                  stopColor="var(--primary)"
                  stopOpacity="0.02"
                />
              </linearGradient>
            </defs>
            {gridYs.map((y) => (
              <line
                key={y}
                className="codex-usage-trend__grid"
                x1={PLOT.left}
                x2={CHART_WIDTH - PLOT.right}
                y1={y}
                y2={y}
              />
            ))}
            <line
              className="codex-usage-trend__baseline"
              x1={PLOT.left}
              x2={CHART_WIDTH - PLOT.right}
              y1={CHART_HEIGHT - PLOT.bottom}
              y2={CHART_HEIGHT - PLOT.bottom}
            />
            {areaPath ? (
              <path
                className="codex-usage-trend__area"
                d={areaPath}
                fill={`url(#${chartId}-area)`}
              />
            ) : null}
            {linePath ? (
              <path className="codex-usage-trend__line" d={linePath} />
            ) : null}
            {hoveredPoint ? (
              <>
                <rect
                  className="codex-usage-trend__hover-slice"
                  x={hoveredPoint.x - 18}
                  y={PLOT.top}
                  width="36"
                  height={CHART_HEIGHT - PLOT.top - PLOT.bottom}
                />
                <line
                  className="codex-usage-trend__hover-line"
                  x1={hoveredPoint.x}
                  x2={hoveredPoint.x}
                  y1={PLOT.top}
                  y2={CHART_HEIGHT - PLOT.bottom}
                />
              </>
            ) : null}
            <rect
              className="codex-usage-trend__interaction-layer"
              x={PLOT.left}
              y={PLOT.top}
              width={CHART_WIDTH - PLOT.left - PLOT.right}
              height={CHART_HEIGHT - PLOT.top - PLOT.bottom}
              onMouseMove={handleChartMove}
              onMouseLeave={() => setHoveredIndex(null)}
            />
            {points.map((point) => (
              <g key={point.bucket.bucketKey}>
                {point.value > 0 ? (
                  <circle
                    className={`codex-usage-trend__point${hoveredIndex === point.index ? ' is-hovered' : ''}`}
                    cx={point.x}
                    cy={point.y}
                    r={
                      point.value === maxValue || hoveredIndex === point.index
                        ? 5
                        : 3.5
                    }
                    tabIndex={0}
                    onMouseEnter={() => setHoveredIndex(point.index)}
                    onFocus={() => setHoveredIndex(point.index)}
                    onBlur={() => setHoveredIndex(null)}
                  >
                    <title>{`${point.bucket.label} · ${formatMetricValue(point.value, metric, lang)}`}</title>
                  </circle>
                ) : null}
                {valueLabels.has(point.index)
                  ? (() => {
                      const label = valueLabels.get(point.index);
                      if (!label) return null;
                      const labelWidth = Math.max(
                        42,
                        Math.min(112, label.text.length * 7 + 16),
                      );
                      const labelY = Math.max(PLOT.top + 3, point.y - 29);
                      return (
                        <g className="codex-usage-trend__value-label">
                          <rect
                            x={label.x - labelWidth / 2}
                            y={labelY}
                            width={labelWidth}
                            height="20"
                            rx="7"
                          />
                          <text
                            className="codex-usage-trend__value"
                            x={label.x}
                            y={labelY + 14}
                            textAnchor="middle"
                          >
                            {label.text}
                          </text>
                        </g>
                      );
                    })()
                  : null}
                {axisIndexes.has(point.index) ? (
                  <text
                    className="codex-usage-trend__axis-label"
                    x={point.x}
                    y={CHART_HEIGHT - 21}
                    textAnchor="middle"
                  >
                    {point.bucket.label}
                  </text>
                ) : null}
                {axisIndexes.has(point.index) ? (
                  <text
                    className="codex-usage-trend__axis-value"
                    x={point.x}
                    y={CHART_HEIGHT - 4}
                    textAnchor="middle"
                  >
                    {formatMetricValue(point.value, metric, lang)}
                  </text>
                ) : null}
              </g>
            ))}
          </svg>

          {hoveredPoint ? (
            <div
              ref={tooltipRef}
              className="codex-usage-trend__tooltip"
              role="status"
              style={{ left: tooltipLeft, width: 210 }}
            >
              <strong>
                {hoveredPoint.bucket.startDate === hoveredPoint.bucket.endDate
                  ? hoveredPoint.bucket.startDate
                  : `${hoveredPoint.bucket.startDate} → ${hoveredPoint.bucket.endDate}`}
              </strong>
              <div>
                <span>
                  {t('antigravityTokenUsage.cards.total', 'Total Tokens')}
                </span>
                <b>
                  {formatSessionUsageTokensShort(
                    hoveredPoint.bucket.exact.totalTokens,
                    lang,
                    2,
                  )}
                </b>
              </div>
              <div>
                <span>{t('antigravityTokenUsage.cards.input', 'Input Tokens')}</span>
                <b>
                  {formatSessionUsageTokensShort(
                    hoveredPoint.bucket.exact.inputTokens,
                    lang,
                    1,
                  )}
                </b>
              </div>
              <div>
                <span>
                  {t('antigravityTokenUsage.cards.thinking', 'Thinking Tokens')}
                </span>
                <b>
                  {formatSessionUsageTokensShort(
                    hoveredPoint.bucket.exact.thinkingTokens,
                    lang,
                    1,
                  )}
                </b>
              </div>
              <div>
                <span>{t('antigravityTokenUsage.cards.output', 'Output Tokens')}</span>
                <b>
                  {formatSessionUsageTokensShort(
                    hoveredPoint.bucket.exact.outputTokens,
                    lang,
                    1,
                  )}
                </b>
              </div>
              <div>
                <span>{t('antigravityTokenUsage.cards.cache', 'Cache Tokens')}</span>
                <b>
                  {formatSessionUsageTokensShort(
                    hoveredPoint.bucket.exact.cacheReadTokens +
                      hoveredPoint.bucket.exact.cacheWriteTokens,
                    lang,
                    1,
                  )}
                </b>
              </div>
              <div>
                <span>
                  {t('antigravityTokenUsage.cards.turns', 'Turns')}
                </span>
                <b>{formatSessionUsageCount(hoveredPoint.bucket.turnCount)}</b>
              </div>
            </div>
          ) : null}
        </div>
      )}
    </section>
  );
}
