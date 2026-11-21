import { invoke } from '@tauri-apps/api/core';
import { ChevronLeft, ChevronRight, History as HistoryIcon, RefreshCw, X } from 'lucide-react';
import { useEffect, useId, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { useEscCloseTopmost } from '../hooks/useEscClose';
import { useModalFocusTrap } from '../hooks/useModalFocusTrap';
import { useModalScrollLock } from '../hooks/useModalScrollLock';
import { SingleSelectDropdown } from './SingleSelectDropdown';
import { QUOTA_PLOT, quotaHistoryDateRange, quotaHistoryTicks, type QuotaHistoryPreset } from './codex/codexQuotaHistoryChart';
import './PlatformQuotaHistory.css';

interface Point {
  observedAt: number;
  remaining: number;
  resetAt: number | null;
  change: number | null;
}

interface History {
  series: { key: string; label: string }[];
  selected: string | null;
  points: Point[];
}

interface Props {
  platform: 'antigravity' | 'cursor';
  accounts: { value: string; label: string; createdAt: number }[];
}

function QuotaTrend({ points, period, label, lang }: {
  points: Point[];
  period: QuotaHistoryPreset;
  label: string;
  lang: string;
}) {
  const { width, height, top, bottom } = QUOTA_PLOT;
  const left = 68;
  const right = 62;
  const first = points[0].observedAt;
  const last = points[points.length - 1].observedAt;
  const x = (time: number) => first === last ? width / 2 : left + (time - first) / (last - first) * (width - left - right);
  const y = (remaining: number) => top + (100 - remaining) / 100 * (height - top - bottom);
  const path = points.map((point, index) => `${index === 0 || point.change === null ? 'M' : 'L'}${x(point.observedAt).toFixed(2)} ${y(point.remaining).toFixed(2)}`).join(' ');
  return (
    <svg viewBox={`0 0 ${width} ${height}`} role="img" aria-label={label} className="platform-quota-history__chart">
      {[0, 25, 50, 75, 100].map((level) => <g key={level}>
        <line x1={left} x2={width - right} y1={y(level)} y2={y(level)} />
        <text x={left - 7} y={y(level)} dominantBaseline="middle" textAnchor="end">{level}%</text>
      </g>)}
      {quotaHistoryTicks(first, last, period, 5, lang).map((tick) => (
        <text key={tick.value} x={x(tick.value)} y={height - 17} textAnchor="middle">{tick.label}</text>
      ))}
      <path d={path} />
      {points.filter((point, index) => index === 0 || point.change === null || points[index + 1]?.change === null || index === points.length - 1)
        .map((point) => <circle key={point.observedAt} cx={x(point.observedAt)} cy={y(point.remaining)} r="3.5" />)}
    </svg>
  );
}

function PlatformQuotaHistoryDialog({ platform, accounts, onClose }: Props & { onClose: () => void }) {
  const { t, i18n } = useTranslation();
  const titleId = useId();
  const dialogRef = useRef<HTMLDivElement>(null);
  const [accountChoice, setAccountChoice] = useState(accounts[0]?.value ?? '');
  const [metric, setMetric] = useState<string | null>(null);
  const [period, setPeriod] = useState<QuotaHistoryPreset>('thisMonth');
  const [offset, setOffset] = useState(0);
  const [refresh, setRefresh] = useState(0);
  const [shown, setShown] = useState(50);
  const [result, setResult] = useState<{ query: string; history?: History }>();
  const account = accounts.find(({ value }) => value === accountChoice) ?? accounts[0];
  const accountId = account?.value;
  const { from, to } = quotaHistoryDateRange(period, offset);
  const query = JSON.stringify([platform, accountId, account?.createdAt, from, to, metric, refresh]);
  const current = result?.query === query ? result : undefined;
  const history = current?.history;
  const lang = i18n.resolvedLanguage || i18n.language;
  const date = new Intl.DateTimeFormat(lang, { dateStyle: 'medium' });
  const time = new Intl.DateTimeFormat(lang, { dateStyle: 'medium', timeStyle: 'short' });
  const percentage = new Intl.NumberFormat(lang, { style: 'percent', maximumFractionDigits: 0 });
  const delta = new Intl.NumberFormat(lang, { signDisplay: 'exceptZero' });
  const hours = new Intl.NumberFormat(lang, { style: 'unit', unit: 'hour', unitDisplay: 'narrow' });
  const minutes = new Intl.NumberFormat(lang, { style: 'unit', unit: 'minute', unitDisplay: 'narrow' });
  const formatCountdown = (milliseconds: number) => {
    const totalMinutes = Math.round(milliseconds / 60_000);
    return totalMinutes < 60 ? minutes.format(totalMinutes)
      : `${hours.format(Math.floor(totalMinutes / 60))} ${minutes.format(totalMinutes % 60)}`;
  };
  const cursorLabels: Record<string, string> = {
    total: t('common.shared.quota.usage'), auto: t('cursor.quota.cursorModels'),
    api: t('cursor.quota.otherModels'), sand: t('cursor.quota.sandUsage'), on_demand: t('cursor.quota.onDemand'),
  };
  const title = t('codex.quotaHistory.title');
  const points = history?.points ?? [];
  const visible = points.slice(-shown).reverse();

  useEscCloseTopmost(true, onClose, dialogRef);
  useModalFocusTrap(dialogRef, true);
  useModalScrollLock(true);

  useEffect(() => {
    if (!accountId) return;
    let active = true;
    setShown(50);
    void invoke<History>('get_platform_quota_history', { platform, accountId, from, to, metric })
      .then((history) => { if (active) setResult({ query, history }); })
      .catch(() => { if (active) setResult({ query }); });
    return () => { active = false; };
  }, [accountId, metric, platform, from, to, query]);

  return createPortal(
    <div className="modal-overlay" onClick={onClose}>
      <div ref={dialogRef} className="modal-content platform-quota-history" role="dialog" aria-modal="true"
        aria-labelledby={titleId} tabIndex={-1} onClick={(event) => event.stopPropagation()}>
        <div className="modal-header">
          <h2 id={titleId}>{title}</h2>
          <button type="button" className="modal-close" onClick={onClose} aria-label={t('common.close')}><X size={20} /></button>
        </div>
        <div className="modal-body">
          <div className="platform-quota-history__filters">
            <SingleSelectDropdown options={accounts} value={accountId ?? ''} ariaLabel={t('codex.quotaHistory.account')}
              onChange={(value) => { setAccountChoice(value); setMetric(null); }} />
            <SingleSelectDropdown options={[
              { value: 'today', label: t('codex.apiService.statsRange.today') },
              { value: 'thisWeek', label: t('codex.apiService.statsRange.thisWeek') },
              { value: 'thisMonth', label: t('codex.apiService.statsRange.thisMonth') },
            ]} value={period} ariaLabel={t('codex.sessionUsage.range.label')}
              onChange={(value) => { setPeriod(value as QuotaHistoryPreset); setOffset(0); }} />
            <button type="button" className="btn btn-secondary icon-only" onClick={() => setOffset(offset - 1)}
              aria-label={t('pagination.prev')}><ChevronLeft size={15} /></button>
            <span>{date.format(from)}{period === 'today' ? '' : ` – ${date.format(to - 1)}`}</span>
            <button type="button" className="btn btn-secondary icon-only" onClick={() => setOffset(offset + 1)}
              aria-label={t('pagination.next')} disabled={offset === 0}><ChevronRight size={15} /></button>
            <button type="button" className="btn btn-secondary icon-only" onClick={() => setRefresh(refresh + 1)}
              aria-label={t('common.refresh')}><RefreshCw size={15} /></button>
          </div>
          {current && !history ? <p role="alert">{t('codex.quotaHistory.readFailed')}</p> : !accountId ? (
            <p role="status">{t('common.shared.empty.title')}</p>
          ) : !history ? <p role="status">{t('common.loading')}</p> : points.length === 0 ? (
            <p role="status">{t('codex.quotaHistory.empty')}</p>
          ) : <>
            <SingleSelectDropdown value={history.selected ?? ''} ariaLabel={t('common.codexUsage.model')}
              options={history.series.map(({ key, label }) => ({ value: key, label: platform === 'cursor' ? cursorLabels[key] ?? label : label }))}
              onChange={setMetric} />
            <QuotaTrend points={points} period={period} label={t('codex.quotaHistory.chart')} lang={lang} />
            <div className="platform-quota-history__table">
              <table className="account-table">
                <thead><tr>
                  <th>{t('codex.quotaHistory.recordedAt')}</th>
                  <th>{t('apiKeyFun.usage.remaining')}</th>
                  <th>{t('codex.quotaHistory.changeFromPrevious')}</th>
                  <th>{t('modals.quota.resetTime')}</th>
                </tr></thead>
                <tbody>{visible.map((point) => <tr key={point.observedAt}>
                  <td>{time.format(point.observedAt)}</td>
                  <td>{percentage.format(point.remaining / 100)}</td>
                  <td>{point.change === null ? '—' : t('codex.quotaHistory.points', { value: delta.format(point.change) })}</td>
                  <td>{point.resetAt === null ? '—' : <>
                    {time.format(point.resetAt)}
                    {point.resetAt >= point.observedAt && <small>
                      {formatCountdown(point.resetAt - point.observedAt)}
                    </small>}
                  </>}</td>
                </tr>)}</tbody>
              </table>
            </div>
            {visible.length < points.length && <button type="button" className="btn btn-secondary" onClick={() => setShown(shown + 50)}>{t('common.codexUsage.more')}</button>}
          </>}
        </div>
      </div>
    </div>, document.body,
  );
}

export function PlatformQuotaHistoryButton(props: Props) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const label = t('codex.quotaHistory.title');
  return <>
    <button type="button" className="btn btn-secondary icon-only" title={label} aria-label={label}
      onClick={() => setOpen(true)} disabled={props.accounts.length === 0}><HistoryIcon size={15} /></button>
    {open && <PlatformQuotaHistoryDialog {...props} onClose={() => setOpen(false)} />}
  </>;
}
