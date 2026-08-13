/**
 * Trae SOLO CN 签到弹窗
 *
 * 基于 CodebuddySuiteCheckinModal 模式，适配 Trae 的签到 API。
 */

import { useState, useCallback, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
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
} from 'lucide-react';
import { TraeAccount } from '../../types/trae';
import { TraeCheckinStatusResult, getTraeCheckinStatus, claimTraeCheckin } from '../../services/traeService';
import { useEscClose } from '../../hooks/useEscClose';
import { getTraeAccountDisplayEmail } from '../../types/trae';

type CheckinUiState = 'loading' | 'available' | 'claimed' | 'inactive' | 'error';

interface AccountCheckinState {
  status: TraeCheckinStatusResult | null;
  uiState: CheckinUiState;
  checkingIn: boolean;
  error: string | null;
}

function resolveUiState(status: TraeCheckinStatusResult | null): CheckinUiState {
  if (!status) {
    return 'inactive';
  }
  if (status.checked_in) {
    return 'claimed';
  }
  return 'available';
}

function emptyAccountState(uiState: CheckinUiState = 'loading'): AccountCheckinState {
  return {
    status: null,
    uiState,
    checkingIn: false,
    error: null,
  };
}

interface TraeCheckinModalProps {
  accounts: TraeAccount[];
  onClose: () => void;
  onCheckinComplete?: () => void;
}

