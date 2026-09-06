import { useId, useMemo, useState, type MouseEvent } from "react";
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
  >;
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

function formatBucketLabel(timestamp: number, hourly: boolean, lang: string): string {
  const date = new Date(timestamp);
  return hourly
    ? date.toLocaleTimeString(lang, { hour: "2-digit", minute: "2-digit" })
    : date.toLocaleDateString(lang, { month: "numeric", day: "numeric" });
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

export function CodexUsageTrend({
  statsWindow,
  points: suppliedPoints,
  title,
  emptyLabel,
  metrics = ["totalTokens", "requestCount", "estimatedCostUsd"],
  labels,
  hourly: hourlyOverride,
}: {
  statsWindow?: CodexLocalAccessStatsWindow | null;
  points?: UsageTrendPoint[];
  title?: string;
  emptyLabel?: string;
  metrics?: UsageTrendMetric[];
  labels?: Partial<Record<UsageTrendMetric, string>>;
  hourly?: boolean;
}) {
  const { t, i18n } = useTranslation();
  const lang = i18n.resolvedLanguage || i18n.language || "zh-CN";
  const chartId = useId().replace(/:/g, "");
  const [metric, setMetric] = useState<UsageTrendMetric>("totalTokens");
  const [hoveredIndex, setHoveredIndex] = useState<number | null>(null);
  const trend = suppliedPoints ?? statsWindow?.trend ?? [];
  const hourly = hourlyOverride ?? statsWindow?.trendHourly ?? false;

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
    const xSpan = CHART_WIDTH - PLOT.left - PLOT.right;
    const ySpan = CHART_HEIGHT - PLOT.top - PLOT.bottom;
    return values.map((item, index) => ({
      ...item,
      index,
      x: PLOT.left + (values.length === 1 ? xSpan / 2 : (index / (values.length - 1)) * xSpan),
      y: PLOT.top + (1 - (item.value - min + span * 0.03) / (span * 1.06)) * ySpan,
    }));
  }, [metric, trend]);

  const linePath = buildSmoothPath(points);
  const areaPath = points.length > 0
    ? `${linePath} L ${points[points.length - 1].x.toFixed(2)} ${(CHART_HEIGHT - PLOT.bottom).toFixed(2)} L ${points[0].x.toFixed(2)} ${(CHART_HEIGHT - PLOT.bottom).toFixed(2)} Z`
    : "";
  const maxValue = points.length > 0 ? Math.max(...points.map((point) => point.value)) : 0;
  const totalValue = points.reduce((sum, point) => sum + point.value, 0);
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
        CHART_WIDTH - PLOT.right - width / 2,
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
  }, [lang, metric, points]);
  const gridYs = [0, 1, 2].map(
    (index) => PLOT.top + (index / 2) * (CHART_HEIGHT - PLOT.top - PLOT.bottom),
  );
  const hoveredPoint = hoveredIndex === null ? null : points[hoveredIndex] ?? null;
  const tooltipX = hoveredPoint
    ? Math.min(CHART_WIDTH - 132, Math.max(PLOT.left + 132, hoveredPoint.x))
    : 0;
  const tooltipStyle = hoveredPoint
    ? { left: `${(tooltipX / CHART_WIDTH) * 100}%` }
    : undefined;
  const handleChartMove = (event: MouseEvent<SVGRectElement>) => {
    const svg = event.currentTarget.ownerSVGElement;
    if (!svg || points.length === 0) return;
    const bounds = svg.getBoundingClientRect();
    const relativeX = ((event.clientX - bounds.left) / bounds.width) * CHART_WIDTH;
    const fraction = (relativeX - PLOT.left) / (CHART_WIDTH - PLOT.left - PLOT.right);
    setHoveredIndex(Math.round(Math.max(0, Math.min(1, fraction)) * (points.length - 1)));
  };

  return (
    <section className="codex-usage-trend" aria-labelledby="codex-usage-trend-title">
      <div className="codex-usage-trend__header">
        <div>
          <h2 id="codex-usage-trend-title">
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
        <div className="codex-usage-trend__chart-wrap">
          <svg
            className="codex-usage-trend__chart"
            viewBox={`0 0 ${CHART_WIDTH} ${CHART_HEIGHT}`}
            role="img"
            aria-label={`${title ?? t("codex.apiService.usage.title", "用量统计")} · ${formatMetricValue(maxValue, metric, lang)}`}
          >
            <title>{title ?? t("codex.apiService.usage.title", "用量统计")}</title>
            <defs>
              <linearGradient id={`${chartId}-area`} x1="0" y1="0" x2="0" y2="1">
                <stop offset="0%" stopColor="var(--primary)" stopOpacity="0.26" />
                <stop offset="100%" stopColor="var(--primary)" stopOpacity="0.02" />
              </linearGradient>
            </defs>
            {gridYs.map((y) => (
              <line key={y} className="codex-usage-trend__grid" x1={PLOT.left} x2={CHART_WIDTH - PLOT.right} y1={y} y2={y} />
            ))}
            <line className="codex-usage-trend__baseline" x1={PLOT.left} x2={CHART_WIDTH - PLOT.right} y1={CHART_HEIGHT - PLOT.bottom} y2={CHART_HEIGHT - PLOT.bottom} />
            {areaPath ? <path className="codex-usage-trend__area" d={areaPath} fill={`url(#${chartId}-area)`} /> : null}
            {linePath ? <path className="codex-usage-trend__line" d={linePath} /> : null}
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
                    <title>{`${formatBucketLabel(point.point.bucketStart, hourly, lang)} · ${formatMetricValue(point.value, metric, lang)}`}</title>
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
                    {formatBucketLabel(point.point.bucketStart, hourly, lang)}
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
            <div className="codex-usage-trend__tooltip" role="status" style={tooltipStyle}>
              <strong>{formatBucketLabel(hoveredPoint.point.bucketStart, hourly, lang)}</strong>
              <div>
                <span>{labels?.totalTokens ?? t("codex.localAccess.stats.tokens", "总 Token 数")}</span>
                <b>{formatMetricValue(hoveredPoint.point.usage.totalTokens, "totalTokens", lang)}</b>
              </div>
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
      )}
    </section>
  );
}
