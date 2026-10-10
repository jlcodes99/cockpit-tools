/**
 * Trae SOLO CN 签到弹窗
 *
 * 基于 CodebuddySuiteCheckinModal 模式，适配 Trae 的签到 API。
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
  Trophy,
  Ban,
  Settings,
  Clock,
} from 'lucide-react';
import { TraeAccount, getTraeAccountDisplayLabel } from '../../types/trae';
import { TraeCheckinStatusResult, getTraeCheckinStatus, claimTraeCheckin } from '../../services/traeService';
import {
  getTraeAutoCheckinConfig,
  getTraeAutoCheckinConfigAsync,
  saveTraeAutoCheckinConfigAsync,
  TRAE_AUTO_CHECKIN_CONFIG_CHANGED_EVENT,
  type TraeAutoCheckinConfig,
} from '../../services/traeAutoCheckinService';
import { useEscClose } from '../../hooks/useEscClose';
import { TraeAutoCheckinConfigModal } from './TraeAutoCheckinConfigModal';

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
  if (status.active === false) {
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
  };
}

interface TraeCheckinModalProps {
  accounts: TraeAccount[];
  platformLabel: string;
  onClose: () => void;
  onCheckinComplete?: () => void;
}

export function TraeCheckinModal({
  accounts,
  platformLabel,
  onClose,
  onCheckinComplete,
}: TraeCheckinModalProps) {
  const { t } = useTranslation();
  useEscClose(true, onClose);
  const [accountStates, setAccountStates] = useState<Record<string, AccountCheckinState>>({});
  const [checkAllLoading, setCheckAllLoading] = useState(false);
  const [refreshLoading, setRefreshLoading] = useState(false);
  const [showConfigModal, setShowConfigModal] = useState(false);
  const [autoCheckinConfig, setAutoCheckinConfig] = useState<TraeAutoCheckinConfig>(() =>
    getTraeAutoCheckinConfig(),
  );

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    const handleConfigChange = () => {
      void getTraeAutoCheckinConfigAsync().then((nextConfig) => {
        if (!disposed) {
          setAutoCheckinConfig(nextConfig);
        }
      });
    };
    handleConfigChange();
    void listen(TRAE_AUTO_CHECKIN_CONFIG_CHANGED_EVENT, handleConfigChange)
      .then((stopListening) => {
        if (disposed) {
          stopListening();
        } else {
          unlisten = stopListening;
        }
      })
      .catch((err) => {
        console.warn('[TraeAutoCheckin] 监听后端签到配置事件失败:', err);
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

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
        updateAccountState(accountId, { status, uiState: resolveUiState(status), error: null });
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

  // 仅挂载时拉取一次状态
  useEffect(() => {
    if (accounts.length > 0) {
      void fetchAllStatus();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

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
        // 领取失败后回查状态：9095 已签到等场景服务端已幂等处理
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
    const targetIds: string[] = [];
    await Promise.allSettled(
      accounts.map(async (account) => {
        try {
          const status = await getTraeCheckinStatus(account.id);
          updateAccountState(account.id, {
            status,
            uiState: resolveUiState(status),
            error: null,
          });
          if (!status.checked_in && status.active !== false) {
            targetIds.push(account.id);
          }
        } catch (err) {
          console.warn('[TraeCheckin] 一键签到预检失败:', account.id, err);
        }
      }),
    );
    await Promise.allSettled(targetIds.map((accountId) => handleCheck(accountId)));
    setCheckAllLoading(false);
  }, [accounts, handleCheck, updateAccountState]);

  const claimedCount = accounts.filter((account) => {
    const state = accountStates[account.id];
    return state?.uiState === 'claimed';
  }).length;

  const availableCount = accounts.filter((account) => {
    const state = accountStates[account.id];
    return state?.uiState === 'available';
  }).length;

  const inactiveCount = accounts.filter((account) => {
    const state = accountStates[account.id];
    return state?.uiState === 'inactive';
  }).length;

  const errorCount = accounts.filter((account) => {
    const state = accountStates[account.id];
    return state?.uiState === 'error';
  }).length;

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
            <CalendarCheck size={20} /> {t('trae.checkin.modalTitle', '每日签到')} - {platformLabel}
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
              disabled={checkAllLoading || accounts.length === 0}
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
                const displayEmail = getTraeAccountDisplayLabel(account);
                const uiState = state.uiState;
                const showMeta = state.status != null && state.status.active !== false;

                return (
                  <div
                    key={account.id}
                    className={`checkin-account-row ${uiState === 'claimed' ? 'checked' : ''} ${
                      uiState === 'inactive' ? 'inactive' : ''
                    }`}
                  >
                    <div className="checkin-account-info">
                      <span className="checkin-account-name" title={displayEmail}>
                        {displayEmail}
                      </span>
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
                        <span className="checkin-status-unknown">
                          <Ban size={16} />
                          {t('workbuddy.checkin.inactive', '不可用')}
                        </span>
                      )}

                      {showMeta && (state.status?.total_credits ?? 0) > 0 && (
                        <span className="checkin-credit-badge" title={state.status?.message || ''}>
                          <Trophy size={12} />
                          {t('workbuddy.checkin.totalCredits', '{{credits}} 积分', {
                            credits: state.status?.total_credits ?? 0,
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
          <TraeAutoCheckinConfigModal
            config={autoCheckinConfig}
            onSave={async (newConfig) => {
              await saveTraeAutoCheckinConfigAsync(newConfig);
              setAutoCheckinConfig(newConfig);
            }}
            onClose={() => setShowConfigModal(false)}
          />
        )}
      </div>
    </div>
  );
}
