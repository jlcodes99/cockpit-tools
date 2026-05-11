/**
 * CodeBuddy Suite 签到弹窗
 *
 * 支持 CodeBuddy CN 和 WorkBuddy 的签到功能
 */

import { useState, useCallback, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { X, Gift, CheckCircle, XCircle, Loader2, RefreshCw, CalendarCheck, Flame, Trophy } from 'lucide-react';
import type { CodebuddySuiteAccountBase, CodebuddyAccount, WorkbuddyAccount } from '../../types/codebuddy-suite';
import type { CheckinStatusResponse, CheckinResponse } from '../../types/codebuddy';

interface CodebuddySuiteCheckinModalProps<TAccount extends CodebuddySuiteAccountBase> {
  accounts: TAccount[];
  platform: 'codebuddy_cn' | 'workbuddy';
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
  platform,
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
  const platformLabel = platform === 'codebuddy_cn' ? 'CodeBuddy CN' : 'WorkBuddy';

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal-content checkin-modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <h2><CalendarCheck size={20} /> {t('codebuddyCn.checkin.modalTitle', "Daily Check-in")} - {platformLabel}</h2>
          <button className="modal-close" onClick={onClose}><X size={18} /></button>
        </div>

        <div className="checkin-modal-toolbar">
          <div className="checkin-summary">
            <span className="checkin-stat checked">
              <CheckCircle size={14} /> {checkedCount} {t('codebuddyCn.checkin.checkedIn', "Checked In")}
            </span>
            <span className="checkin-stat unchecked">
              <XCircle size={14} /> {uncheckedCount} {t('codebuddyCn.checkin.notCheckedIn', "Not Checked In")}
            </span>
          </div>
          <div className="checkin-actions">
            <button
              className="btn btn-secondary btn-sm"
              onClick={fetchAllStatus}
              disabled={refreshLoading}
            >
              {refreshLoading ? <Loader2 size={14} className="animate-spin" /> : <RefreshCw size={14} />}
              {t('codebuddyCn.checkin.refreshStatus', "Refresh")}
            </button>
            <button
              className="btn btn-primary btn-sm"
              onClick={handleCheckAll}
              disabled={checkAllLoading || uncheckedCount === 0}
            >
              {checkAllLoading ? <Loader2 size={14} className="animate-spin" /> : <Gift size={14} />}
              {t('codebuddyCn.checkin.checkAll', "Check In All")}
            </button>
          </div>
        </div>

        <div className="modal-body checkin-modal-body">
          {accounts.length === 0 ? (
            <div className="checkin-empty">{t('codebuddyCn.checkin.noAccounts', "No accounts")}</div>
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
                        <span className="checkin-status-unknown">{t('codebuddyCn.checkin.querying', "Loading...")}</span>
                      ) : isCheckedIn ? (
                        <span className="checkin-status-yes">
                          <CheckCircle size={16} />
                          {t('codebuddyCn.checkin.checkedIn', "Checked In")}
                        </span>
                      ) : (
                        <span className="checkin-status-no">
                          <XCircle size={16} />
                          {t('codebuddyCn.checkin.notCheckedIn', "Not Checked In")}
                        </span>
                      )}

                      {state?.status && streakDays > 0 && (
                        <span className="checkin-streak-badge">
                          <Flame size={12} />
                          {t('codebuddyCn.checkin.streakDays', "{{days}} days", { days: streakDays })}
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
                            ? t('codebuddyCn.checkin.streakRewardToday', "Bonus reward available today!")
                            : t('codebuddyCn.checkin.streakRewardCountdown', "{{days}} more days for bonus reward", { days: nextStreakDay })}
                        </span>
                      )}
                    </div>

                    <div className="checkin-account-action">
                      {isCheckingIn ? (
                        <button className="btn btn-primary btn-sm" disabled>
                          <Loader2 size={14} className="animate-spin" />
                          {t('codebuddyCn.checkin.button.loading', 'Checking in...')}
                        </button>
                      ) : isCheckedIn ? (
                        <button className="btn btn-ghost btn-sm" disabled>
                          <CheckCircle size={14} />
                          {t('codebuddyCn.checkin.done', "Done")}
                        </button>
                      ) : (
                        <button className="btn btn-primary btn-sm" onClick={() => handleSingleCheckin(account.id)}>
                          <Gift size={14} />
                          {t('codebuddyCn.checkin.button', "Check In")}
                        </button>
                      )}
                    </div>

                    {state?.checkinResult && state.checkinResult.success && (
                      <div className="checkin-account-success">
                        <CheckCircle size={14} />
                        {t('codebuddyCn.checkin.success', "Check-in successful! {{days}} day streak", {
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
                        {t('codebuddyCn.checkin.recentDates', "Recent: ")}
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
            {t('common.close', "Close")}
          </button>
        </div>
      </div>
    </div>
  );
}

// 便捷导出：CodeBuddy CN 签到弹窗
import * as codebuddyCnService from '../../services/codebuddyCnService';
import { getAccountDisplayEmail } from '../../utils/codebuddy-suite';

export function CodeBuddyCNCheckinModal({
  accounts,
  onClose,
  onCheckinComplete,
}: {
  accounts: CodebuddyAccount[];
  onClose: () => void;
  onCheckinComplete?: () => void;
}) {
  return (
    <CodebuddySuiteCheckinModal
      accounts={accounts}
      platform="codebuddy_cn"
      getCheckinStatus={codebuddyCnService.getCheckinStatusCodebuddyCn}
      performCheckin={codebuddyCnService.checkinCodebuddyCn}
      getDisplayEmail={getAccountDisplayEmail}
      onClose={onClose}
      onCheckinComplete={onCheckinComplete}
    />
  );
}

// 便捷导出：WorkBuddy 签到弹窗
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
      platform="workbuddy"
      getCheckinStatus={codebuddyCnService.getCheckinStatusWorkbuddy}
      performCheckin={codebuddyCnService.checkinWorkbuddy}
      getDisplayEmail={getAccountDisplayEmail}
      onClose={onClose}
      onCheckinComplete={onCheckinComplete}
    />
  );
}