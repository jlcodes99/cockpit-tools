import { useId, useLayoutEffect, useMemo, useRef, useState, type MouseEvent } from "react";
import { useTranslation } from "react-i18next";
import type { CodexLocalAccessStatsWindow } from "../../types/codexLocalAccess";
import { formatSessionUsageTokensShort } from "../../utils/codexSessionUsageFormat";
import "./CodexUsageTrend.css";

type UsageTrendMetric = "totalTokens" | "requestCount" | "estimatedCostUsd";

type UsageTrendLabel = {
  text: string;
  x: number;
};

export type UsageTrendPoint = {
  bucketStart: number;
  usage: Pick<
    CodexLocalAccessStatsWindow["totals"],
    "totalTokens" | "requestCount" | "estimatedCostUsd"
  > & Partial<Pick<CodexLocalAccessStatsWindow["totals"], "inputTokens" | "outputTokens" | "reasoningTokens">>;
};

const CHART_WIDTH = 960;
const CHART_HEIGHT = 286;
const PLOT = { left: 58, right: 18, top: 28, bottom: 58 };

function formatCost(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return "$0";
  if (value < 0.01) return `$${value.toFixed(4)}`;
  return `$${value.toLocaleString(undefined, { maximumFractionDigits: 2 })}`;
}

function formatMetricValue(value: number, metric: UsageTrendMetric, lang: string): string {
  if (metric === "requestCount") return value.toLocaleString(lang);
  if (metric === "estimatedCostUsd") return formatCost(value);
  return formatSessionUsageTokensShort(value, lang, 2);
}

function formatBucketLabel(timestamp: number, hourly: boolean, lang: string, timeZone?: string): string {
  const date = new Date(timestamp);
  return hourly
    ? date.toLocaleTimeString(lang, { hour: "2-digit", minute: "2-digit", timeZone })
    : date.toLocaleDateString(lang, { month: "numeric", day: "numeric", timeZone });
}

