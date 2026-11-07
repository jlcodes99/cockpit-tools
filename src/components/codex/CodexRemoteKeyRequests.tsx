import { Fragment, useState } from 'react';
import type { TFunction } from 'i18next';
import type { CodexRemoteKeyRequests as RequestPage } from '../../types/codexRemoteService';
import { formatBudgetUsd } from '../../utils/codexKeyBudget';

export function CodexRemoteKeyRequests({ data, loading, t }: { data?: RequestPage; loading: boolean; t: TFunction }) {
  const [expanded, setExpanded] = useState<number | null>(null);
  const number = (value: number) => new Intl.NumberFormat(undefined, { notation: 'compact', maximumFractionDigits: 1 }).format(value);
  const duration = (value: number | null) => value == null ? '—' : `${(value / 1000).toFixed(2)}s`;
  const outcomeKey = { success: 'success', failed: 'failed', canceled: 'cancelled', incomplete: 'incomplete', timeout: '' };
  return <div className="codex-remote-log-table" aria-busy={loading}>
    <table>
      <thead><tr>
        {['time', 'model', 'result', 'tokens', 'performance'].map(key => <th key={key}>{t(`codex.apiService.logs.columns.${key}`)}</th>)}
        <th>{t('common.apiBudget.settled')}</th><th>{t('common.detail')}</th>
      </tr></thead>
      <tbody>
        {(data?.items ?? []).map(row => <Fragment key={row.id}>
          <tr>
            <td>{new Date(row.timestamp).toLocaleString(undefined, { timeZone: 'UTC', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', second: '2-digit' })}</td>
            <td><code>{row.model || '—'}</code>{row.kind === 'image_generation' || row.kind === 'image_edit'
              ? <div>{t(`codex.localAccess.requestKind.${row.kind === 'image_generation' ? 'imageGeneration' : 'imageEdit'}`)}</div> : null}</td>
            <td>{row.outcome === 'timeout' ? t('common.apiBudget.timeout') : t(`common.codexUsage.${outcomeKey[row.outcome]}`)}</td>
            <td>{number(row.totalTokens)}</td><td>{duration(row.latencyMs)}</td>
            <td>{row.settlementStatus === 'settled' && row.settledMicrousd != null
              ? formatBudgetUsd(row.settledMicrousd) : t(`common.apiBudget.${row.settlementStatus === 'unmetered' ? 'unmeteredShort' : 'unrecorded'}`)}</td>
            <td><button type="button" className="btn btn-secondary btn-sm" aria-expanded={expanded === row.id}
              onClick={() => setExpanded(expanded === row.id ? null : row.id)}>{t(expanded === row.id ? 'common.close' : 'common.detail')}</button></td>
          </tr>
          {expanded === row.id && <tr className="codex-remote-log-expanded"><td colSpan={7}>
            <div className="codex-remote-details-grid">
              {[
                [t('common.requestId'), row.requestId || '—'],
                [t('common.codexUsage.input'), number(row.inputTokens)],
                [t('common.codexUsage.output'), number(row.outputTokens)],
                [t('common.codexUsage.cached'), number(row.cachedTokens)],
                [t('common.codexUsage.firstResponse'), duration(row.firstResponseMs)],
                [t('common.codexUsage.estimatedValue'), formatBudgetUsd(Math.round(row.estimatedCostUsd * 1_000_000))],
                ['HTTP', row.httpStatus == null ? '—' : String(row.httpStatus)],
              ].map(([label, value]) => <div className="codex-remote-detail-value" key={label}><span>{label}</span><strong>{value}</strong></div>)}
            </div>
          </td></tr>}
        </Fragment>)}
        {!data?.items?.length && <tr><td colSpan={7}>{t(loading ? 'common.loading' : data && !data.ready ? 'common.apiBudget.preparing' : 'common.codexUsage.noRequests')}</td></tr>}
      </tbody>
    </table>
  </div>;
}
