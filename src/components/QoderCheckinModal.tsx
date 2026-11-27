import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  Ban,
  CalendarCheck,
  CheckCircle,
  Gift,
  Loader2,
  RefreshCw,
  Trophy,
  X,
  XCircle,
} from 'lucide-react';
import {
  QoderAccount,
  getQoderAccountDisplayEmail,
  getQoderPlanBadge,
  resolveQoderPlanBadgeClass,
} from '../types/qoder';
import {
  QoderCheckinStatus,
  claimQoderCnCheckin,
  getQoderCnCheckinStatus,
} from '../services/qoderCnService';
import { useEscClose } from '../hooks/useEscClose';

type CheckinUiState = 'loading' | 'available' | 'claimed' | 'inactive' | 'error';

interface AccountCheckinState {
  status: QoderCheckinStatus | null;
  uiState: CheckinUiState;
  checkingIn: boolean;
  error: string | null;
}

function resolveUiState(status: QoderCheckinStatus | null): CheckinUiState {
  if (!status) {
    return 'inactive';
  }
  if (status.checkedIn) {
    return 'claimed';
  }
  if (status.active) {
    return 'available';
  }
  return 'inactive';
}

function emptyAccountState(uiState: CheckinUiState = 'loading'): AccountCheckinState {
  return {
    status: null,
    uiState,
    checkingIn: false,
    error: null,
  };
}

interface QoderCheckinModalProps {
  accounts: QoderAccount[];
  platformLabel: string;
  onClose: () => void;
  onCheckinComplete?: (accountId: string) => void;
}

