import { useCallback, useEffect, useId, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Box, ChevronRight, Coins, ExternalLink, Gift, Sparkles, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { openUrl } from '@tauri-apps/plugin-opener';
import { useEscCloseTopmost } from '../../hooks/useEscClose';
import { useModalFocusTrap } from '../../hooks/useModalFocusTrap';
import { useModalScrollLock } from '../../hooks/useModalScrollLock';
import {
  getWorkbuddyCreditPackages,
  getWorkbuddyRecentCreditUsage,
  type WorkbuddyCreditUsageRow,
} from '../../services/workbuddyService';
import type { WorkbuddyAccount } from '../../types/workbuddy';
import {
  getWorkbuddyCredits,
  readWorkbuddyCreditPackage,
  WORKBUDDY_CREDIT_GROUPS,
  type WorkbuddyCreditGroup,
} from '../../utils/workbuddyCredits';
import { parseDateTimeToEpoch, parseNumeric } from '../../utils/codebuddy-suite/parser';
import { ModalErrorMessage } from '../ModalErrorMessage';

type Credits = ReturnType<typeof getWorkbuddyCredits>;
const groupKey = (kind: WorkbuddyCreditGroup) => `workbuddy.credits.${kind}` as const;
const creditGroupIcons = { planBase: Box, planBonus: Gift, benefit: Sparkles, addon: Coins };

function useCreditFormat() {
  const { i18n } = useTranslation();
  const locale = i18n.resolvedLanguage ?? i18n.language;
  return {
    number: new Intl.NumberFormat(locale),
    date: new Intl.DateTimeFormat(locale, { dateStyle: 'medium', timeStyle: 'medium' }),
  };
}

export function WorkbuddyCreditsSummary({
  credits,
  onGroup,
  onDetails,
}: {
  credits: Credits;
  onDetails?: () => void;
  onGroup?: (group: WorkbuddyCreditGroup) => void;
}) {
  const { t } = useTranslation();
  const { number, date } = useCreditFormat();
  return (
    <div className={`workbuddy-credits-summary${onDetails ? ' workbuddy-credits-compact' : ''}`}>
      {credits.left != null && (
        <div className="workbuddy-credit-balance">
          <div className="workbuddy-credit-balance-value">
            <span>{t('workbuddy.credits.total')}</span>
            <strong>{number.format(credits.left)}</strong>
          </div>
          {onDetails && (
            <button type="button" className="btn btn-sm workbuddy-credit-link" onClick={onDetails}>
              {t('workbuddy.credits.details')} <ChevronRight size={13} aria-hidden="true" />
            </button>
          )}
        </div>
      )}
      {credits.groups.map((group) => {
        const label = t(groupKey(group.kind));
        const remainPercent = group.total > 0 ? Math.min(100, Math.floor(group.left / group.total * 100)) : 0;
        const usedPercent = group.total > 0 ? Math.min(100, group.used / group.total * 100) : 0;
        const Icon = creditGroupIcons[group.kind];
        return (
          <div key={group.kind} className="workbuddy-credit-entry" data-kind={group.kind}>
            <div className="workbuddy-credit-line workbuddy-credit-group">
              <strong><Icon size={14} aria-hidden="true" />{label}</strong>
              <span>{number.format(group.left)}</span>
            </div>
            <div className="workbuddy-credit-line workbuddy-credit-muted">
              <span>{t('workbuddy.credits.usedOf', { used: number.format(group.used), total: number.format(group.total) })}</span>
              <span>{t('workbuddy.credits.remainingPercent', { percent: number.format(remainPercent) })}</span>
            </div>
            <div className="workbuddy-credit-track" aria-hidden="true">
              <span style={{ width: `${usedPercent}%` }} />
            </div>
            {onGroup && (
              <>
                {group.time != null && (
                  <div className="workbuddy-credit-muted">
                    {t(group.usesExpiry ? 'workbuddy.credits.nearestExpiry' : 'workbuddy.credits.nextRefresh', { time: date.format(group.time) })}
                  </div>
                )}
                {group.expiring && <span className="workbuddy-credit-expiry">{t('workbuddy.credits.expiring')}</span>}
                <button
                  type="button"
                  className="btn btn-secondary btn-sm"
                  aria-label={`${t('accounts.actions.viewDetails')} · ${label}`}
                  onClick={() => onGroup(group.kind)}
                >
                  {t('accounts.actions.viewDetails')} <ChevronRight size={14} aria-hidden="true" />
                </button>
              </>
            )}
          </div>
        );
      })}
    </div>
  );
}

function CreditPackages({ accountId, group, expired, active }: {
  accountId: string;
  group: WorkbuddyCreditGroup;
  expired: boolean;
  active: boolean;
}) {
  const { t, i18n } = useTranslation();
  const { number, date } = useCreditFormat();
  const locale = i18n.resolvedLanguage ?? i18n.language;
  const [rows, setRows] = useState<ReturnType<typeof readWorkbuddyCreditPackage>[]>([]);
  const [nextPage, setNextPage] = useState<number | null>(1);
  const [loading, setLoading] = useState(false);
  const [ready, setReady] = useState(false);
  const [failure, setFailure] = useState<{ page: number; error: string } | null>(null);
  const request = useRef(0);
  const load = useCallback(async (page: number) => {
    const version = ++request.current;
    setLoading(true);
    setFailure(null);
    try {
      const result = await getWorkbuddyCreditPackages(accountId, WORKBUDDY_CREDIT_GROUPS[group], expired, page, locale);
      if (request.current !== version) return;
      setRows((previous) => {
        const current = page === 1 ? [] : previous;
        const seen = new Set(current.map((row) => row.id).filter(Boolean));
        const items = result.items.map((item) => readWorkbuddyCreditPackage(item, expired)).filter((row) => {
          if (!row.id) return true;
          if (seen.has(row.id)) return false;
          seen.add(row.id);
          return true;
        });
        return [...current, ...items];
      });
      setReady(true);
      setNextPage(result.hasMore ? page + 1 : null);
    } catch (cause) {
      if (request.current === version) setFailure({ page, error: String(cause) });
    } finally {
      if (request.current === version) setLoading(false);
    }
  }, [accountId, expired, group, locale]);
  const started = useRef<typeof load | null>(null);
  useEffect(() => () => {
    started.current = null;
    request.current++;
  }, [load]);
  useEffect(() => {
    if (active && started.current !== load) {
      started.current = load;
      void load(1);
    }
  }, [active, load]);

  return (
    <div hidden={!active}>
      <div className="workbuddy-credits-summary workbuddy-credit-results" aria-busy={loading}>
        <ModalErrorMessage message={active && failure ? t('workbuddy.credits.loadFailed', { error: failure.error }) : null} />
        {failure && (
          <button type="button" className="btn btn-secondary btn-sm" onClick={() => void load(failure.page)}>
            {t('common.retry')}
          </button>
        )}
        {rows.map((row, index) => {
          const days = row.expireAt == null ? 0 : (row.expireAt - Date.now()) / 86_400_000;
          return (
            <div className="workbuddy-credit-entry" key={row.id || index}>
              <div className="workbuddy-credit-line">
                <strong>{t(groupKey(group))}</strong>
                <span>{t('workbuddy.credits.remaining', { amount: number.format(row.left) })}</span>
              </div>
              <div className="workbuddy-credit-line workbuddy-credit-muted">
                <span>{t('workbuddy.credits.usedOf', { used: number.format(row.used), total: number.format(row.total) })}</span>
                {row.expireAt != null && <span>{t('workbuddy.credits.expiresAt', { time: date.format(row.expireAt) })}</span>}
              </div>
              {row.name && <span className="workbuddy-credit-muted">{row.name}</span>}
              {!expired && row.left > 0 && days > 0 && days <= 3 && (
                <span className="workbuddy-credit-expiry">
                  {t(days <= 1 ? 'workbuddy.credits.expiryHours' : 'workbuddy.credits.expiryDays')}
                </span>
              )}
            </div>
          );
        })}
        {ready && rows.length === 0 && nextPage === null && (
          <div className="workbuddy-credit-empty">{t('workbuddy.credits.empty')}</div>
        )}
        {ready && !failure && nextPage !== null && (
          <button type="button" className="btn btn-secondary btn-sm" disabled={loading} onClick={() => void load(nextPage)}>
            {t('workbuddy.credits.loadMore')}
          </button>
        )}
      </div>
    </div>
  );
}

function CreditUsage({ accountId, active }: { accountId: string; active: boolean }) {
  const { t, i18n } = useTranslation();
  const { date } = useCreditFormat();
  const amount = new Intl.NumberFormat(i18n.resolvedLanguage ?? i18n.language, {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  });
  const [attempt, setAttempt] = useState(0);
  const [rows, setRows] = useState<WorkbuddyCreditUsageRow[] | null>(null);
  const [error, setError] = useState('');
  const [linkError, setLinkError] = useState('');
  useEffect(() => {
    let active = true;
    setError('');
    void getWorkbuddyRecentCreditUsage(accountId).then(
      (result) => { if (active) setRows(result); },
      (cause) => { if (active) setError(String(cause)); },
    );
    return () => { active = false; };
  }, [accountId, attempt]);

  const openHistory = async () => {
    setLinkError('');
    try {
      await openUrl('https://www.workbuddy.cn/profile/plans-usage#usage-detail');
    } catch (cause) {
      setLinkError(String(cause));
    }
  };

  return (
    <div className="workbuddy-credits-summary" aria-busy={rows === null && !error}>
      <ModalErrorMessage message={active ? linkError || (error ? t('workbuddy.credits.loadFailed', { error }) : null) : null} />
      {error && <button type="button" className="btn btn-sm btn-secondary" onClick={() => {
        setError('');
        setAttempt((value) => value + 1);
      }}>{t('common.retry')}</button>}
      <div className="workbuddy-credit-results">
        <table className="workbuddy-credit-usage">
          <thead>
            <tr>
              <th scope="col">{t('workbuddy.credits.requestAndTime')}</th>
              <th scope="col">{t('workbuddy.credits.creditUsed')}</th>
              <th scope="col">{t('workbuddy.credits.model')}</th>
            </tr>
          </thead>
          <tbody>
            {rows?.length === 0 && <tr><td colSpan={3} className="workbuddy-credit-empty">{t('workbuddy.credits.empty')}</td></tr>}
            {rows?.map((row, index) => {
              const input = row.inputTrunc || row.input;
              const time = parseDateTimeToEpoch(row.requestTime);
              const used = parseNumeric(row.credit);
              return (
                <tr key={`${row.requestId ?? ''}-${index}`}>
                  <td>
                    <div className="workbuddy-credit-request" title={row.input || input || undefined}>{input || '—'}</div>
                    <div className="workbuddy-credit-muted">
                      {time != null && Math.abs(time) <= 8.64e15 ? date.format(time) : row.requestTime || '—'}
                    </div>
                  </td>
                  <td>{used == null ? '—' : amount.format(used)}</td>
                  <td>{row.model || '—'}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
      <div className="workbuddy-credit-line">
        <span className="workbuddy-credit-muted">{t('workbuddy.credits.recentUsage')}</span>
        <button type="button" className="btn btn-sm btn-secondary" onClick={() => void openHistory()}>
          {t('workbuddy.credits.viewMore')} <ExternalLink size={13} aria-hidden="true" />
        </button>
      </div>
    </div>
  );
}

export function WorkbuddyCreditsDialog({ account, accountLabel, onClose }: {
  account: WorkbuddyAccount;
  accountLabel: string;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const dialog = useRef<HTMLDivElement>(null);
  const title = useId();
  const body = useRef<HTMLDivElement>(null);
  const [group, setGroup] = useState<WorkbuddyCreditGroup | null>(null);
  const [expired, setExpired] = useState(false);
  const [usage, setUsage] = useState(false);
  const [usageOpened, setUsageOpened] = useState(false);
  const [openedGroups, setOpenedGroups] = useState<WorkbuddyCreditGroup[]>([]);
  const credits = getWorkbuddyCredits(account);
  useEscCloseTopmost(true, onClose, dialog);
  useModalFocusTrap(dialog, true);
  useModalScrollLock(true);
  useEffect(() => {
    if (body.current) body.current.scrollTop = 0;
  }, [group, expired, usage]);
  const openGroup = (kind: WorkbuddyCreditGroup) => {
    setExpired(false);
    setGroup(kind);
    setOpenedGroups((previous) => previous.includes(kind) ? previous : [...previous, kind]);
  };
  return createPortal(
    <div className="modal-overlay workbuddy-credits-overlay">
      <div className="modal workbuddy-credits-dialog" ref={dialog} tabIndex={-1}
        role="dialog" aria-modal="true" aria-labelledby={title}>
        <div className="modal-header">
          <div className="workbuddy-credits-heading">
            <h2 id={title}>{t(group ? groupKey(group) : 'workbuddy.credits.details')}</h2>
            <span className="workbuddy-credit-muted">{accountLabel}</span>
          </div>
          <button type="button" className="modal-close" aria-label={t('common.close')} onClick={onClose}>
            <X size={20} aria-hidden="true" />
          </button>
        </div>
        <div className="modal-body workbuddy-credits-body" ref={body}>
          {group ? (
            <>
              <div className="workbuddy-credits-tabs" role="group" aria-label={t('workbuddy.credits.details')}>
                {([false, true] as const).map((value) => (
                  <button type="button" key={String(value)} aria-pressed={expired === value}
                    className="btn btn-sm workbuddy-credit-tab"
                    onClick={() => setExpired(value)}>
                    {t(value ? 'workbuddy.credits.expired' : 'workbuddy.credits.valid')}
                  </button>
                ))}
              </div>
            </>
          ) : (
            <>
              <div className="workbuddy-credits-tabs" role="group" aria-label={t('workbuddy.credits.details')}>
                {([false, true] as const).map((showUsage) => (
                  <button type="button" key={String(showUsage)} aria-pressed={usage === showUsage}
                    className="btn btn-sm workbuddy-credit-tab"
                    onClick={() => {
                      setUsage(showUsage);
                      if (showUsage) setUsageOpened(true);
                    }}>
                    {t(showUsage ? 'workbuddy.credits.usage' : 'workbuddy.credits.overview')}
                  </button>
                ))}
              </div>
              {!usage && (
                <>
                  {account.quota_query_last_error && (
                    <div className="workbuddy-credit-muted" role="status">
                      {t(credits.left == null ? 'workbuddy.quotaQuery.failedRefreshCompact' : 'common.shared.quota.cachedRefreshFailed')}
                    </div>
                  )}
                  <WorkbuddyCreditsSummary credits={credits} onGroup={openGroup} />
                  <div className="workbuddy-credits-tabs">
                    {(Object.keys(WORKBUDDY_CREDIT_GROUPS) as WorkbuddyCreditGroup[])
                      .filter((kind) => !credits.groups.some((entry) => entry.kind === kind))
                      .map((kind) => (
                        <button type="button" key={kind} className="btn btn-sm btn-secondary"
                          onClick={() => openGroup(kind)}>
                          {t(groupKey(kind))} <ChevronRight size={14} aria-hidden="true" />
                        </button>
                      ))}
                  </div>
                </>
              )}
            </>
          )}
          {usageOpened && (
            <div hidden={group != null || !usage}>
              <CreditUsage accountId={account.id} active={group == null && usage} />
            </div>
          )}
          {openedGroups.flatMap((kind) => ([false, true] as const).map((isExpired) => (
            <CreditPackages key={`${kind}-${isExpired}`} accountId={account.id} group={kind}
              expired={isExpired} active={group === kind && expired === isExpired} />
          )))}
        </div>
        <div className="modal-footer">
          {group && (
            <button type="button" className="btn btn-secondary" onClick={() => setGroup(null)}>
              {t('workbuddy.credits.overview')}
            </button>
          )}
          <button type="button" className="btn btn-secondary" onClick={onClose}>{t('common.close')}</button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