export function TraeCheckinModal({
  accounts,
  onClose,
  onCheckinComplete,
}: TraeCheckinModalProps) {
  const { t } = useTranslation();
  useEscClose(true, onClose);
  const [accountStates, setAccountStates] = useState<Record<string, AccountCheckinState>>({});
  const [checkAllLoading, setCheckAllLoading] = useState(false);
  const [refreshLoading, setRefreshLoading] = useState(false);

  const updateAccountState = useCallback(
    (accountId: string, update: Partial<AccountCheckinState>) => {
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
        const status = await getTraeCheckinStatus(accountId);
        const uiState = resolveUiState(status);
        updateAccountState(accountId, { status, uiState, error: null });
      } catch (err) {
        const errMsg = err instanceof Error ? err.message : String(err);
        updateAccountState(accountId, {
          status: null,
          uiState: 'error',
          error: errMsg,
        });
      }
    },
    [updateAccountState],
  );

  const fetchAllStatus = useCallback(async () => {
    setRefreshLoading(true);
    await Promise.all(accounts.map((account) => fetchStatus(account.id)));
    setRefreshLoading(false);
  }, [accounts, fetchStatus]);

  const handleCheck = useCallback(
    async (accountId: string) => {
      updateAccountState(accountId, { checkingIn: true, error: null });
      try {
        const result = await claimTraeCheckin(accountId);
        updateAccountState(accountId, {
          status: result,
          uiState: 'claimed',
          checkingIn: false,
          error: null,
        });
        onCheckinComplete?.();
      } catch (err) {
        const errMsg = err instanceof Error ? err.message : String(err);
        // 重新查询状态
        try {
          const status = await getTraeCheckinStatus(accountId);
          const uiState = resolveUiState(status);
          updateAccountState(accountId, {
            status,
            uiState,
            checkingIn: false,
            error: uiState === 'claimed' ? null : errMsg,
          });
        } catch {
          updateAccountState(accountId, {
            checkingIn: false,
            error: errMsg,
          });
        }
      }
    },
    [updateAccountState, onCheckinComplete],
  );

  const handleCheckAll = useCallback(async () => {
    setCheckAllLoading(true);
    const availableAccounts = accounts.filter((account) => {
      const state = accountStates[account.id];
      return state?.uiState === 'available';
    });
    for (const account of availableAccounts) {
      await handleCheck(account.id);
    }
    setCheckAllLoading(false);
  }, [accounts, accountStates, handleCheck]);

  useEffect(() => {
    void fetchAllStatus();
  }, [fetchAllStatus]);

  const claimedCount = accounts.filter((account) => {
    const state = accountStates[account.id];
    return state?.uiState === 'claimed';
  }).length;

  const availableCount = accounts.filter((account) => {
    const state = accountStates[account.id];
    return state?.uiState === 'available';
  }).length;

  const errorCount = accounts.filter((account) => {
    const state = accountStates[account.id];
    return state?.uiState === 'error';
  }).length;

  const getDisplayEmail = useCallback(
    (account: TraeAccount) => getTraeAccountDisplayEmail(account),
    [],
  );

  return (
    <div className="modal-overlay" onClick={onClose}>
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
            <CalendarCheck size={20} /> {t('trae.checkin.modalTitle', 'TRAE SOLO CN 每日签到')}
          </h2>
          <button className="modal-close" onClick={onClose}>
            <X size={18} />
          </button>
        </div>

        <div className="checkin-modal-toolbar">
          <div className="checkin-summary">
            <span className="checkin-stat checked">
              <CheckCircle size={14} /> {claimedCount} {t('workbuddy.checkin.checkedIn', '已签到')}
            </span>
            <span className="checkin-stat unchecked">
              <XCircle size={14} /> {availableCount} {t('workbuddy.checkin.notCheckedIn', '未签到')}
            </span>
            {errorCount > 0 && (
              <span className="checkin-stat error">
                <Ban size={14} /> {errorCount} {t('workbuddy.checkin.errors', '异常')}
              </span>
            )}
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

        <div className="checkin-list">
          {accounts.map((account) => {
            const state = accountStates[account.id] || emptyAccountState();
            const displayEmail = getDisplayEmail(account);

            return (
              <div key={account.id} className="checkin-account-row">
                <div className="checkin-account-info">
                  <span className="checkin-account-email">{displayEmail}</span>
                  {state.status && (
                    <span className="checkin-account-streak">
                      <Flame size={12} />
                      {t('workbuddy.checkin.streakDays', '连续 {{days}} 天', {
                        days: state.status.consecutive_days,
                      })}
                    </span>
                  )}
                  {state.status && state.status.total_credits > 0 && (
                    <span className="checkin-account-credits">
                      <Trophy size={12} />
                      {t('workbuddy.checkin.totalCredits', '{{credits}} 积分', {
                        credits: state.status.total_credits,
                      })}
                    </span>
                  )}
                </div>

                <div className="checkin-account-action">
                  {state.uiState === 'loading' && (
                    <Loader2 size={16} className="animate-spin" />
                  )}
                  {state.uiState === 'error' && (
                    <span className="checkin-error-text" title={state.error || ''}>
                      {t('workbuddy.checkin.error', '查询失败')}
                    </span>
                  )}
                  {state.uiState === 'inactive' && (
                    <span className="checkin-inactive">
                      <Ban size={14} />
                      {t('workbuddy.checkin.inactive', '不可用')}
                    </span>
                  )}
                  {state.uiState === 'claimed' && (
                    <span className="checkin-claimed">
                      <CheckCircle size={14} />
                      {t('workbuddy.checkin.claimed', '已领取')}
                    </span>
                  )}
                  {state.uiState === 'available' && (
                    <button
                      className="btn btn-primary btn-sm"
                      onClick={() => void handleCheck(account.id)}
                      disabled={state.checkingIn}
                    >
                      {state.checkingIn ? (
                        <Loader2 size={14} className="animate-spin" />
                      ) : (
                        <Gift size={14} />
                      )}
                      {t('workbuddy.checkin.checkin', '签到')}
                    </button>
                  )}
                </div>
              </div>
            );
          })}
        </div>

        {accounts.length === 0 && (
          <div className="checkin-empty">
            <p>{t('workbuddy.checkin.noAccounts', '暂无账号')}</p>
          </div>
        )}
      </div>
    </div>
  );
}
