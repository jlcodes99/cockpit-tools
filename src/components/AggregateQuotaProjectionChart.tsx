import React, { useState, useMemo, useRef, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { History } from 'lucide-react';
import {
  buildAntigravityAccountPresentation,
  buildClaudeAccountPresentation,
  buildGeminiAccountPresentation,
  buildCodexAccountPresentation,
  buildCursorAccountPresentation,
  buildWindsurfAccountPresentation,
} from '../presentation/platformAccountPresentation';
import './AggregateQuotaProjectionChart.css';

export interface AggregateQuotaProjectionChartProps {
  accounts: any[];
  platform: 'antigravity' | 'claude' | 'gemini' | 'codex' | 'cursor' | 'windsurf';
  renderPanel?: boolean;
}

interface NormalizedQuotaItem {
  email: string;
  cycle: 'short_term' | 'long_term';
  family: string;
  familyLabel: string;
  tier: string;
  percentage: number; // remaining percentage (0 to 100)
  resetDate?: Date;
}

export const AggregateQuotaProjectionChart: React.FC<AggregateQuotaProjectionChartProps> = ({
  accounts,
  platform,
  renderPanel = true,
}) => {
  const { t } = useTranslation();
  const [selectedCycle, setSelectedCycle] = useState<'short_term' | 'long_term'>('long_term');
  const [hiddenSeriesIds, setHiddenSeriesIds] = useState<string[]>([]);
  const [hoveredPoint, setHoveredPoint] = useState<any>(null);
  const [hoverIdx, setHoverIdx] = useState<number>(-1);
  const svgRef = useRef<SVGSVGElement>(null);

  // Persistence of panel collapse state per platform
  const storageKey = `agtools.${platform}.show_aggregate_projection`;
  const [showAggregateProjection, setShowAggregateProjection] = useState<boolean>(() => {
    try {
      const val = localStorage.getItem(storageKey);
      return val !== 'false';
    } catch {
      return true;
    }
  });

  const toggleProjection = () => {
    const newVal = !showAggregateProjection;
    setShowAggregateProjection(newVal);
    try {
      localStorage.setItem(storageKey, String(newVal));
    } catch (e) {
      console.error('Failed to save projection toggle state', e);
    }
  };

  const toggleSeries = (id: string) => {
    setHiddenSeriesIds((prev) =>
      prev.includes(id) ? prev.filter((x) => x !== id) : [...prev, id]
    );
  };

  // 1. Filter out disabled/forbidden accounts and build presentations
  const normalizedAccounts = useMemo(() => {
    return accounts
      .filter((acc) => acc && !acc.disabled && !acc.quota?.is_forbidden)
      .map((acc) => {
        try {
          switch (platform) {
            case 'antigravity':
              return buildAntigravityAccountPresentation(acc, [], t);
            case 'claude':
              return buildClaudeAccountPresentation(acc, t);
            case 'gemini':
              return buildGeminiAccountPresentation(acc, t);
            case 'codex':
              return buildCodexAccountPresentation(acc, t);
            case 'cursor':
              return buildCursorAccountPresentation(acc, t);
            case 'windsurf':
              return buildWindsurfAccountPresentation(acc, t);
            default:
              return null;
          }
        } catch (e) {
          console.error(`Failed to build presentation for ${platform} account`, e);
          return null;
        }
      })
      .filter((p): p is NonNullable<typeof p> => p !== null);
  }, [accounts, platform, t]);

  // Helpers for classifying cycle and model family
  const getResetCycle = (item: any): 'short_term' | 'long_term' => {
    const key = (item.key || '').toLowerCase();
    const label = (item.label || '').toLowerCase();
    if (
      key.includes('hourly') ||
      key.includes('5h') ||
      key.includes('daily') ||
      key.includes('session') ||
      key === 'primary' || // Codex primary is 5-hour
      label.includes('hourly') ||
      label.includes('5h') ||
      label.includes('5-hour') ||
      label.includes('session') ||
      label.includes('daily') ||
      label.includes('day limit')
    ) {
      return 'short_term';
    }
    return 'long_term';
  };

  const getModelFamily = (item: any, platformId: string): string => {
    const key = (item.key || '').toLowerCase();
    const label = (item.label || '').toLowerCase();
    if (key.includes('claude') || label.includes('claude')) return 'claude';
    if (key.includes('gemini') || label.includes('gemini')) return 'gemini';
    if (key.includes('openai') || key.includes('gpt') || key.includes('codex') || label.includes('openai') || label.includes('gpt')) return 'openai';
    if (platformId === 'cursor') return 'cursor';
    if (platformId === 'windsurf') return 'windsurf';
    if (platformId === 'codex') return 'openai';
    if (platformId === 'claude') return 'claude';
    if (platformId === 'gemini') return 'gemini';
    return 'unified';
  };

  const getFamilyLabel = (family: string): string => {
    switch (family) {
      case 'claude': return 'Claude';
      case 'gemini': return 'Gemini';
      case 'openai': return 'OpenAI / Codex';
      case 'cursor': return 'Cursor';
      case 'windsurf': return 'Windsurf';
      default: return t('accounts.aggregate.quota', 'Quota');
    }
  };

  // Robust Date Parsing
  const parseResetDate = (resetAt: any): Date | undefined => {
    if (!resetAt) return undefined;
    if (typeof resetAt === 'number') {
      const ms = resetAt > 10_000_000_000 ? resetAt : resetAt * 1000;
      return new Date(ms);
    }
    if (typeof resetAt === 'string') {
      const parsed = new Date(resetAt);
      return Number.isNaN(parsed.getTime()) ? undefined : parsed;
    }
    if (resetAt instanceof Date) {
      return Number.isNaN(resetAt.getTime()) ? undefined : resetAt;
    }
    return undefined;
  };

  // Used vs Remaining platform mapping
  const getRemainingPercent = (percentage: number): number => {
    const clamped = Math.max(0, Math.min(100, percentage));
    const isUsedPlat = ['claude', 'cursor', 'windsurf'].includes(platform);
    return isUsedPlat ? 100 - clamped : clamped;
  };

  // Extract all quota items across active accounts
  const allQuotaItems = useMemo(() => {
    const itemsList: NormalizedQuotaItem[] = [];
    normalizedAccounts.forEach((acc) => {
      const tier = (acc.planLabel || 'FREE').toString().trim().toUpperCase();
      const email = acc.displayName || acc.id;
      acc.quotaItems?.forEach((item) => {
        const cycle = getResetCycle(item);
        const family = getModelFamily(item, platform);
        const familyLabel = getFamilyLabel(family);
        const percentage = getRemainingPercent(item.percentage);
        const resetDate = parseResetDate(item.resetAt);

        itemsList.push({
          email,
          cycle,
          family,
          familyLabel,
          tier,
          percentage,
          resetDate,
        });
      });
    });
    return itemsList;
  }, [normalizedAccounts, platform]);

  // Determine available cycles
  const availableCycles = useMemo(() => {
    const cycles = new Set<'short_term' | 'long_term'>();
    allQuotaItems.forEach((item) => cycles.add(item.cycle));
    return Array.from(cycles);
  }, [allQuotaItems]);

  // Default to long_term if available, otherwise fallback
  useEffect(() => {
    if (availableCycles.length > 0 && !availableCycles.includes(selectedCycle)) {
      setSelectedCycle(availableCycles[0]);
    }
  }, [availableCycles, selectedCycle]);

  // Filter items to selected cycle
  const currentCycleItems = useMemo(() => {
    return allQuotaItems.filter((item) => item.cycle === selectedCycle);
  }, [allQuotaItems, selectedCycle]);

  // Determine time horizon dynamically
  const horizonHours = useMemo(() => {
    let maxResetHours = 0;
    const now = new Date();
    currentCycleItems.forEach((item) => {
      if (item.resetDate) {
        const diffHours = (item.resetDate.getTime() - now.getTime()) / (60 * 60 * 1000);
        if (diffHours > maxResetHours) {
          maxResetHours = diffHours;
        }
      }
    });

    if (maxResetHours > 0) {
      if (selectedCycle === 'short_term') {
        return Math.max(5, Math.ceil(maxResetHours) + 1); // Minimum 5h, or furthest reset + 1h buffer
      } else {
        const days = Math.ceil(maxResetHours / 24);
        return Math.max(168, (days + 1) * 24); // Minimum 7d (168h), or furthest reset rounded up to whole days + 24h buffer
      }
    } else {
      return selectedCycle === 'short_term' ? 5 : 168; // 5 hours for short-term, 7 days for long-term if no future resets
    }
  }, [currentCycleItems, selectedCycle]);

  // Brand colors & styles for series based on exact tier
  const getTierStyle = (family: string, tier: string): { color: string; isDashed: boolean } => {
    const norm = tier.toUpperCase();
    const isFree = norm === 'FREE' || norm === 'STANDARD' || norm === 'UNKNOWN' || norm === '';
    const isUltra = norm.includes('ULTRA') || norm.includes('MAX') || norm.includes('TEAM') || norm.includes('ENTERPRISE');
    const isGo = norm.includes('GO') || norm.includes('PLUS');

    // Base colors
    let baseHex = '#7c3aed'; // default
    if (family === 'claude') baseHex = '#d97706';
    else if (family === 'gemini') baseHex = '#4f46e5';
    else if (family === 'openai') baseHex = '#059669';
    else if (family === 'cursor') baseHex = '#0891b2';
    else if (family === 'windsurf') baseHex = '#2563eb';

    if (isFree) {
      // Lighter color for Free
      let freeHex = '#8b5cf6';
      if (family === 'claude') freeHex = '#f59e0b';
      else if (family === 'gemini') freeHex = '#6366f1';
      else if (family === 'openai') freeHex = '#10b981';
      else if (family === 'cursor') freeHex = '#06b6d4';
      else if (family === 'windsurf') freeHex = '#3b82f6';
      return { color: freeHex, isDashed: true };
    }

    if (isUltra) {
      // Darker/vibrant color for Ultra
      let ultraHex = '#5b21b6';
      if (family === 'claude') ultraHex = '#92400e';
      else if (family === 'gemini') ultraHex = '#3730a3';
      else if (family === 'openai') ultraHex = '#065f46';
      else if (family === 'cursor') ultraHex = '#075985';
      else if (family === 'windsurf') ultraHex = '#1e40af';
      return { color: ultraHex, isDashed: false };
    }

    if (isGo) {
      // Intermediate color for Go / Plus
      let goHex = '#6d28d9';
      if (family === 'claude') goHex = '#b45309';
      else if (family === 'gemini') goHex = '#4338ca';
      else if (family === 'openai') goHex = '#047857';
      else if (family === 'cursor') goHex = '#0e7490';
      else if (family === 'windsurf') goHex = '#1d4ed8';
      return { color: goHex, isDashed: false };
    }

    return { color: baseHex, isDashed: false };
  };

  // Group items into separate overlapping series
  const projectionData = useMemo(() => {
    if (currentCycleItems.length === 0) {
      return { series: [], resetEvents: [] };
    }

    const now = new Date();
    const horizonEnd = new Date(now.getTime() + horizonHours * 60 * 60 * 1000);

    // Group items by series (family + tier combination)
    const seriesGroups: Record<string, {
      id: string;
      family: string;
      tier: string;
      label: string;
      color: string;
      isDashed: boolean;
      items: NormalizedQuotaItem[];
    }> = {};

    currentCycleItems.forEach((item) => {
      const seriesId = `${item.family}-${item.tier}`;
      if (!seriesGroups[seriesId]) {
        const { color, isDashed } = getTierStyle(item.family, item.tier);
        const seriesLabel = `${item.familyLabel} (${item.tier})`;
        seriesGroups[seriesId] = {
          id: seriesId,
          family: item.family,
          tier: item.tier,
          label: seriesLabel,
          color,
          isDashed,
          items: [],
        };
      }
      seriesGroups[seriesId].items.push(item);
    });

    const activeSeriesList = Object.values(seriesGroups);

    const NUM_POINTS = 150;
    const stepSizeHours = horizonHours / NUM_POINTS;

    // Compute timeline points for each series
    const seriesWithPoints = activeSeriesList.map((ser) => {
      const points: { time: Date; label: string; value: number }[] = [];

      for (let idx = 0; idx <= NUM_POINTS; idx++) {
        const offsetHours = idx * stepSizeHours;
        const targetTime = new Date(now.getTime() + offsetHours * 60 * 60 * 1000);
        let totalValue = 0;

        ser.items.forEach((acc) => {
          if (acc.resetDate && targetTime.getTime() >= acc.resetDate.getTime()) {
            totalValue += 100;
          } else {
            totalValue += acc.percentage;
          }
        });

        const label = targetTime.toLocaleString(undefined, {
          month: 'short',
          day: 'numeric',
          hour: '2-digit',
          minute: '2-digit',
          hour12: false,
        });

        points.push({
          time: targetTime,
          label,
          value: totalValue,
        });
      }

      return {
        ...ser,
        points,
        activeAccountsCount: ser.items.length,
      };
    });

    // Collect all reset events for markers
    const resetEvents: {
      email: string;
      resetTime: Date;
      capacityIncrease: number;
      hoursFromNow: number;
      seriesLabel: string;
      family: string;
      tier: string;
    }[] = [];

    currentCycleItems.forEach((item) => {
      if (
        item.resetDate &&
        item.resetDate.getTime() > now.getTime() &&
        item.resetDate.getTime() < horizonEnd.getTime()
      ) {
        const capacityIncrease = 100 - item.percentage;
        if (capacityIncrease > 0) {
          const seriesLabel = `${item.familyLabel} (${item.tier})`;
          resetEvents.push({
            email: item.email,
            resetTime: item.resetDate,
            capacityIncrease,
            hoursFromNow: (item.resetDate.getTime() - now.getTime()) / (60 * 60 * 1000),
            seriesLabel,
            family: item.family,
            tier: item.tier,
          });
        }
      }
    });

    resetEvents.sort((a, b) => a.resetTime.getTime() - b.resetTime.getTime());

    return { series: seriesWithPoints, resetEvents };
  }, [currentCycleItems, horizonHours, t]);

  const { series, resetEvents } = projectionData;

  const visibleSeries = useMemo(() => {
    return series.filter((ser) => !hiddenSeriesIds.includes(ser.id));
  }, [series, hiddenSeriesIds]);

  // SVG parameters
  const width = 1000;
  const height = 150;
  const paddingLeft = 40;
  const paddingRight = 20;
  const paddingTop = 15;
  const paddingBottom = 25;
  const chartWidth = width - paddingLeft - paddingRight;
  const chartHeight = height - paddingTop - paddingBottom;

  // Max value calculation across visible series
  const maxVal = useMemo(() => {
    let max = 100;
    const targets = visibleSeries.length > 0 ? visibleSeries : series;
    targets.forEach((ser) => {
      const serMax = Math.max(...ser.points.map((p) => p.value), 100);
      if (serMax > max) max = serMax;
    });
    return Math.ceil(max / 100) * 100;
  }, [series, visibleSeries]);

  // Interactivity handlers
  const handleMouseMove = (e: React.MouseEvent<SVGSVGElement, MouseEvent>) => {
    if (!svgRef.current) return;
    const rect = svgRef.current.getBoundingClientRect();
    const mouseX = ((e.clientX - rect.left) / rect.width) * width;

    const relativeX = mouseX - paddingLeft;
    const pct = Math.max(0, Math.min(1, relativeX / chartWidth));
    const index = Math.round(pct * 150); // Map to 150 points

    if (series.length > 0 && series[0].points[index]) {
      const targetTime = series[0].points[index].time;
      const proximityThresholdMs = Math.max(15 * 60 * 1000, (horizonHours / 40) * 60 * 60 * 1000);

      // Only show reset markers for visible series
      const visibleFamilyKeys = new Set(visibleSeries.map((s) => s.family));
      const closeResets = resetEvents.filter((event) => {
        const diffMs = Math.abs(event.resetTime.getTime() - targetTime.getTime());
        return diffMs <= proximityThresholdMs && visibleFamilyKeys.has(event.family);
      });

      // Get values for all visible series at this hover point
      const seriesValues = visibleSeries.map((ser) => ({
        label: ser.label,
        value: ser.points[index].value,
        color: ser.color,
      }));

      setHoveredPoint({
        label: series[0].points[index].label,
        time: targetTime,
        resets: closeResets,
        seriesValues,
      });
      setHoverIdx(index);
    }
  };

  const handleMouseLeave = () => {
    setHoveredPoint(null);
    setHoverIdx(-1);
  };

  // Generate X-axis ticks (Now, +4h, etc. or +1d, etc.)
  const xTicks: { x: number; label: string }[] = [];
  let tickStepHours = 24;
  let tickLabelFn = (h: number) => `+${h / 24}d`;

  if (horizonHours <= 6) {
    tickStepHours = 1;
    tickLabelFn = (h: number) => `+${h}h`;
  } else if (horizonHours <= 12) {
    tickStepHours = 2;
    tickLabelFn = (h: number) => `+${h}h`;
  } else if (horizonHours <= 36) {
    tickStepHours = 4;
    tickLabelFn = (h: number) => `+${h}h`;
  } else if (horizonHours <= 168) {
    tickStepHours = 24;
    tickLabelFn = (h: number) => `+${h / 24}d`;
  } else if (horizonHours <= 336) {
    tickStepHours = 48; // every 2 days
    tickLabelFn = (h: number) => `+${h / 24}d`;
  } else if (horizonHours <= 720) {
    tickStepHours = 96; // every 4 days
    tickLabelFn = (h: number) => `+${h / 24}d`;
  } else {
    tickStepHours = 168; // every week
    tickLabelFn = (h: number) => `+${Math.round(h / 168)}w`;
  }

  const numTicks = horizonHours / tickStepHours;
  for (let i = 0; i <= numTicks; i++) {
    const hours = i * tickStepHours;
    const x = paddingLeft + (hours / horizonHours) * chartWidth;
    const label = i === 0 ? t('common.now', 'Now') : tickLabelFn(hours);
    xTicks.push({ x, label });
  }

  // Generate Y-axis ticks
  const yTicks: { y: number; val: number }[] = [];
  const steps = maxVal / 100;
  for (let i = 0; i <= steps; i++) {
    const val = i * 100;
    const y = paddingTop + chartHeight - (val / maxVal) * chartHeight;
    yTicks.push({ y, val });
  }

  const activeHoverPointCoord = hoverIdx !== -1 ? paddingLeft + (hoverIdx / 150) * chartWidth : null;

  const renderChartContent = () => {
    if (currentCycleItems.length === 0) {
      return (
        <div className="aggregate-empty-state">
          {t('accounts.aggregate.emptyState', 'No active accounts found for this metric cycle.')}
        </div>
      );
    }

    return (
      <div className="claude-aggregate-chart-container glass-card" style={{ display: 'flex', flexDirection: 'column', gap: '12px' }}>
        {/* Control Header & Cycle Switcher */}
        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', flexWrap: 'wrap', gap: '12px' }}>
          <div className="projection-legend">
            {series.map((ser) => {
              const isToggledOff = hiddenSeriesIds.includes(ser.id);
              return (
                <div 
                  key={ser.id} 
                  className={`legend-item family-${ser.family} ${isToggledOff ? 'toggled-off' : ''}`}
                  onClick={() => toggleSeries(ser.id)}
                  title={t('accounts.aggregate.toggleTooltip', 'Click to toggle visibility')}
                >
                  {isToggledOff ? (
                    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round" style={{ opacity: 0.6 }}>
                      <path d="M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19m-6.72-1.07a3 3 0 1 1-4.24-4.24" />
                      <line x1="1" y1="1" x2="23" y2="23" />
                    </svg>
                  ) : (
                    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
                      <path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z" />
                      <circle cx="12" cy="12" r="3" />
                    </svg>
                  )}
                  <span>
                    {ser.label} ({ser.activeAccountsCount})
                  </span>
                </div>
              );
            })}
          </div>
          <div>
            {availableCycles.length > 1 && (
              <div className="cycle-tabs">
                {availableCycles.includes('long_term') && (
                  <button
                    className={`cycle-tab-btn ${selectedCycle === 'long_term' ? 'active' : ''}`}
                    onClick={() => setSelectedCycle('long_term')}
                  >
                    {t('accounts.aggregate.longTerm', 'Long-term')}
                  </button>
                )}
                {availableCycles.includes('short_term') && (
                  <button
                    className={`cycle-tab-btn ${selectedCycle === 'short_term' ? 'active' : ''}`}
                    onClick={() => setSelectedCycle('short_term')}
                  >
                    {t('accounts.aggregate.shortTerm', 'Short-term')}
                  </button>
                )}
              </div>
            )}
          </div>
        </div>

        {/* SVG Graph Area */}
        <div style={{ position: 'relative' }}>
          <svg
            ref={svgRef}
            viewBox={`0 0 ${width} ${height}`}
            width="100%"
            height="100%"
            onMouseMove={handleMouseMove}
            onMouseLeave={handleMouseLeave}
            style={{ overflow: 'visible', userSelect: 'none' }}
          >
            {/* Gradients defs */}
            <defs>
              {series.map((ser) => (
                <linearGradient key={`grad-${ser.id}`} id={`grad-${ser.id}`} x1="0" y1="0" x2="0" y2="1">
                  <stop offset="0%" stopColor={ser.color} stopOpacity="0.15" />
                  <stop offset="100%" stopColor={ser.color} stopOpacity="0.00" />
                </linearGradient>
              ))}
            </defs>

            {/* Y Grid Ticks */}
            {yTicks.map((t, idx) => (
              <g key={idx}>
                <line
                  x1={paddingLeft}
                  y1={t.y}
                  x2={width - paddingRight}
                  y2={t.y}
                  stroke="var(--border)"
                  strokeDasharray="4 4"
                  strokeWidth="0.5"
                />
                <text
                  x={paddingLeft - 8}
                  y={t.y + 3}
                  textAnchor="end"
                  fontSize="9"
                  fill="var(--text-secondary)"
                  fontFamily="var(--font-mono)"
                >
                  {t.val}%
                </text>
              </g>
            ))}

            {/* X Grid Ticks */}
            {xTicks.map((t, idx) => (
              <g key={idx}>
                <line
                  x1={t.x}
                  y1={paddingTop}
                  x2={t.x}
                  y2={paddingTop + chartHeight}
                  stroke="var(--border)"
                  strokeWidth="0.5"
                  opacity={idx === 0 ? 1 : 0.5}
                />
                <text
                  x={t.x}
                  y={paddingTop + chartHeight + 14}
                  textAnchor="middle"
                  fontSize="9"
                  fill="var(--text-secondary)"
                  fontFamily="var(--font-mono)"
                >
                  {t.label}
                </text>
              </g>
            ))}

            {/* Reset Event Markers - Only show for visible series */}
            {resetEvents.map((event, idx) => {
              const isVisible = visibleSeries.some((s) => s.family === event.family && s.tier === event.tier);
              if (!isVisible) return null;

              const x = paddingLeft + (event.hoursFromNow / horizonHours) * chartWidth;
              const { color } = getTierStyle(event.family, event.tier);
              return (
                <g key={`reset-${idx}`}>
                  <line
                    x1={x}
                    y1={paddingTop}
                    x2={x}
                    y2={paddingTop + chartHeight}
                    stroke={color}
                    strokeWidth="1"
                    strokeDasharray="2 3"
                    opacity="0.6"
                  />
                  <circle
                    cx={x}
                    cy={paddingTop}
                    r="3.5"
                    fill={color}
                    stroke="var(--bg-primary)"
                    strokeWidth="1"
                  />
                </g>
              );
            })}

            {/* Overlapping Line & Area paths */}
            {visibleSeries.map((ser) => {
              const serSvgPoints = ser.points.map((p, idx) => {
                const x = paddingLeft + (idx / 150) * chartWidth;
                const yActual = paddingTop + chartHeight - (p.value / maxVal) * chartHeight;
                return { x, y: yActual };
              });

              const serLinePath = serSvgPoints.reduce((acc, p, idx) => {
                return acc + `${idx === 0 ? 'M' : 'L'} ${p.x} ${p.y}`;
              }, '');

              const serAreaPath = serLinePath
                ? `${serLinePath} L ${serSvgPoints[serSvgPoints.length - 1].x} ${paddingTop + chartHeight} L ${serSvgPoints[0].x} ${paddingTop + chartHeight} Z`
                : '';

              return (
                <g key={ser.id}>
                  {serAreaPath && <path d={serAreaPath} fill={`url(#grad-${ser.id})`} />}
                  {serLinePath && (
                    <path
                      d={serLinePath}
                      stroke={ser.color}
                      strokeWidth="2"
                      fill="none"
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeDasharray={ser.isDashed ? '4 4' : undefined}
                    />
                  )}
                </g>
              );
            })}

            {/* Hover Crosshair Vertical Line */}
            {activeHoverPointCoord !== null && (
              <g>
                <line
                  x1={activeHoverPointCoord}
                  y1={paddingTop}
                  x2={activeHoverPointCoord}
                  y2={paddingTop + chartHeight}
                  stroke="var(--text-secondary)"
                  strokeWidth="1"
                  strokeDasharray="2 2"
                  opacity="0.5"
                />
                {/* Draw circle markers on each series line at the hover X position */}
                {visibleSeries.map((ser) => {
                  if (!ser.points[hoverIdx]) return null;
                  const y = paddingTop + chartHeight - (ser.points[hoverIdx].value / maxVal) * chartHeight;
                  return (
                    <g key={`hover-marker-${ser.id}`}>
                      <circle
                        cx={activeHoverPointCoord}
                        cy={y}
                        r="7"
                        fill={ser.color}
                        opacity="0.3"
                        style={{ animation: 'pulse-glow 1s infinite alternate' }}
                      />
                      <circle
                        cx={activeHoverPointCoord}
                        cy={y}
                        r="4"
                        fill={ser.color}
                        stroke="var(--bg-primary)"
                        strokeWidth="1.5"
                      />
                    </g>
                  );
                })}
              </g>
            )}
          </svg>

          {/* Interactive Tooltip overlay */}
          {hoveredPoint && activeHoverPointCoord !== null && visibleSeries.length > 0 && (
            <div
              className="claude-aggregate-chart-tooltip glass-tooltip"
              style={{
                position: 'absolute',
                left: `${(activeHoverPointCoord / width) * 100}%`,
                top: `0%`,
                transform: 'translateX(-50%) translateY(-105%)',
              }}
            >
              <div style={{ display: 'flex', flexDirection: 'column', gap: '4px' }}>
                <div style={{ fontWeight: 600, borderBottom: '1px solid var(--border)', paddingBottom: '4px', fontSize: '11px', color: 'var(--text-primary)' }}>
                  {hoveredPoint.label}
                </div>
                
                {/* Values for all visible series */}
                <div style={{ display: 'flex', flexDirection: 'column', gap: '3px', marginTop: '3px' }}>
                  {hoveredPoint.seriesValues.map((sv: any, idx: number) => (
                    <div key={idx} style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', fontSize: '11px', gap: '16px' }}>
                      <span style={{ color: 'var(--text-secondary)', display: 'flex', alignItems: 'center', gap: '4px' }}>
                        <span style={{ display: 'inline-block', width: '6px', height: '6px', borderRadius: '50%', backgroundColor: sv.color }} />
                        {sv.label}:
                      </span>
                      <span style={{ fontWeight: 700, color: 'var(--text-primary)' }}>
                        {sv.value}% <span style={{ fontSize: '9px', fontWeight: 400, color: 'var(--text-secondary)' }}>({(sv.value / 100).toFixed(1)} accts)</span>
                      </span>
                    </div>
                  ))}
                </div>

                {/* Reset Events lists */}
                {hoveredPoint.resets && hoveredPoint.resets.length > 0 && (
                  <div style={{ marginTop: '5px', borderTop: '1px solid var(--border)', paddingTop: '4px' }}>
                    <div style={{ fontSize: '9px', color: 'var(--warning)', fontWeight: 700, marginBottom: '2px' }}>
                      {t('accounts.aggregate.upcomingResets', 'UPCOMING RESETS')}:
                    </div>
                    <div style={{ display: 'flex', flexDirection: 'column', gap: '2px', maxHeight: '60px', overflowY: 'auto' }}>
                      {hoveredPoint.resets.map((r: any, rIdx: number) => (
                        <div key={rIdx} style={{ fontSize: '8px', color: 'var(--text-primary)', whiteSpace: 'nowrap', display: 'flex', justifyContent: 'space-between', gap: '12px' }}>
                          <span>• {r.email} ({r.seriesLabel})</span>
                          <span style={{ color: '#10b981', fontWeight: 600 }}>+{r.capacityIncrease}%</span>
                        </div>
                      ))}
                    </div>
                  </div>
                )}
              </div>
            </div>
          )}

          {/* Empty selection state */}
          {visibleSeries.length === 0 && (
            <div style={{
              position: 'absolute',
              top: 0,
              left: 0,
              width: '100%',
              height: '100%',
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              background: 'rgba(0,0,0,0.02)',
              borderRadius: '8px',
              fontSize: '12px',
              color: 'var(--text-secondary)',
              fontStyle: 'italic'
            }}>
              {t('accounts.aggregate.selectToView', 'Click legend items above to toggle model projections.')}
            </div>
          )}
        </div>
      </div>
    );
  };

  if (!renderPanel) {
    return renderChartContent();
  }

  const displayPlatformName = platform === 'antigravity'
    ? 'Antigravity'
    : platform === 'codex'
      ? 'Codex'
      : platform.charAt(0).toUpperCase() + platform.slice(1);

  return (
    <div className="aggregate-projection-panel">
      <div
        className="panel-header"
        onClick={toggleProjection}
        style={{ cursor: 'pointer' }}
      >
        <div className="panel-title-container">
          <History size={16} className="panel-icon" />
          <h3>{displayPlatformName} Aggregate Capacity Projection</h3>
        </div>
        <button className="panel-toggle-btn">
          {showAggregateProjection ? 'Hide' : 'Show'}
        </button>
      </div>
      {showAggregateProjection && (
        <div className="panel-body">
          <p className="panel-desc">
            Aggregated projection of capacity across all active accounts. Resets recover capacity dynamically.
          </p>
          {renderChartContent()}
        </div>
      )}
    </div>
  );
};