export function QoderCheckinModal({
  accounts,
  platformLabel,
  onClose,
  onCheckinComplete,
}: QoderCheckinModalProps) {
  const { t } = useTranslation();
  useEscClose(true, onClose);
  const [accountStates, setAccountStates] = useState<Record<string, AccountCheckinState>>({});
  const [checkAllLoading, setCheckAllLoading] = useState(false);
  const [refreshLoading, setRefreshLoading] = useState(false);
  // 一键签到的预检阶段还没置 checkingIn，单账号按钮仍可点击；
  // 用一个同步的 in-flight 集合兜住并发，避免同一账号被领取两次。
  const inFlightRef = useRef<Set<string>>(new Set());
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const updateAccountState = useCallback(
    (accountId: string, update: Partial<AccountCheckinState>) => {
      if (!mountedRef.current) return;
      setAccountStates((prev) => ({
        ...prev,
        [accountId]: { ...prev[accountId], ...update } as AccountCheckinState,
      }));
    },
    [],
  );

  const fetchStatus = useCallback(
    async (accountId: string) => {
      updateAccountState(accountId, { error: null, uiState: 'loading' });
      try {
        const status = await getQoderCnCheckinStatus(accountId);
        updateAccountState(accountId, {
          status,
          uiState: resolveUiState(status),
          error: null,
        });
      } catch (err) {
        updateAccountState(accountId, {
          status: null,
          uiState: 'error',
          error: err instanceof Error ? err.message : String(err),
        });
      }
    },
    [updateAccountState],
  );

  const fetchAllStatus = useCallback(async () => {
    if (!mountedRef.current) return;
    setRefreshLoading(true);
    await Promise.all(accounts.map((account) => fetchStatus(account.id)));
    if (mountedRef.current) {
      setRefreshLoading(false);
    }
  }, [accounts, fetchStatus]);

  const accountIdsKey = accounts.map((account) => account.id).join(',');

  useEffect(() => {
    if (accounts.length > 0) {
      void fetchAllStatus();
    }
    // 账号集合变化（新增/删除）后必须重新拉状态，否则新增行会一直停在 loading。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [accountIdsKey]);

  const handleCheck = useCallback(
    async (accountId: string) => {
      if (inFlightRef.current.has(accountId)) return;
      inFlightRef.current.add(accountId);
      updateAccountState(accountId, { checkingIn: true, error: null });
      try {
        const result = await claimQoderCnCheckin(accountId);
        const uiState = resolveUiState(result);
        updateAccountState(accountId, {
          status: result,
          uiState,
          checkingIn: false,
          error: null,
        });
        if (uiState === 'claimed' && mountedRef.current) {
          onCheckinComplete?.(accountId);
        }
      } catch (err) {
        const errMsg = err instanceof Error ? err.message : String(err);
        // 领取失败后回查状态：上游可能已幂等领取，只是响应异常
        try {
          const status = await getQoderCnCheckinStatus(accountId);
          const uiState = resolveUiState(status);
          updateAccountState(accountId, {
            status,
            uiState,
            checkingIn: false,
            error: uiState === 'claimed' ? null : errMsg,
          });
        } catch {
          // 回查也失败时进入 error 态，让「重试」按钮可达
          updateAccountState(accountId, {
            checkingIn: false,
            uiState: 'error',
            error: errMsg,
          });
        }
      } finally {
        inFlightRef.current.delete(accountId);
      }
    },
    [updateAccountState, onCheckinComplete],
  );

  const handleCheckAll = useCallback(async () => {
    if (!mountedRef.current) return;
    setCheckAllLoading(true);
    const targetIds: string[] = [];
    await Promise.allSettled(
      accounts.map(async (account) => {
        if (inFlightRef.current.has(account.id)) return;
        // 预检期间先置 checkingIn，避免单账号按钮在等待响应时仍可点击。
        updateAccountState(account.id, { checkingIn: true, error: null });
        try {
          const status = await getQoderCnCheckinStatus(account.id);
          updateAccountState(account.id, {
            status,
            uiState: resolveUiState(status),
            error: null,
          });
          if (status.active && !status.checkedIn) {
            targetIds.push(account.id);
          } else {
            updateAccountState(account.id, { checkingIn: false });
          }
        } catch (err) {
          console.warn('[QoderCheckin] 一键领取预检失败:', account.id, err);
          updateAccountState(account.id, {
            checkingIn: false,
            uiState: 'error',
            error: err instanceof Error ? err.message : String(err),
          });
        }
      }),
    );
    await Promise.allSettled(targetIds.map((accountId) => handleCheck(accountId)));
    if (mountedRef.current) {
      setCheckAllLoading(false);
    }
  }, [accounts, handleCheck, updateAccountState]);

  const claimedCount = accounts.filter(
    (account) => accountStates[account.id]?.uiState === 'claimed',
  ).length;
  const availableCount = accounts.filter(
    (account) => accountStates[account.id]?.uiState === 'available',
  ).length;
  const inactiveCount = accounts.filter(
    (account) => accountStates[account.id]?.uiState === 'inactive',
  ).length;
  const errorCount = accounts.filter(
    (account) => accountStates[account.id]?.uiState === 'error',
  ).length;

  return (
    <div className="modal-overlay">
      <div className="modal-content checkin-modal" onClick={(event) => event.stopPropagation()}>
        <div className="modal-header">
          <h2>
            <CalendarCheck size={20} /> {t('workbuddy.checkin.modalTitle', '每日签到')} -{' '}
            {platformLabel}
          </h2>
          <button
            className="modal-close"
            onClick={onClose}
            aria-label={t('common.close', '关闭')}
          >
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
                <Ban size={14} /> {inactiveCount} {t('workbuddy.checkin.inactive', '不可用')}
              </span>
            )}
            {errorCount > 0 && (
              <span className="checkin-stat unchecked">
                <XCircle size={14} /> {errorCount} {t('workbuddy.checkin.errors', '异常')}
              </span>
            )}
          </div>
          <div className="checkin-actions">
            <button
              className="btn btn-secondary btn-sm"
              onClick={() => void fetchAllStatus()}
              disabled={refreshLoading || checkAllLoading || accounts.length === 0}
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
              disabled={checkAllLoading || refreshLoading || accounts.length === 0}
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
            <div className="checkin-empty">{t('workbuddy.checkin.noAccounts', '暂无账号')}</div>
          ) : (
            <div className="checkin-account-list">
              {accounts.map((account) => {
                const state = accountStates[account.id] || emptyAccountState();
                const displayEmail = getQoderAccountDisplayEmail(account);
                const plan = getQoderPlanBadge(account);
                const planClass = resolveQoderPlanBadgeClass(plan);
                const uiState = state.uiState;

                return (
                  <div
                    key={account.id}
                    className={`checkin-account-row ${uiState === 'claimed' ? 'checked' : ''} ${
                      uiState === 'inactive' ? 'inactive' : ''
                    }`}
                  >
                    <div className="checkin-account-info">
                      <div className="checkin-account-name-row">
                        <span className="checkin-account-name" title={displayEmail}>
                          {displayEmail}
                        </span>
                        <span className={`tier-badge account-picker-plan raw-value ${planClass}`}>
                          {plan}
                        </span>
                      </div>
                    </div>

                    <div className="checkin-account-status">
                      {uiState === 'loading' && (
                        <span className="checkin-status-unknown">
                          {t('workbuddy.checkin.querying', '查询中...')}
                        </span>
                      )}
                      {uiState === 'error' && (
                        <span className="checkin-status-no" title={state.error || ''}>
                          <XCircle size={16} />
                          {t('workbuddy.checkin.queryFailed', '状态查询失败')}
                        </span>
                      )}
                      {uiState === 'claimed' && (
                        <span className="checkin-status-yes">
                          <CheckCircle size={16} />
                          {t('workbuddy.checkin.checkedIn', '已签到')}
                        </span>
                      )}
                      {uiState === 'available' && (
                        <span className="checkin-status-no">
                          <XCircle size={16} />
                          {t('workbuddy.checkin.notCheckedIn', '未签到')}
                        </span>
                      )}
                      {uiState === 'inactive' && (
                        <span
                          className="checkin-status-unknown"
                          title={state.status?.message || ''}
                        >
                          <Ban size={16} />
                          {t('workbuddy.checkin.inactive', '不可用')}
                        </span>
                      )}

                      {state.status?.amount != null && state.status.amount > 0 && (
                        <span className="checkin-credit-badge" title={state.status.message || ''}>
                          <Trophy size={12} />
                          {t('workbuddy.checkin.reward', '奖励：{{reward}}', {
                            reward: `${state.status.amount} Credits`,
                          })}
                        </span>
                      )}
                    </div>

                    <div className="checkin-account-action">
                      {state.checkingIn ? (
                        <button className="btn btn-primary btn-sm" disabled>
                          <Loader2 size={14} className="animate-spin" />
                          {t('workbuddy.checkin.button.loading', '签到中...')}
                        </button>
                      ) : uiState === 'claimed' ? (
                        <button className="btn btn-ghost btn-sm" disabled>
                          <CheckCircle size={14} />
                          {t('workbuddy.checkin.claimed', '已领取')}
                        </button>
                      ) : uiState === 'available' ? (
                        <button
                          className="btn btn-primary btn-sm"
                          onClick={() => void handleCheck(account.id)}
                        >
                          <Gift size={14} />
                          {t('workbuddy.checkin.button', '签到')}
                        </button>
                      ) : uiState === 'error' ? (
                        <button
                          className="btn btn-secondary btn-sm"
                          onClick={() => void fetchStatus(account.id)}
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

                    {state.error && uiState !== 'claimed' && (
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
          <button className="btn btn-secondary" onClick={onClose}>
            {t('common.close', '关闭')}
          </button>
        </div>
      </div>
    </div>
  );
}