function buildSmoothPath(points: Array<{ x: number; y: number }>): string {
  if (points.length === 0) return "";
  if (points.length === 1) return `M ${points[0].x.toFixed(2)} ${points[0].y.toFixed(2)}`;
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

function tokenComposition(points: UsageTrendPoint[]) {
  let input = 0;
  let output = 0;
  let reasoning = 0;
  let hasReasoning = true;
  for (const { usage } of points) {
    const { inputTokens, outputTokens, totalTokens, reasoningTokens } = usage;
    if (totalTokens === 0 && !inputTokens && !outputTokens && !reasoningTokens) continue;
    if (inputTokens == null || outputTokens == null || inputTokens < 0 || outputTokens < 0
      || inputTokens + outputTokens !== totalTokens) return null;
    input += inputTokens;
    output += outputTokens;
    if (reasoningTokens == null || reasoningTokens < 0 || reasoningTokens > outputTokens) hasReasoning = false;
    else reasoning += reasoningTokens;
  }
  if (input + output === 0) return null;
  return { input, output: hasReasoning ? output - reasoning : output, reasoning: hasReasoning ? reasoning : null };
}

export function CodexUsageTrend({
  statsWindow,
  points: suppliedPoints,
  title,
  emptyLabel,
  metrics = ["totalTokens", "requestCount", "estimatedCostUsd"],
  labels,
  hourly: hourlyOverride,
  timeZone,
}: {
  statsWindow?: CodexLocalAccessStatsWindow | null;
  points?: UsageTrendPoint[];
  title?: string;
  emptyLabel?: string;
  metrics?: UsageTrendMetric[];
  labels?: Partial<Record<UsageTrendMetric, string>>;
  hourly?: boolean;
  timeZone?: string;
}) {
  const { t, i18n } = useTranslation();
  const lang = i18n.resolvedLanguage || i18n.language || "zh-CN";
  const chartId = useId().replace(/:/g, "");
  const [metric, setMetric] = useState<UsageTrendMetric>("totalTokens");
  const [hoveredIndex, setHoveredIndex] = useState<number | null>(null);
  const svgRef = useRef<SVGSVGElement>(null);
  const chartWrapRef = useRef<HTMLDivElement>(null);
  const tooltipRef = useRef<HTMLDivElement>(null);
  const [tooltipLeft, setTooltipLeft] = useState(0);
  const [chartWidth, setChartWidth] = useState(CHART_WIDTH);
  const trend = suppliedPoints ?? statsWindow?.trend ?? [];
  const hasTrend = trend.length > 0;
  const hourly = hourlyOverride ?? statsWindow?.trendHourly ?? false;
  const tokenMetric = metric === "totalTokens";
  const composition = useMemo(() => tokenComposition(trend), [trend]);
  const periodTokensLabel = t("codex.apiService.usage.periodTokens", "时段 Token 用量");
  const cumulativeTokensLabel = t("codex.apiService.usage.cumulativeTokens", "累计 Token 用量");
  const compositionLabel = t("codex.apiService.usage.tokenComposition", "Token 用量构成");
  const plotBottom = CHART_HEIGHT - PLOT.bottom;
  const plotHeight = plotBottom - PLOT.top;
  const plotRight = chartWidth - PLOT.right;
  const plotWidth = plotRight - PLOT.left;
  const firstBucket = trend[0]?.bucketStart ?? 0;
  const elapsed = (trend[trend.length - 1]?.bucketStart ?? firstBucket) - firstBucket;

  useLayoutEffect(() => {
    const wrap = chartWrapRef.current;
    if (!hasTrend || !wrap) return;
    const resize = () => setChartWidth(Math.max(300, Math.min(wrap.clientWidth, CHART_WIDTH)));
    resize();
    const observer = new ResizeObserver(resize);
    observer.observe(wrap);
    return () => observer.disconnect();
  }, [hasTrend]);

  const points = useMemo(() => {
    const values = trend.map((point) => ({
      point,
      value:
        metric === "requestCount"
          ? point.usage.requestCount
          : metric === "estimatedCostUsd"
            ? point.usage.estimatedCostUsd
            : point.usage.totalTokens,
    }));
    if (values.length === 0) return [];
    const min = Math.min(...values.map((item) => item.value));
    const max = Math.max(...values.map((item) => item.value));
    const span = Math.max(max - min, max * 0.08, 1);
    let cumulative = 0;
    return values.map((item, index) => {
      cumulative += item.value;
      const fraction = tokenMetric ? item.value / Math.max(1, max) : (item.value - min + span * 0.03) / (span * 1.06);
      return {
        ...item, index, cumulative,
        x: PLOT.left + (elapsed > 0 ? (item.point.bucketStart - firstBucket) / elapsed : 0.5) * plotWidth,
        y: PLOT.top + (1 - fraction) * plotHeight,
      };
    });
  }, [metric, tokenMetric, trend, elapsed, firstBucket, plotWidth, plotHeight]);

  const linePath = tokenMetric ? "" : buildSmoothPath(points);
  const areaPath = points.length > 0
    ? `${linePath} L ${points[points.length - 1].x.toFixed(2)} ${(CHART_HEIGHT - PLOT.bottom).toFixed(2)} L ${points[0].x.toFixed(2)} ${(CHART_HEIGHT - PLOT.bottom).toFixed(2)} Z`
    : "";
  const maxValue = points.length > 0 ? Math.max(...points.map((point) => point.value)) : 0;
  const totalValue = points[points.length - 1]?.cumulative ?? 0;
  const cumulativeY = (value: number) => plotBottom - value / Math.max(1, totalValue) * plotHeight;
  const cumulativeLine = points.map((point, index) => index === 0
    ? `M ${point.x} ${cumulativeY(point.cumulative)}`
    : `H ${point.x} V ${cumulativeY(point.cumulative)}`).join(" ");
  const intervalCount = 1 + elapsed / (hourly ? 3_600_000 : 86_400_000);
  const barWidth = Math.min(22, plotWidth / intervalCount * 0.55);
  const slices = !tokenMetric || !composition ? [] : [
    { label: t("common.codexUsage.input"), value: composition.input, color: "var(--primary)" },
    { label: composition.reasoning === null ? t("common.codexUsage.output") : t("codex.apiService.usage.nonReasoningOutput", "非推理输出"),
      value: composition.output, color: "var(--success)" },
    ...(composition.reasoning === null ? [] : [{ label: t("codex.apiService.usage.reasoningTokens", "推理"), value: composition.reasoning, color: "var(--warning)" }]),
  ];
  const percent = new Intl.NumberFormat(lang, { style: "percent", maximumFractionDigits: 1 });
  let sliceStart = 0;
  const compositionGradient = slices.map(({ value, color }) => {
    const sliceEnd = sliceStart + value / totalValue * 100;
    const stop = `${color} ${sliceStart}% ${sliceEnd}%`;
    sliceStart = sliceEnd;
    return stop;
  }).join(", ");
  const axisStep = Math.max(1, Math.ceil(points.length / 7));
  const axisIndexes = new Set(
    points
      .filter((point) => point.index % axisStep === 0 || point.index === points.length - 1)
      .map((point) => point.index),
  );
  const valueLabels = useMemo<Map<number, UsageTrendLabel>>(() => {
    const occupiedRanges: Array<{ start: number; end: number }> = [];
    const labels = new Map<number, UsageTrendLabel>();
    const candidates = points
      .filter((point) => point.value > 0)
      .sort((left, right) => right.value - left.value || left.index - right.index);

    candidates.forEach((point) => {
      const text = formatMetricValue(point.value, metric, lang);
      const width = Math.max(42, Math.min(112, text.length * 7 + 16));
      const x = Math.min(
        plotRight - width / 2,
        Math.max(PLOT.left + width / 2, point.x),
      );
      const start = x - width / 2 - 4;
      const end = x + width / 2 + 4;
      if (occupiedRanges.some((range) => start < range.end && end > range.start)) {
        return;
      }
      occupiedRanges.push({ start, end });
      labels.set(point.index, { text, x });
    });
    return labels;
  }, [lang, metric, points, plotRight]);
  const gridYs = [0, 1, 2].map(
    (index) => PLOT.top + (index / 2) * (CHART_HEIGHT - PLOT.top - PLOT.bottom),
  );
  const hoveredPoint = hoveredIndex === null ? null : points[hoveredIndex] ?? null;
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
      // Project into the HTML container, including SVG letterboxing and UI zoom.
      const localX = (point.matrixTransform(matrix).x - bounds.left) * wrap.clientWidth / bounds.width;
      const halfWidth = (tooltipRef.current?.offsetWidth ?? 0) / 2 + 8;
      setTooltipLeft(Math.max(halfWidth, Math.min(wrap.clientWidth - halfWidth, localX)));
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
    const nearest = points.reduce((best, point) =>
      Math.abs(point.x - relativeX) < Math.abs(best.x - relativeX) ? point : best);
    setHoveredIndex(nearest.index);
  };

  return (
    <section className="codex-usage-trend" aria-labelledby={`${chartId}-title`}>
      <div className="codex-usage-trend__header">
        <div>
          <h2 id={`${chartId}-title`}>
            {title ?? t("codex.apiService.usage.title", "用量统计")}
          </h2>
        </div>
        <div className="codex-usage-trend__header-right">
          <strong className="codex-usage-trend__total">
            {formatMetricValue(totalValue, metric, lang)}
          </strong>
          <div className="codex-usage-trend__metrics" role="group" aria-label={t("codex.apiService.usage.title", "用量统计")}>
          {([
            ["totalTokens", labels?.totalTokens ?? t("codex.localAccess.stats.tokens", "总 Token 数")],
            ["requestCount", labels?.requestCount ?? t("codex.localAccess.stats.requests", "总请求数")],
            ["estimatedCostUsd", labels?.estimatedCostUsd ?? t("codex.localAccess.stats.estimatedCost", "估算价值")],
          ] as const).filter(([value]) => metrics.includes(value)).map(([value, label]) => (
            <button
              key={value}
              type="button"
              className={`btn btn-secondary btn-sm${metric === value ? " is-active" : ""}`}
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
          {emptyLabel ?? t("codex.apiService.usage.trendEmpty", "所选时间范围暂无请求记录")}
        </div>
      ) : (
        <div className="codex-usage-trend__body">
          <div ref={chartWrapRef} className="codex-usage-trend__chart-wrap">
            {tokenMetric && (
              <div className="codex-usage-trend__legend">
                <span>{periodTokensLabel}</span><span>{cumulativeTokensLabel}</span>
              </div>
            )}
            <svg
              ref={svgRef}
              className="codex-usage-trend__chart"
              viewBox={`0 0 ${chartWidth} ${CHART_HEIGHT}`}
              role="img"
              aria-label={tokenMetric
                ? `${periodTokensLabel} · ${cumulativeTokensLabel} ${formatMetricValue(totalValue, metric, lang)}`
                : `${title ?? t("codex.apiService.usage.title", "用量统计")} · ${formatMetricValue(maxValue, metric, lang)}`}
            >
              <title>{title ?? t("codex.apiService.usage.title", "用量统计")}</title>
              <defs>
                <linearGradient id={`${chartId}-area`} x1="0" y1="0" x2="0" y2="1">
                  <stop offset="0%" stopColor="var(--primary)" stopOpacity="0.26" />
                  <stop offset="100%" stopColor="var(--primary)" stopOpacity="0.02" />
                </linearGradient>
              </defs>
              {gridYs.map((y) => (
                <line key={y} className="codex-usage-trend__grid" x1={PLOT.left} x2={plotRight} y1={y} y2={y} />
              ))}
              <line className="codex-usage-trend__baseline" x1={PLOT.left} x2={plotRight} y1={plotBottom} y2={plotBottom} />
              {tokenMetric ? (
                <>
                  <text className="codex-usage-trend__period-scale" x={PLOT.left} y={PLOT.top - 9}>
                    {formatMetricValue(maxValue, metric, lang)}
                  </text>
                  <text className="codex-usage-trend__cumulative-scale" x={plotRight} y={PLOT.top - 9} textAnchor="end">
                    {formatMetricValue(totalValue, metric, lang)}
                  </text>
                  {points.map((point) => <rect key={point.point.bucketStart} className="codex-usage-trend__period-bar"
                    x={point.x - barWidth / 2} y={point.y} width={barWidth} height={plotBottom - point.y} rx="2" />)}
                  {points.length === 1 ? (
                    <line className="codex-usage-trend__cumulative-line" x1={points[0].x - barWidth / 2} x2={points[0].x + barWidth / 2}
                      y1={cumulativeY(totalValue)} y2={cumulativeY(totalValue)} />
                  ) : <path className="codex-usage-trend__cumulative-line" d={cumulativeLine} />}
                </>
              ) : (
                <>
                  <path className="codex-usage-trend__area" d={areaPath} fill={`url(#${chartId}-area)`} />
                  <path className="codex-usage-trend__line" d={linePath} />
                </>
              )}
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
                width={plotWidth}
                height={CHART_HEIGHT - PLOT.top - PLOT.bottom}
                onMouseMove={handleChartMove}
                onMouseLeave={() => setHoveredIndex(null)}
              />
              {points.map((point) => (
                <g key={point.point.bucketStart}>
                  {point.value > 0 ? (
                    <circle
                      className={`codex-usage-trend__point${hoveredIndex === point.index ? " is-hovered" : ""}`}
                      cx={point.x}
                      cy={point.y}
                      r={point.value === maxValue || hoveredIndex === point.index ? 5 : 3.5}
                      tabIndex={0}
                      onMouseEnter={() => setHoveredIndex(point.index)}
                      onFocus={() => setHoveredIndex(point.index)}
                      onBlur={() => setHoveredIndex(null)}
                    >
                      <title>{`${formatBucketLabel(point.point.bucketStart, hourly, lang, timeZone)} · ${formatMetricValue(point.value, metric, lang)}${tokenMetric ? ` · ${cumulativeTokensLabel} ${formatMetricValue(point.cumulative, metric, lang)}` : ""}`}</title>
                    </circle>
                  ) : null}
                  {valueLabels.has(point.index) ? (
                    (() => {
                      const label = valueLabels.get(point.index);
                      if (!label) return null;
                      const labelWidth = Math.max(42, Math.min(112, label.text.length * 7 + 16));
                      const labelY = Math.max(PLOT.top + 3, point.y - 29);
                      return (
                        <g className="codex-usage-trend__value-label">
                          <rect x={label.x - labelWidth / 2} y={labelY} width={labelWidth} height="20" rx="7" />
                          <text className="codex-usage-trend__value" x={label.x} y={labelY + 14} textAnchor="middle">
                            {label.text}
                          </text>
                        </g>
                      );
                    })()
                  ) : null}
                  {axisIndexes.has(point.index) ? (
                    <text className="codex-usage-trend__axis-label" x={point.x} y={CHART_HEIGHT - 21} textAnchor="middle">
                      {formatBucketLabel(point.point.bucketStart, hourly, lang, timeZone)}
                    </text>
                  ) : null}
                  {axisIndexes.has(point.index) ? (
                    <text className="codex-usage-trend__axis-value" x={point.x} y={CHART_HEIGHT - 4} textAnchor="middle">
                      {formatMetricValue(point.value, metric, lang)}
                    </text>
                  ) : null}
                </g>
              ))}
            </svg>
            {hoveredPoint ? (
              <div ref={tooltipRef} className="codex-usage-trend__tooltip" role="status" style={{ left: tooltipLeft }}>
                <strong>{formatBucketLabel(hoveredPoint.point.bucketStart, hourly, lang, timeZone)}</strong>
                <div>
                  <span>{labels?.totalTokens ?? t("codex.localAccess.stats.tokens", "总 Token 数")}</span>
                  <b>{formatMetricValue(hoveredPoint.point.usage.totalTokens, "totalTokens", lang)}</b>
                </div>
                {tokenMetric && <div>
                  <span>{cumulativeTokensLabel}</span>
                  <b>{formatMetricValue(hoveredPoint.cumulative, metric, lang)}</b>
                </div>}
                <div>
                  <span>{labels?.requestCount ?? t("codex.localAccess.stats.requests", "总请求数")}</span>
                  <b>{formatMetricValue(hoveredPoint.point.usage.requestCount, "requestCount", lang)}</b>
                </div>
                {metrics.includes("estimatedCostUsd") ? (
                  <div>
                    <span>{labels?.estimatedCostUsd ?? t("codex.localAccess.stats.estimatedCost", "估算价值")}</span>
                    <b>{formatMetricValue(hoveredPoint.point.usage.estimatedCostUsd, "estimatedCostUsd", lang)}</b>
                  </div>
                ) : null}
              </div>
            ) : null}
          </div>
          {tokenMetric && slices.length > 0 && (
            <div className="codex-usage-trend__composition" role="group" aria-label={compositionLabel}>
              <div className="codex-usage-trend__ring" style={{ background: `conic-gradient(${compositionGradient})` }} aria-hidden="true">
                <strong>{formatMetricValue(totalValue, metric, lang)}</strong>
              </div>
              <ul>
                {slices.map(({ label, value, color }) => (
                  <li key={label}><i aria-hidden="true" style={{ background: color }} /><span>{label}</span>
                    <b title={value.toLocaleString(lang)}>{percent.format(value / totalValue)}</b></li>
                ))}
              </ul>
            </div>
          )}
        </div>
      )}
    </section>
  );
}
