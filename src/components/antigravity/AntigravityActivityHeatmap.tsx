import { useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { Calendar, X } from 'lucide-react';
import type { HeatmapDayPoint } from '../../types/antigravityTokenUsage';
import {
  formatSessionUsageCount,
  formatSessionUsageTokensShort,
} from '../../utils/codexSessionUsageFormat';

interface HeatmapCell {
  date: string;
  point: HeatmapDayPoint | null;
  level: number;
}

const WEEKS_TO_SHOW = 28;

function formatYmd(d: Date): string {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, '0');
  const day = String(d.getDate()).padStart(2, '0');
  return `${y}-${m}-${day}`;
}

export function formatUsdShort(usd: number | undefined | null): string {
  const val = Number(usd ?? 0);
  if (!Number.isFinite(val) || val <= 0) return '$0.00';
  if (val < 0.01) return `$${val.toFixed(4)}`;
  return `$${val.toFixed(2)}`;
}

export function AntigravityActivityHeatmap({
  points,
  selectedDate,
  onSelectDate,
  lang,
}: {
  points: HeatmapDayPoint[];
  selectedDate: string | null;
  onSelectDate: (date: string | null) => void;
  lang: string;
}) {
  const { t } = useTranslation();

  const { weeks, activeDaysCount, peakDay } = useMemo(() => {
    const byDate = new Map<string, HeatmapDayPoint>();
    let peak: HeatmapDayPoint | null = null;
    for (const pt of points) {
      byDate.set(pt.date, pt);
      if (pt.totalTokens > 0 && (!peak || pt.totalTokens > peak.totalTokens)) {
        peak = pt;
      }
    }

    const today = new Date();
    today.setHours(0, 0, 0, 0);
    // Align end of grid to the end of the current week (Sunday = 6 in Mon-Sun or Sat = 6 in Sun-Sat)
    const dayOfWeek = today.getDay(); // 0 (Sun) .. 6 (Sat)
    const endGrid = new Date(today);
    endGrid.setDate(today.getDate() + (6 - dayOfWeek));

    const totalDays = WEEKS_TO_SHOW * 7;
    const startGrid = new Date(endGrid);
    startGrid.setDate(endGrid.getDate() - totalDays + 1);

    const cols: HeatmapCell[][] = [];
    const cursor = new Date(startGrid);
    for (let w = 0; w < WEEKS_TO_SHOW; w += 1) {
      const col: HeatmapCell[] = [];
      for (let d = 0; d < 7; d += 1) {
        const ymd = formatYmd(cursor);
        const isFuture = cursor.getTime() > today.getTime();
        const pt = isFuture ? null : (byDate.get(ymd) ?? null);
        col.push({
          date: isFuture ? '' : ymd,
          point: pt,
          level: pt ? pt.level : 0,
        });
        cursor.setDate(cursor.getDate() + 1);
      }
      cols.push(col);
    }

    return {
      weeks: cols,
      activeDaysCount: points.filter((p) => p.totalTokens > 0).length,
      peakDay: peak,
    };
  }, [points]);

  return (
    <section className="ag-token-section ag-heatmap-card">
      <div className="ag-token-section__head">
        <div className="ag-heatmap-head__title">
          <Calendar size={15} />
          <h4>
            {t(
              'antigravityTokenUsage.heatmap.title',
              'Activity Heatmap (Click a Day to Filter)',
            )}
          </h4>
          <span className="ag-heatmap-head__meta">
            {t('antigravityTokenUsage.heatmap.activeDays', {
              count: activeDaysCount,
              defaultValue: '{{count}} active days',
            })}
            {peakDay ? (
              <>
                {' · '}
                {t('antigravityTokenUsage.heatmap.peak', {
                  date: peakDay.date,
                  tokens: formatSessionUsageTokensShort(
                    peakDay.totalTokens,
                    lang,
                    1,
                  ),
                  defaultValue: 'Peak {{date}} ({{tokens}})',
                })}
              </>
            ) : null}
          </span>
        </div>

        <div className="ag-heatmap-head__controls">
          {selectedDate ? (
            <button
              type="button"
              className="btn btn-secondary btn-sm ag-heatmap-clear-btn"
              onClick={() => onSelectDate(null)}
            >
              <span>{selectedDate}</span>
              <X size={12} />
            </button>
          ) : null}
          <div className="ag-heatmap-legend" aria-hidden="true">
            <span>{t('antigravityTokenUsage.heatmap.less', 'Less')}</span>
            {[0, 1, 2, 3, 4].map((lv) => (
              <i
                key={lv}
                className={`ag-heatmap-cell is-level-${lv}`}
              />
            ))}
            <span>{t('antigravityTokenUsage.heatmap.more', 'More')}</span>
          </div>
        </div>
      </div>

      <div className="ag-heatmap-grid-wrap">
        <div className="ag-heatmap-grid" role="grid">
          {weeks.map((col, wIdx) => (
            <div key={wIdx} className="ag-heatmap-col" role="row">
              {col.map((cell, dIdx) => {
                if (!cell.date) {
                  return (
                    <span
                      key={dIdx}
                      className="ag-heatmap-cell is-future"
                      aria-hidden="true"
                    />
                  );
                }
                const isSelected = selectedDate === cell.date;
                const tok = cell.point?.totalTokens ?? 0;
                const turns = cell.point?.turnCount ?? 0;
                const cost = formatUsdShort(cell.point?.estimatedCostUsd);
                const tip =
                  tok > 0
                    ? `${cell.date}\n${formatSessionUsageCount(tok)} tokens (${turns} turns · ≈ ${cost})`
                    : `${cell.date} · 0 tokens`;
                return (
                  <button
                    key={cell.date}
                    type="button"
                    className={`ag-heatmap-cell is-level-${cell.level}${isSelected ? ' is-selected' : ''}`}
                    title={tip}
                    aria-label={tip}
                    aria-pressed={isSelected}
                    onClick={() =>
                      onSelectDate(isSelected ? null : cell.date)
                    }
                  />
                );
              })}
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}
