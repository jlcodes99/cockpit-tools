/**
 * CodeBuddy Suite 签到弹窗
 *
 * 状态机对齐官方 WorkBuddy `useCheckinService` / DailyCheckin：
 * - inactive：无 data 或 active 显式关闭
 * - available：活动可用且今日未领（today_checked_in === false）
 * - claimed：今日已领（today_checked_in === true）→ 官方按钮文案「已领取」
 * - claim 成功：先本地设 claimed + today_checked_in=true，再后台 refresh
 * - loading / error：查询中或查询失败
 */

import { useState, useCallback, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { listen } from '@tauri-apps/api/event';
import {
  X,
  ChevronLeft,
  Gift,
  CheckCircle,
  XCircle,
  Loader2,
  RefreshCw,
  CalendarCheck,
  Flame,
  Trophy,
  Ban,
  Settings,
  Clock,
} from 'lucide-react';
import type { CodebuddySuiteAccountBase, WorkbuddyAccount } from '../../types/codebuddy-suite';
import type { CheckinStatusResponse, CheckinResponse } from '../../types/codebuddy';
import { useEscClose } from '../../hooks/useEscClose';
import { WorkbuddyAutoCheckinConfigModal } from './WorkbuddyAutoCheckinConfigModal';
import {
  getWorkbuddyAutoCheckinConfig,
  getWorkbuddyAutoCheckinConfigAsync,
  saveWorkbuddyAutoCheckinConfigAsync,
  WORKBUDDY_AUTO_CHECKIN_CONFIG_CHANGED_EVENT,
  WorkbuddyAutoCheckinConfig,
} from '../../services/workbuddyAutoCheckinService';

interface CodebuddySuiteCheckinModalProps<TAccount extends CodebuddySuiteAccountBase> {
  accounts: TAccount[];
  getCheckinStatus: (accountId: string) => Promise<CheckinStatusResponse>;
  performCheckin: (accountId: string) => Promise<CheckinResponse>;
  getDisplayEmail: (account: TAccount) => string;
  onClose: () => void;
  onCheckinComplete?: () => void;
}

/** 与官方 WorkBuddy uiState 对齐 */
type CheckinUiState = 'loading' | 'available' | 'claimed' | 'inactive' | 'error';

interface AccountCheckinState {
  status: CheckinStatusResponse | null;
  uiState: CheckinUiState;
  checkingIn: boolean;
  error: string | null;
  checkinResult: CheckinResponse | null;
}

function resolveUiState(status: CheckinStatusResponse | null): CheckinUiState {
  if (!status) {
    return 'inactive';
  }
  // 官方：今日已领优先显示 claimed（主按钮「已领取」）
  // 注意：管理多账号时，即使 active 字段缺省/异常，只要 today_checked_in 为真就应显示已领
  if (status.today_checked_in) {
    return 'claimed';
  }
  // 官方：!data.active → inactive（活动未开启）
  if (status.active !== true) {
    return 'inactive';
  }
  return 'available';
}

function emptyAccountState(uiState: CheckinUiState = 'loading'): AccountCheckinState {
  return {
    status: null,
    uiState,
    checkingIn: false,
    error: null,
    checkinResult: null,
  };
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
  useEscClose(true, onClose);
  const [accountStates, setAccountStates] = useState<Record<string, AccountCheckinState>>({});
  const [checkAllLoading, setCheckAllLoading] = useState(false);
  const [refreshLoading, setRefreshLoading] = useState(false);
  const [showConfigModal, setShowConfigModal] = useState(false);
  const [autoCheckinConfig, setAutoCheckinConfig] = useState<WorkbuddyAutoCheckinConfig>(() =>
    getWorkbuddyAutoCheckinConfig(),
  );

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    const handleConfigChange = () => {
      void getWorkbuddyAutoCheckinConfigAsync().then((nextConfig) => {
        if (!disposed) {
          setAutoCheckinConfig(nextConfig);
        }
      });
    };
    handleConfigChange();
    window.addEventListener(WORKBUDDY_AUTO_CHECKIN_CONFIG_CHANGED_EVENT, handleConfigChange);
    void listen(WORKBUDDY_AUTO_CHECKIN_CONFIG_CHANGED_EVENT, handleConfigChange)
      .then((stopListening) => {
        if (disposed) {
          stopListening();
        } else {
          unlisten = stopListening;
        }
      })
      .catch((err) => {
        console.warn('[WorkbuddyAutoCheckin] 监听后端签到配置事件失败:', err);
      });
    return () => {
      disposed = true;
      unlisten?.();
      window.removeEventListener(WORKBUDDY_AUTO_CHECKIN_CONFIG_CHANGED_EVENT, handleConfigChange);
    };
  }, []);

  useEffect(() => {
    if (accounts.length > 0) {
      void fetchAllStatus();
    }
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const updateAccountState = useCallback(
    (accountId: string, patch: Partial<AccountCheckinState>) => {
      setAccountStates((prev) => {
        const previous = prev[accountId] ?? emptyAccountState();
        const next: AccountCheckinState = {
          ...previous,
          ...patch,
        };
        // 若只更新了 status 且未显式指定 uiState，按官方规则重算
        if (patch.status !== undefined && patch.uiState === undefined) {
          next.uiState = resolveUiState(patch.status);
        }
        return {
          ...prev,
          [accountId]: next,
        };
      });
    },
    [],
  );

  const fetchAllStatus = useCallback(async () => {
    setRefreshLoading(true);
    const newStates: Record<string, AccountCheckinState> = {};

    // 先进入 loading，避免旧状态闪烁
    for (const account of accounts) {
      newStates[account.id] = emptyAccountState('loading');
    }
    setAccountStates(newStates);

    await Promise.allSettled(
      accounts.map(async (account) => {
        try {
          const status = await getCheckinStatus(account.id);
          newStates[account.id] = {
            status,
            uiState: resolveUiState(status),
            checkingIn: false,
            error: null,
            checkinResult: null,
          };
        } catch (err: unknown) {
          const message = err instanceof Error ? err.message : String(err);
          newStates[account.id] = {
            status: null,
            uiState: 'error',
            checkingIn: false,
            error: message,
            checkinResult: null,
          };
        }
      }),
    );

    setAccountStates({ ...newStates });
    setRefreshLoading(false);
  }, [accounts, getCheckinStatus]);

  const refreshOneStatus = useCallback(
    async (accountId: string, keepResult?: CheckinResponse | null) => {
      try {
        const status = await getCheckinStatus(accountId);
        updateAccountState(accountId, {
          status,
          uiState: resolveUiState(status),
          error: null,
          checkinResult: keepResult ?? null,
          checkingIn: false,
        });
        return status;
      } catch (err: unknown) {
        const message = err instanceof Error ? err.message : String(err);
        updateAccountState(accountId, {
          checkingIn: false,
          error: message,
          // 查询失败：不假装未签到；保留上一状态，uiState=error
          uiState: 'error',
          checkinResult: keepResult ?? null,
        });
        return null;
      }
    },
    [getCheckinStatus, updateAccountState],
  );

  const handleSingleCheckin = useCallback(
    async (accountId: string) => {
      updateAccountState(accountId, {
        checkingIn: true,
        error: null,
        checkinResult: null,
      });
      try {
        const result = await performCheckin(accountId);

        if (result.success) {
        // 官方 handleClaim：先本地 claimed + today_checked_in=true，再 background refresh
        updateAccountState(accountId, {
          checkingIn: false,
          error: null,
          checkinResult: result,
          uiState: 'claimed',
          status: {
            today_checked_in: true,
            active: true,
            streak_days: result.streak_days ?? 0,
            daily_credit: result.credit ?? 0,
            today_credit: result.credit ?? null,
            next_streak_day: null,
            is_streak_day: result.is_streak_day ?? null,
            checkin_dates: null,
            streak_bonus_days: null,
            streak_bonus_credit: null,
          },
        });
        // 后台刷新；失败则保留本地 claimed（与官方 refreshStatus().catch(ignored) 一致）
        try {
          const remote = await getCheckinStatus(accountId);
          const merged: CheckinStatusResponse = {
            ...remote,
            // 服务端短暂未更新时仍视为今日已领
            today_checked_in: remote.today_checked_in || true,
            active: remote.active !== false,
          };
          updateAccountState(accountId, {
            status: merged,
            uiState: resolveUiState(merged),
            checkinResult: result,
            error: null,
            checkingIn: false,
          });
        } catch {
          updateAccountState(accountId, { checkingIn: false, uiState: 'claimed' });
        }
        onCheckinComplete?.();
        return;
      }

        // 业务失败（如已签到）：刷新状态对齐官方
        const alreadyChecked = /已签到|already\s*check|already\s*claim/i.test(
          result.message || '',
        );
        updateAccountState(accountId, {
          checkingIn: false,
          checkinResult: result,
          error: null,
          ...(alreadyChecked
            ? {
                status: {
                  today_checked_in: true,
                  active: true,
                  streak_days: result.streak_days ?? 0,
                  daily_credit: result.credit ?? 0,
                  today_credit: result.credit ?? null,
                  next_streak_day: null,
                  is_streak_day: result.is_streak_day ?? null,
                  checkin_dates: null,
                  streak_bonus_days: null,
                  streak_bonus_credit: null,
                },
                uiState: 'claimed' as const,
              }
            : {}),
        });
        await refreshOneStatus(accountId, result);
      } catch (err: unknown) {
        const message = err instanceof Error ? err.message : String(err);
        updateAccountState(accountId, {
          checkingIn: false,
          error: message,
        });
      }
    },
    [updateAccountState, performCheckin, refreshOneStatus, onCheckinComplete],
  );

  const handleCheckAll = useCallback(async () => {
    setCheckAllLoading(true);
    // 官方：仅 available 可领取
    const available = accounts.filter((a) => {
      const state = accountStates[a.id];
      return state?.uiState === 'available';
    });

    await Promise.allSettled(available.map((a) => handleSingleCheckin(a.id)));
    setCheckAllLoading(false);
    onCheckinComplete?.();
  }, [accounts, accountStates, handleSingleCheckin, onCheckinComplete]);

  const claimedCount = Object.values(accountStates).filter(
    (s) => s.uiState === 'claimed',
  ).length;
  const availableCount = Object.values(accountStates).filter(
    (s) => s.uiState === 'available',
  ).length;
  const inactiveCount = Object.values(accountStates).filter(
    (s) => s.uiState === 'inactive',
  ).length;
  const platformLabel = 'WorkBuddy';

  return (
    <div className="modal-overlay">
      <div className="modal-content checkin-modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <button
            className="btn btn-secondary icon-only"
            onClick={onClose}
            title={t('common.back', '返回')}
            aria-label={t('common.back', '返回')}
          >
            <ChevronLeft size={14} />
          </button>
          <h2>
            <CalendarCheck size={20} /> {t('workbuddy.checkin.modalTitle', '每日签到')} -{' '}
            {platformLabel}
          </h2>
          <button className="modal-close" onClick={onClose}>
            <X size={18} />
          </button>
        </div>

        <div className="checkin-modal-toolbar">
          <div className="checkin-summary">
            <span className="checkin-stat checked">
              <CheckCircle size={14} /> {claimedCount}{' '}
              {t('workbuddy.checkin.checkedIn', '已签到')}
            </span>
            <span className="checkin-stat unchecked">
              <XCircle size={14} /> {availableCount}{' '}
              {t('workbuddy.checkin.notCheckedIn', '未签到')}
            </span>
            {inactiveCount > 0 && (
              <span className="checkin-stat inactive">
                <Ban size={14} /> {inactiveCount}{' '}
                {t('workbuddy.checkin.inactive', '不可用')}
              </span>
            )}
            <span
              className={`checkin-stat auto-checkin-badge ${
                autoCheckinConfig.enabled ? 'enabled' : 'disabled'
              }`}
              onClick={() => setShowConfigModal(true)}
              role="button"
              tabIndex={0}
              onKeyDown={(e) => {
                if (e.key === 'Enter' || e.key === ' ') {
                  e.preventDefault();
                  setShowConfigModal(true);
                }
              }}
              title={
                autoCheckinConfig.enabled
                  ? t(
                      'workbuddy.checkin.autoCheckinEnabledHint',
                      '自动签到已开启：将在 {{start}} 至 {{end}} 随机签到（点击设置）',
                      {
                        start: autoCheckinConfig.startTime,
                        end: autoCheckinConfig.endTime,
                      },
                    )
                  : t('workbuddy.checkin.autoCheckinDisabledHint', '自动签到未开启（点击设置）')
              }
            >
              <Clock size={14} />
              {autoCheckinConfig.enabled
                ? `${t('workbuddy.checkin.autoCheckinLabel', '自动签到')} (${
                    autoCheckinConfig.startTime
                  }-${autoCheckinConfig.endTime})`
                : t('workbuddy.checkin.autoCheckinOff', '自动签到未开启')}
            </span>
          </div>
          <div className="checkin-actions">
            <button
              className="btn btn-secondary btn-sm"
              onClick={() => void fetchAllStatus()}
              disabled={refreshLoading}
            >
              {refreshLoading ? (
                <Loader2 size={14} className="animate-spin" />
              ) : (
                <RefreshCw size={14} />
              )}
              {t('workbuddy.checkin.refreshStatus', '刷新状态')}
            </button>
            <button
              className="btn btn-primary btn-sm"
              onClick={() => void handleCheckAll()}
              disabled={checkAllLoading || availableCount === 0}
            >
              {checkAllLoading ? (
                <Loader2 size={14} className="animate-spin" />
              ) : (
                <Gift size={14} />
              )}
              {t('workbuddy.checkin.checkAll', '一键签到')}
            </button>
          </div>
        </div>

        <div className="modal-body checkin-modal-body">
          {accounts.length === 0 ? (
            <div className="checkin-empty">
              {t('workbuddy.checkin.noAccounts', '暂无账号')}
            </div>
          ) : (
            <div className="checkin-account-list">
              {accounts.map((account) => {
                const state = accountStates[account.id];
                const displayEmail = getDisplayEmail(account);
                const uiState: CheckinUiState = state?.uiState ?? 'loading';
                const isCheckingIn = state?.checkingIn ?? false;
                const isClaimed = uiState === 'claimed';
                const isAvailable = uiState === 'available';
                const isInactive = uiState === 'inactive';
                const isError = uiState === 'error';
                const isLoading = uiState === 'loading' || state === undefined;

                const streakDays = state?.status?.streak_days ?? 0;
                const dailyCredit = state?.status?.daily_credit ?? 0;
                const todayCredit = state?.status?.today_credit;
                const displayCredit = todayCredit ?? dailyCredit;
                const nextStreakDay = state?.status?.next_streak_day;
                const isStreakDay = state?.status?.is_streak_day ?? false;
                const checkinDates = state?.status?.checkin_dates;
                // 仅 active 状态展示连续/奖励信息（与官方一致）
                const showMeta = !!state?.status?.active;

                return (
                  <div
                    key={account.id}
                    className={`checkin-account-row ${isClaimed ? 'checked' : ''} ${
                      isInactive ? 'inactive' : ''
                    }`}
                  >
                    <div className="checkin-account-info">
                      <span className="checkin-account-name" title={displayEmail}>
                        {displayEmail}
                      </span>
                    </div>

                    <div className="checkin-account-status">
                      {isLoading ? (
                        <span className="checkin-status-unknown">
                          {t('workbuddy.checkin.querying', '查询中...')}
                        </span>
                      ) : isError ? (
                        <span className="checkin-status-no">
                          <XCircle size={16} />
                          {t('workbuddy.checkin.queryFailed', '状态查询失败')}
                        </span>
                      ) : isClaimed ? (
                        <span className="checkin-status-yes">
                          <CheckCircle size={16} />
                          {t('workbuddy.checkin.checkedIn', '已签到')}
                        </span>
                      ) : isAvailable ? (
                        <span className="checkin-status-no">
                          <XCircle size={16} />
                          {t('workbuddy.checkin.notCheckedIn', '未签到')}
                        </span>
                      ) : (
                        <span className="checkin-status-unknown">
                          <Ban size={16} />
                          {t('workbuddy.checkin.inactive', '不可用')}
                        </span>
                      )}

                      {showMeta && streakDays > 0 && (
                        <span className="checkin-streak-badge">
                          <Flame size={12} />
                          {t('workbuddy.checkin.streakDays', '{{days}} 天', {
                            days: streakDays,
                          })}
                        </span>
                      )}

                      {showMeta && displayCredit > 0 && (
                        <span className="checkin-credit-badge">
                          <Gift size={12} />+{displayCredit}
                        </span>
                      )}

                      {showMeta && nextStreakDay != null && nextStreakDay > 0 && (
                        <span
                          className={`checkin-streak-reward ${
                            isStreakDay ? 'streak-today' : ''
                          }`}
                        >
                          <Trophy size={12} />
                          {isStreakDay
                            ? t(
                                'workbuddy.checkin.streakRewardToday',
                                '今日可获得大礼包!',
                              )
                            : t(
                                'workbuddy.checkin.streakRewardCountdown',
                                '再签 {{days}} 天获大礼包',
                                { days: nextStreakDay },
                              )}
                        </span>
                      )}
                    </div>

                    <div className="checkin-account-action">
                      {isCheckingIn ? (
                        <button className="btn btn-primary btn-sm" disabled>
                          <Loader2 size={14} className="animate-spin" />
                          {t('workbuddy.checkin.button.loading', '签到中...')}
                        </button>
                      ) : isClaimed ? (
                        <button className="btn btn-ghost btn-sm" disabled>
                          <CheckCircle size={14} />
                          {t('workbuddy.checkin.claimed', '已领取')}
                        </button>
                      ) : isAvailable ? (
                        <button
                          className="btn btn-primary btn-sm"
                          onClick={() => void handleSingleCheckin(account.id)}
                        >
                          <Gift size={14} />
                          {t('workbuddy.checkin.button', '签到')}
                        </button>
                      ) : isError ? (
                        <button
                          className="btn btn-secondary btn-sm"
                          onClick={() => void refreshOneStatus(account.id)}
                        >
                          <RefreshCw size={14} />
                          {t('workbuddy.checkin.retry', '重试')}
                        </button>
                      ) : (
                        <button className="btn btn-ghost btn-sm" disabled>
                          <Ban size={14} />
                          {t('workbuddy.checkin.inactive', '不可用')}
                        </button>
                      )}
                    </div>

                    {isInactive && !isError && (
                      <div className="checkin-account-info-msg">
                        {t(
                          'workbuddy.checkin.inactiveHint',
                          '签到活动未开启或不适用当前账号',
                        )}
                      </div>
                    )}

                    {state?.checkinResult && state.checkinResult.success && (
                      <div className="checkin-account-success">
                        <CheckCircle size={14} />
                        {t('workbuddy.checkin.success', '签到成功！连续签到 {{days}} 天', {
                          days: state.status?.streak_days ?? 0,
                        })}
                      </div>
                    )}

                    {state?.checkinResult &&
                      !state.checkinResult.success &&
                      state.checkinResult.message && (
                        <div className="checkin-account-info-msg">
                          <XCircle size={12} /> {state.checkinResult.message}
                        </div>
                      )}

                    {(state?.checkinResult?.credit != null ||
                      state?.checkinResult?.reward) && (
                      <div className="checkin-reward-badge">
                        <Trophy size={12} />
                        <span>
                          {state.checkinResult.credit != null
                            ? `+${state.checkinResult.credit}`
                            : typeof state.checkinResult.reward === 'object'
                              ? JSON.stringify(state.checkinResult.reward)
                              : String(state.checkinResult.reward)}
                        </span>
                      </div>
                    )}

                    {showMeta && checkinDates && checkinDates.length > 0 && (
                      <div className="checkin-dates">
                        {t('workbuddy.checkin.recentDates', '近期签到：')}
                        {checkinDates.slice(0, 5).map((d) => (
                          <span key={d} className="checkin-date-tag">
                            {d}
                          </span>
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

        <div className="modal-footer checkin-modal-footer">
          <button
            className="btn btn-secondary icon-only"
            onClick={() => setShowConfigModal(true)}
            title={t('workbuddy.checkin.autoCheckinSettings', '自动签到设置')}
            aria-label={t('workbuddy.checkin.autoCheckinSettings', '自动签到设置')}
          >
            <Settings size={14} />
          </button>
          <button className="btn btn-secondary" onClick={onClose}>
            {t('common.close', '关闭')}
          </button>
        </div>

        {showConfigModal && (
          <WorkbuddyAutoCheckinConfigModal
            config={autoCheckinConfig}
            onSave={async (newConfig) => {
              await saveWorkbuddyAutoCheckinConfigAsync(newConfig);
              setAutoCheckinConfig(newConfig);
            }}
            onClose={() => setShowConfigModal(false)}
          />
        )}
      </div>
    </div>
  );
}

// 便捷导出：WorkBuddy 签到弹窗
import * as workbuddyService from '../../services/workbuddyService';
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
      getCheckinStatus={workbuddyService.getCheckinStatusWorkbuddy}
      performCheckin={workbuddyService.checkinWorkbuddy}
      getDisplayEmail={getAccountDisplayEmail}
      onClose={onClose}
      onCheckinComplete={onCheckinComplete}
    />
  );
}
