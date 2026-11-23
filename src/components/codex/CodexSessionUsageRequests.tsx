import { Fragment, useEffect, useMemo, useState } from 'react';
import { ChevronDown, ChevronRight } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { querySessionUsageRequests } from '../../services/codexInstanceService';
import type {
  CodexSessionUsageCursor,
  CodexSessionUsagePage,
  CodexSessionUsageQuery,
} from '../../types/codex';
import {
  formatSessionUsageCostUsd,
  formatSessionUsageCount,
  formatSessionUsageTokensShort,
} from '../../utils/codexSessionUsageFormat';

export function usageProjectLabel(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

interface Props {
  query: CodexSessionUsageQuery;
  revision: object | null;
  lang: string;
  sessionNames?: ReadonlyMap<string, string>;
  onSelectSession: (id: string, name: string) => void;
}

export function CodexSessionUsageRequests({ query, revision, lang, sessionNames, onSelectSession }: Props) {
  const { t } = useTranslation();
  const [cursors, setCursors] = useState<(CodexSessionUsageCursor | null)[]>([null]);
  const [page, setPage] = useState<CodexSessionUsagePage | null>(null);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [retry, setRetry] = useState(0);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');

  const cursor = cursors[cursors.length - 1];
  useEffect(() => {
    let current = true;
    setLoading(true);
    setError('');
    void querySessionUsageRequests(query, cursor).then((result) => {
      if (current) setPage(result);
    }).catch((cause: unknown) => {
      if (current) setError(cause instanceof Error ? cause.message : String(cause));
    }).finally(() => {
      if (current) setLoading(false);
    });
    return () => { current = false; };
  }, [query, cursor, revision, retry]);

  const time = useMemo(
    () => new Intl.DateTimeFormat(lang, { dateStyle: 'short', timeStyle: 'medium' }),
    [lang],
  );
  const percent = useMemo(
    () => new Intl.NumberFormat(lang, { style: 'percent', maximumFractionDigits: 1 }),
    [lang],
  );
  const prefix = 'codex.sessionUsage.analysis';
  const columns = [
    { id: 'time', label: t(`${prefix}.time`) },
    { id: 'model', label: t('codex.sessionUsage.tables.modelName') },
    { id: 'instance', label: t('codex.sessionUsage.tables.instanceName') },
    { id: 'project', label: t(`${prefix}.project`) },
    { id: 'input', label: t(`${prefix}.freshInput`) },
    { id: 'cached', label: t(`${prefix}.cached`) },
    { id: 'output', label: t('codex.sessionUsage.summary.output') },
    { id: 'cost', label: t(`${prefix}.referenceCost`) },
  ];
  const token = (value: number) => (
    <span title={formatSessionUsageCount(value)}>{formatSessionUsageTokensShort(value, lang)}</span>
  );

  return (
    <div aria-busy={loading}>
      {error && (
        <div className="codex-session-usage__error" role="alert">
          {t('codex.sessionUsage.errors.loadFailed', { error })}
          <button type="button" className="btn btn-secondary" onClick={() => setRetry((n) => n + 1)}>
            {t('common.retry')}
          </button>
        </div>
      )}
      {page?.rows.length ? (
        <div className="codex-session-usage-table-wrap">
          <table className="codex-session-usage-table codex-session-usage__requests">
            <thead><tr>{columns.map(({ id, label }) => (
              <th key={id} scope="col" className={`codex-session-usage__col-${id}`}>{label}</th>
            ))}</tr></thead>
            <tbody>
              {page.rows.map((row) => {
                const isExpanded = expanded === row.requestId;
                const project = usageProjectLabel(row.projectPath) || t(`${prefix}.unknownProject`);
                const title = sessionNames?.get(row.sessionId) || row.sessionTitle || row.sessionId;
                const timestamp = row.timestamp > 0 ? time.format(row.timestamp * 1000) : '—';
                const fresh = Math.max(0, row.inputTokens - row.cachedInputTokens);
                return (
                  <Fragment key={row.requestId}>
                    <tr>
                      <td className="codex-session-usage__col-time">
                        <button type="button" className="btn codex-session-usage__row-button"
                          aria-expanded={isExpanded} title={timestamp}
                          onClick={() => setExpanded(isExpanded ? null : row.requestId)}>
                          {isExpanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                          <span>{timestamp}</span>
                        </button>
                      </td>
                      <td className="codex-session-usage__col-model" title={row.model}>{row.model || '—'}</td>
                      <td className="codex-session-usage__col-instance" title={row.instanceName}>{row.instanceName}</td>
                      <td className="codex-session-usage__col-project" title={row.projectPath || project}>{project}</td>
                      <td className="codex-session-usage__col-input">{token(fresh)}</td>
                      <td className="codex-session-usage__col-cached">{token(row.cachedInputTokens)}</td>
                      <td className="codex-session-usage__col-output">{token(row.outputTokens)}</td>
                      <td className="codex-session-usage__col-cost" title={row.estimatedCostUsd == null ? t(`${prefix}.unpriced`) : undefined}>
                        {row.estimatedCostUsd == null ? '—' : formatSessionUsageCostUsd(row.estimatedCostUsd)}
                      </td>
                    </tr>
                    {isExpanded && (
                      <tr className="codex-session-usage__expanded">
                        <td colSpan={columns.length}>
                          <div className="codex-session-usage__expanded-header">
                            <strong title={title}>{title}</strong>
                            {!query.sessionId && row.sessionId && (
                              <button type="button" className="btn btn-secondary"
                                onClick={() => onSelectSession(row.sessionId, title)}>
                                {t(`${prefix}.viewSession`)}
                              </button>
                            )}
                          </div>
                          <dl>
                            {[
                              [t('codex.sessionUsage.tables.instanceName'), row.instanceName],
                              [t(`${prefix}.project`), row.projectPath || project],
                              [t(`${prefix}.freshInput`), formatSessionUsageCount(fresh)],
                              [t(`${prefix}.cached`), formatSessionUsageCount(row.cachedInputTokens)],
                              [t('codex.sessionUsage.summary.output'), formatSessionUsageCount(row.outputTokens)],
                              [t(`${prefix}.cacheRate`), row.inputTokens ? percent.format(row.cachedInputTokens / row.inputTokens) : '—'],
                            ].map(([label, value]) => (
                              <div key={label}><dt>{label}</dt><dd title={value}>{value}</dd></div>
                            ))}
                          </dl>
                        </td>
                      </tr>
                    )}
                  </Fragment>
                );
              })}
            </tbody>
          </table>
        </div>
      ) : !error && (
        <p className="codex-session-usage__table-message" role="status">
          {t(loading ? 'common.loading' : `${prefix}.noResults`)}
        </p>
      )}
      {(page?.nextCursor || cursors.length > 1) && (
        <div className="codex-session-usage__paging">
          <span role="status">{t(`${prefix}.page`, { page: cursors.length })}</span>
          <button type="button" className="btn btn-secondary" disabled={loading || cursors.length <= 1}
            onClick={() => { setPage(null); setExpanded(null); setCursors((items) => items.slice(0, -1)); }}>
            {t('pagination.prev')}
          </button>
          <button type="button" className="btn btn-secondary" disabled={loading || !page?.nextCursor}
            onClick={() => {
              const next = page?.nextCursor;
              if (!next) return;
              setPage(null);
              setExpanded(null);
              setCursors((items) => [...items, next]);
            }}>
            {t('pagination.next')}
          </button>
        </div>
      )}
    </div>
  );
}
