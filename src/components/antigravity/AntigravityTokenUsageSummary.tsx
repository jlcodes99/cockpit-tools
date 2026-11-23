import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { BarChart3, RefreshCw } from 'lucide-react';
import {
  getSavedAntigravityUsageRange,
  hasRecentAntigravityTokenSync,
  peekCachedAntigravitySummaryByPreset,
  queryAntigravityTokenUsage,
  syncAntigravityTokenUsage,
  useAntigravityUsageRange,
} from '../../services/antigravityTokenUsageService';
import type {
  AntigravityUsageRangePreset,
  DashboardBundleResponse,
} from '../../types/antigravityTokenUsage';
import {
  SESSION_USAGE_SUMMARY_PENDING,
  formatSessionUsageCount,
  formatSessionUsageTokensShort,
} from '../../utils/codexSessionUsageFormat';

const QUICK_RANGES: AntigravityUsageRangePreset[] = [
  'today',
  '7d',
  '30d',
  'month',
  'all',
];

export function AntigravityTokenUsageSummary({
  onOpenDetail,
}: {
  onOpenDetail: () => void;
}) {
  const { t, i18n } = useTranslation();
  const lang = i18n.resolvedLanguage || i18n.language || 'en';
  const [range, setRange] = useAntigravityUsageRange();
  const [bundle, setBundle] = useState<DashboardBundleResponse | null>(() =>
    peekCachedAntigravitySummaryByPreset(getSavedAntigravityUsageRange()),
  );
  const [syncing, setSyncing] = useState(false);
  const [initialLoaded, setInitialLoaded] = useState<boolean>(
    () => peekCachedAntigravitySummaryByPreset(getSavedAntigravityUsageRange()) !== null,
  );
  const reqSeqRef = useRef(0);
  const didMountCheckRef = useRef(false);

  const loadForRange = useCallback(
    async (targetRange: AntigravityUsageRangePreset, forceSync: boolean) => {
      const seq = ++reqSeqRef.current;
      const memHit = peekCachedAntigravitySummaryByPreset(targetRange);
      if (memHit) {
        setBundle(memHit);
        setInitialLoaded(true);
      }

      if (forceSync) {
        setSyncing(true);
        try {
          const synced = await syncAntigravityTokenUsage(false, {
            preset: targetRange,
            pageSize: 1,
          });
          if (seq === reqSeqRef.current) {
            setBundle(synced);
            setInitialLoaded(true);
          }
        } catch {
          if (seq === reqSeqRef.current) {
            setInitialLoaded(true);
          }
        } finally {
          if (seq === reqSeqRef.current) {
            setSyncing(false);
          }
        }
        return;
      }

      try {
        const fastResult = await queryAntigravityTokenUsage({
          preset: targetRange,
          pageSize: 1,
        });
        if (seq === reqSeqRef.current) {
          setBundle(fastResult);
          setInitialLoaded(true);
        }
      } catch {
        if (seq === reqSeqRef.current) {
          setInitialLoaded(true);
        }
      }
    },
    [],
  );

  useEffect(() => {
    const isInitial = !didMountCheckRef.current;
    didMountCheckRef.current = true;
    const needsFreshSync = isInitial && !hasRecentAntigravityTokenSync(4_000);
    if (needsFreshSync) {
      // Render cached backend snapshot first (near-instant when warm), then refresh incrementally if stale.
      void (async () => {
        await loadForRange(range, false);
        if (!hasRecentAntigravityTokenSync(4_000)) {
          await loadForRange(range, true);
        }
      })();
    } else {
      void loadForRange(range, false);
    }
  }, [loadForRange, range]);

  const kpi = bundle?.summary.kpi;
  const exact = kpi?.exact;
  const cacheTotal =
    (exact?.cacheReadTokens ?? 0) + (exact?.cacheWriteTokens ?? 0);
  const ready = bundle !== null;

  const metrics = useMemo(
    () => [
      {
        key: 'input',
        label: t('antigravityTokenUsage.summary.input', 'Input'),
        value: ready
          ? formatSessionUsageTokensShort(exact?.inputTokens ?? 0, lang)
          : SESSION_USAGE_SUMMARY_PENDING,
        exactTitle: ready
          ? formatSessionUsageCount(exact?.inputTokens ?? 0)
          : undefined,
        className: '',
      },
      {
        key: 'thinking',
        label: t('antigravityTokenUsage.summary.thinking', 'Thinking'),
        value: ready
          ? formatSessionUsageTokensShort(exact?.thinkingTokens ?? 0, lang)
          : SESSION_USAGE_SUMMARY_PENDING,
        exactTitle: ready
          ? formatSessionUsageCount(exact?.thinkingTokens ?? 0)
          : undefined,
        className: 'is-thinking',
      },
      {
        key: 'output',
        label: t('antigravityTokenUsage.summary.output', 'Output'),
        value: ready
          ? formatSessionUsageTokensShort(exact?.outputTokens ?? 0, lang)
          : SESSION_USAGE_SUMMARY_PENDING,
        exactTitle: ready
          ? formatSessionUsageCount(exact?.outputTokens ?? 0)
          : undefined,
        className: '',
      },
      {
        key: 'cache',
        label: t('antigravityTokenUsage.summary.cache', 'Cache'),
        value: ready
          ? formatSessionUsageTokensShort(cacheTotal, lang)
          : SESSION_USAGE_SUMMARY_PENDING,
        exactTitle: ready ? formatSessionUsageCount(cacheTotal) : undefined,
        className: '',
      },
      {
        key: 'total',
        label: t('antigravityTokenUsage.summary.total', 'Total'),
        value: ready
          ? formatSessionUsageTokensShort(exact?.totalTokens ?? 0, lang, 2)
          : SESSION_USAGE_SUMMARY_PENDING,
        exactTitle: ready
          ? formatSessionUsageCount(exact?.totalTokens ?? 0)
          : undefined,
        className: 'is-total',
      },
      {
        key: 'turns',
        label: t('antigravityTokenUsage.summary.turns', 'Turns'),
        value: ready
          ? formatSessionUsageCount(kpi?.totalTurns ?? 0)
          : SESSION_USAGE_SUMMARY_PENDING,
        exactTitle: ready
          ? `${formatSessionUsageCount(kpi?.totalConversations ?? 0)} ${t('antigravityTokenUsage.summary.sessionsUnit', 'sessions')}`
          : undefined,
        className: '',
      },
    ],
    [
      cacheTotal,
      exact,
      kpi?.totalConversations,
      kpi?.totalTurns,
      lang,
      ready,
      t,
    ],
  );

  return (
    <div className="ag-token-summary">
      <div className="ag-token-summary__leading">
        <span className="ag-token-summary__badge">
          <BarChart3 size={14} />
          {t('antigravityTokenUsage.summary.badge', 'Local Tokens')}
        </span>
        <div
          className="ag-token-summary__ranges"
          role="group"
          aria-label={t('antigravityTokenUsage.range.label', 'Time Range')}
        >
          {QUICK_RANGES.map((preset) => (
            <button
              key={preset}
              type="button"
              className={`ag-token-summary__range-btn${range === preset ? ' is-active' : ''}`}
              onClick={() => setRange(preset)}
            >
              {t(`antigravityTokenUsage.range.${preset}`, preset)}
            </button>
          ))}
        </div>
        {!initialLoaded || syncing ? (
          <span
            className="ag-token-summary__status is-scanning"
            role="status"
          >
            {!initialLoaded
              ? t('antigravityTokenUsage.summary.scanning', 'Scanning…')
              : t('antigravityTokenUsage.summary.updating', 'Updating…')}
          </span>
        ) : null}
      </div>

      <div className="ag-token-summary__metrics">
        {metrics.map((metric) => (
          <span
            key={metric.key}
            className={`ag-token-summary__metric ${metric.className}`.trim()}
            title={metric.exactTitle}
          >
            <em>{metric.label}</em>
            <strong className={ready ? undefined : 'is-pending'}>
              {metric.value}
            </strong>
          </span>
        ))}
      </div>

      <div className="ag-token-summary__actions">
        <button
          type="button"
          className="ag-token-summary__icon-btn"
          title={t('antigravityTokenUsage.actions.refresh', 'Refresh')}
          onClick={() => void loadForRange(range, true)}
          disabled={syncing}
        >
          <RefreshCw size={13} className={syncing ? 'spin' : undefined} />
        </button>
        <button
          type="button"
          className="ag-token-summary__detail"
          onClick={onOpenDetail}
        >
          {t('antigravityTokenUsage.summary.detail', 'Details')}
        </button>
      </div>
    </div>
  );
}
