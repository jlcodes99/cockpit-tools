/**
 * CodeBuddy Suite 签到弹窗
 *
 * 支持 WorkBuddy 的签到功能
 */

import { useState, useCallback, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { X, Gift, CheckCircle, XCircle, Loader2, RefreshCw, CalendarCheck, Flame, Trophy } from 'lucide-react';
import type { CodebuddySuiteAccountBase, WorkbuddyAccount } from '../../types/codebuddy-suite';
import type { CheckinStatusResponse, CheckinResponse } from '../../types/codebuddy';

interface CodebuddySuiteCheckinModalProps<TAccount extends CodebuddySuiteAccountBase> {
  accounts: TAccount[];
  getCheckinStatus: (accountId: string) => Promise<CheckinStatusResponse>;
  performCheckin: (accountId: string) => Promise<CheckinResponse>;
  getDisplayEmail: (account: TAccount) => string;
  onClose: () => void;
  onCheckinComplete?: () => void;
}

interface AccountCheckinState {
  status: CheckinStatusResponse | null;
  loading: boolean;
  checkingIn: boolean;
  error: string | null;
  checkinResult: CheckinResponse | null;
}

export function CodebuddySuiteCheckinModal<TAccount extends CodebuddySuiteAccountBase>({
  accounts,
  getCheckinStatus,
  performCheckin,
  getDisplayEmail,
  onClose,
  onCheckinComplete,
}: CodebuddySuiteCheckinModalProps<TAccount>) {
  const { t } = useTranslation();
  const [accountStates, setAccountStates] = useState<Record<string, AccountCheckinState>>({});
  const [checkAllLoading, setCheckAllLoading] = useState(false);
  const [refreshLoading, setRefreshLoading] = useState(false);

  // 打开弹窗时自动查询所有账号签到状态
  useEffect(() => {
    if (accounts.length > 0) {
      fetchAllStatus();
    }
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const updateAccountState = useCallback((accountId: string, patch: Partial<AccountCheckinState>) => {
    setAccountStates((prev) => ({
      ...prev,
      [accountId]: { ...prev[accountId], loading: false, checkingIn: false, error: null, checkinResult: null, ...patch },
    }));
  }, []);

  const fetchAllStatus = useCallback(async () => {
    setRefreshLoading(true);
    const newStates: Record<string, AccountCheckinState> = {};

    await Promise.allSettled(
      accounts.map(async (account) => {
        try {
          const status = await getCheckinStatus(account.id);
          newStates[account.id] = { status, loading: false, checkingIn: false, error: null, checkinResult: null };
        } catch (err: any) {
          newStates[account.id] = { status: null, loading: false, checkingIn: false, error: err?.message || String(err), checkinResult: null };
        }
      }),
    );

    setAccountStates(newStates);
    setRefreshLoading(false);
  }, [accounts, getCheckinStatus]);

  const handleSingleCheckin = useCallback(async (accountId: string) => {
    updateAccountState(accountId, { checkingIn: true, error: null, checkinResult: null });
    try {
      const result = await performCheckin(accountId);
      updateAccountState(accountId, { checkingIn: false, checkinResult: result });

      if (result.success) {
        try {
          const status = await getCheckinStatus(accountId);
          updateAccountState(accountId, { status });
        } catch { /* ignore refresh error */ }
        onCheckinComplete?.();
      }
    } catch (err: any) {
      updateAccountState(accountId, { checkingIn: false, error: err?.message || String(err) });
    }
  }, [updateAccountState, performCheckin, getCheckinStatus, onCheckinComplete]);

  const handleCheckAll = useCallback(async () => {
    setCheckAllLoading(true);
    const unchecked = accounts.filter((a) => {
      const state = accountStates[a.id];
      return state?.status && !state.status.today_checked_in;
    });

    await Promise.allSettled(unchecked.map((a) => handleSingleCheckin(a.id)));
    setCheckAllLoading(false);
    onCheckinComplete?.();
  }, [accounts, accountStates, handleSingleCheckin, onCheckinComplete]);

  const checkedCount = Object.values(accountStates).filter((s) => s.status?.today_checked_in).length;
  const queriedCount = Object.keys(accountStates).length;
  const uncheckedCount = queriedCount > 0 ? accounts.length - checkedCount : accounts.length;
  const platformLabel = 'WorkBuddy';

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal-content checkin-modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <h2><CalendarCheck size={20} /> {t('workbuddy.checkin.modalTitle', '每日签到')} - {platformLabel}</h2>
          <button className="modal-close" onClick={onClose}><X size={18} /></button>
        </div>

        <div className="checkin-modal-toolbar">
          <div className="checkin-summary">
            <span className="checkin-stat checked">
              <CheckCircle size={14} /> {checkedCount} {t('workbuddy.checkin.checkedIn', '已签到')}
            </span>
            <span className="checkin-stat unchecked">
              <XCircle size={14} /> {uncheckedCount} {t('workbuddy.checkin.notCheckedIn', '未签到')}
            </span>
          </div>
          <div className="checkin-actions">
            <button
              className="btn btn-secondary btn-sm"
              onClick={fetchAllStatus}
              disabled={refreshLoading}
            >
              {refreshLoading ? <Loader2 size={14} className="animate-spin" /> : <RefreshCw size={14} />}
              {t('workbuddy.checkin.refreshStatus', '刷新状态')}
            </button>
            <button
              className="btn btn-primary btn-sm"
              onClick={handleCheckAll}
              disabled={checkAllLoading || uncheckedCount === 0}
            >
              {checkAllLoading ? <Loader2 size={14} className="animate-spin" /> : <Gift size={14} />}
              {t('workbuddy.checkin.checkAll', '一键签到')}
            </button>
          </div>
        </div>

        <div className="modal-body checkin-modal-body">
          {accounts.length === 0 ? (
            <div className="checkin-empty">{t('workbuddy.checkin.noAccounts', '暂无账号')}</div>
          ) : (
            <div className="checkin-account-list">
              {accounts.map((account) => {
                const state = accountStates[account.id];
                const displayEmail = getDisplayEmail(account);
                const isCheckingIn = state?.checkingIn ?? false;
                const isCheckedIn = state?.status?.today_checked_in ?? false;
                const streakDays = state?.status?.streak_days ?? 0;
                const dailyCredit = state?.status?.daily_credit ?? 0;
                const todayCredit = state?.status?.today_credit;
                const nextStreakDay = state?.status?.next_streak_day;
                const isStreakDay = state?.status?.is_streak_day ?? false;
                const checkinDates = state?.status?.checkin_dates;

                return (
                  <div key={account.id} className={`checkin-account-row ${isCheckedIn ? 'checked' : ''}`}>
                    <div className="checkin-account-info">
                      <span className="checkin-account-name" title={displayEmail}>{displayEmail}</span>
                    </div>

                    <div className="checkin-account-status">
                      {state === undefined || state.status === null ? (
                        <span className="checkin-status-unknown">{t('workbuddy.checkin.querying', '查询中...')}</span>
                      ) : isCheckedIn ? (
                        <span className="checkin-status-yes">
                          <CheckCircle size={16} />
                          {t('workbuddy.checkin.checkedIn', '已签到')}
                        </span>
                      ) : (
                        <span className="checkin-status-no">
                          <XCircle size={16} />
                          {t('workbuddy.checkin.notCheckedIn', '未签到')}
                        </span>
                      )}

                      {state?.status && streakDays > 0 && (
                        <span className="checkin-streak-badge">
                          <Flame size={12} />
                          {t('workbuddy.checkin.streakDays', '{{days}} 天', { days: streakDays })}
                        </span>
                      )}

                      {state?.status && dailyCredit > 0 && (
                        <span className="checkin-credit-badge">
                          <Gift size={12} />
                          +{todayCredit ?? dailyCredit}
                        </span>
                      )}

                      {nextStreakDay != null && nextStreakDay > 0 && (
                        <span className={`checkin-streak-reward ${isStreakDay ? 'streak-today' : ''}`}>
                          <Trophy size={12} />
                          {isStreakDay
                            ? t('workbuddy.checkin.streakRewardToday', '今日可获得大礼包!')
                            : t('workbuddy.checkin.streakRewardCountdown', '再签 {{days}} 天获大礼包', { days: nextStreakDay })}
                        </span>
                      )}
                    </div>

                    <div className="checkin-account-action">
                      {isCheckingIn ? (
                        <button className="btn btn-primary btn-sm" disabled>
                          <Loader2 size={14} className="animate-spin" />
                          {t('workbuddy.checkin.button.loading', '签到中...')}
                        </button>
                      ) : isCheckedIn ? (
                        <button className="btn btn-ghost btn-sm" disabled>
                          <CheckCircle size={14} />
                          {t('workbuddy.checkin.done', '已完成')}
                        </button>
                      ) : (
                        <button className="btn btn-primary btn-sm" onClick={() => handleSingleCheckin(account.id)}>
                          <Gift size={14} />
                          {t('workbuddy.checkin.button', '签到')}
                        </button>
                      )}
                    </div>

                    {state?.checkinResult && state.checkinResult.success && (
                      <div className="checkin-account-success">
                        <CheckCircle size={14} />
                        {t('workbuddy.checkin.success', '签到成功！连续签到 {{days}} 天', {
                          days: state.status?.streak_days ?? 0,
                        })}
                      </div>
                    )}

                    {state?.checkinResult && !state.checkinResult.success && state.checkinResult.message && (
                      <div className="checkin-account-info-msg">
                        <XCircle size={12} /> {state.checkinResult.message}
                      </div>
                    )}

                    {state?.checkinResult?.reward && (
                      <div className="checkin-reward-badge">
                        <Trophy size={12} />
                        <span>
                          {typeof state.checkinResult.reward === 'object'
                            ? JSON.stringify(state.checkinResult.reward)
                            : String(state.checkinResult.reward)}
                        </span>
                      </div>
                    )}

                    {checkinDates && checkinDates.length > 0 && (
                      <div className="checkin-dates">
                        {t('workbuddy.checkin.recentDates', '近期签到：')}
                        {checkinDates.slice(0, 5).map((d) => (
                          <span key={d} className="checkin-date-tag">{d}</span>
                        ))}
                        {checkinDates.length > 5 && (
                          <span
                            className="checkin-date-tag"
                            title={checkinDates.slice(5).join(', ')}
                          >
                            ...
                          </span>
                        )}
                      </div>
                    )}

                    {state?.error && (
                      <div className="checkin-account-error">
                        <XCircle size={12} /> {state.error}
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          )}
        </div>

        <div className="modal-footer">
          <button className="btn btn-secondary" onClick={onClose}>
            {t('common.close', '关闭')}
          </button>
        </div>
      </div>
    </div>
  );
}

// 便捷导出：WorkBuddy 签到弹窗
import * as codebuddyCnService from '../../services/codebuddyCnService';
import { getAccountDisplayEmail } from '../../utils/codebuddy-suite';
export function WorkbuddyCheckinModal({
  accounts,
  onClose,
  onCheckinComplete,
}: {
  accounts: WorkbuddyAccount[];
  onClose: () => void;
  onCheckinComplete?: () => void;
}) {
  return (
    <CodebuddySuiteCheckinModal
      accounts={accounts}
      getCheckinStatus={codebuddyCnService.getCheckinStatusWorkbuddy}
      performCheckin={codebuddyCnService.checkinWorkbuddy}
      getDisplayEmail={getAccountDisplayEmail}
      onClose={onClose}
      onCheckinComplete={onCheckinComplete}
    />
  );
}
