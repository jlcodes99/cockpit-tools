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

import { useState, useCallback, useEffect, useMemo, useRef, type ComponentType } from 'react';
import { useTranslation } from 'react-i18next';
import { listen } from '@tauri-apps/api/event';
import {
  X,
  AlertCircle,
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
import { useEscCloseTopmost } from '../../hooks/useEscClose';
import { WorkbuddyAutoCheckinConfigModal } from './WorkbuddyAutoCheckinConfigModal';
import {
  getWorkbuddyAutoCheckinConfig,
  getWorkbuddyAutoCheckinConfigAsync,
  saveWorkbuddyAutoCheckinConfigAsync,
  WORKBUDDY_AUTO_CHECKIN_CONFIG_CHANGED_EVENT,
  WorkbuddyAutoCheckinConfig,
} from '../../services/workbuddyAutoCheckinService';

/** 自动签到配置的最小结构（WorkBuddy / MiniMax Code 均满足） */
interface CheckinAutoCheckinConfig {
  enabled: boolean;
  startTime: string;
  endTime: string;
}

/**
 * 自动签到配置注入：默认沿用 WorkBuddy 的服务与配置弹窗，MiniMax Code 等平台
 * 传入自己的实现即可复用同一套签到弹窗 UI。
 */
export interface CheckinAutoCheckinBinding {
  getConfig: () => CheckinAutoCheckinConfig;
  getConfigAsync: () => Promise<CheckinAutoCheckinConfig>;
  saveConfigAsync: (config: CheckinAutoCheckinConfig) => Promise<void>;
  changedEvent: string;
  ConfigModal: ComponentType<any>;
}

interface CodebuddySuiteCheckinModalProps<TAccount extends CodebuddySuiteAccountBase> {
  accounts: TAccount[];
  getCheckinStatus: (accountId: string) => Promise<CheckinStatusResponse>;
  performCheckin: (accountId: string) => Promise<CheckinResponse>;
  /** 账号名解析（旧入口，保留给 WorkBuddy）；优先用 `displayEmailResolver` */
  getDisplayEmail?: (account: TAccount) => string;
  onClose: () => void;
  onCheckinComplete?: () => void;
  /** 弹窗标题中的平台名，默认 WorkBuddy（保持既有行为） */
  platformLabel?: string;
  /** i18n 命名空间前缀，默认 workbuddy（保持既有行为） */
  i18nPrefix?: string;
  /** 自动签到配置绑定，默认 WorkBuddy（保持既有行为） */
  autoCheckin?: CheckinAutoCheckinBinding;
  /**
   * 账号名解析，默认按邮箱优先（WorkBuddy 行为）。MiniMax Code 的
   * `/v1/api/user/info` 常常不返回邮箱，此时会显示 numeric realUserID，
   * 因此它注入按用户名优先的解析器。
   */
  displayEmailResolver?: (account: TAccount) => string;
  /** 可选顶部说明提示（如 WorkBuddy AI 国际版说明 0 消耗活跃打卡与次日结算规则） */
  bannerNote?: string;
}

/** 与官方 WorkBuddy uiState 对齐（新增 pending 表示已提交对话待热力图同步确认） */
type CheckinUiState = 'loading' | 'available' | 'claimed' | 'pending' | 'inactive' | 'error';

/** 默认自动签到绑定：WorkBuddy（未注入 autoCheckin 时保持既有行为） */
const WORKBUDDY_AUTO_CHECKIN_BINDING: CheckinAutoCheckinBinding = {
  getConfig: getWorkbuddyAutoCheckinConfig,
  getConfigAsync: getWorkbuddyAutoCheckinConfigAsync,
  saveConfigAsync: (config) =>
    saveWorkbuddyAutoCheckinConfigAsync(config as WorkbuddyAutoCheckinConfig),
  changedEvent: WORKBUDDY_AUTO_CHECKIN_CONFIG_CHANGED_EVENT,
  ConfigModal: WorkbuddyAutoCheckinConfigModal,
};

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
  // 已提交活跃对话待同步确认时锁定为只读待确认状态，禁止再次触发对话
  if (status.pending_confirmation) {
    return 'pending';
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
  platformLabel = 'WorkBuddy',
  i18nPrefix = 'workbuddy',
  autoCheckin,
  displayEmailResolver,
  bannerNote,
}: CodebuddySuiteCheckinModalProps<TAccount>) {
  const resolveDisplayEmail = displayEmailResolver ?? getDisplayEmail ?? getAccountDisplayEmail;
  const { t } = useTranslation();
  const dialogRef = useRef<HTMLDivElement | null>(null);
  useEscCloseTopmost(true, onClose, dialogRef);
  const fetchGenerationRef = useRef(0);
  // 全局单调递增请求令牌，移除再重新添加同账号 ID 时绝不复用旧代次
  const nextRequestTokenRef = useRef(0);
  const latestAccountTokenRef = useRef<Record<string, number>>({});
  const activeAccountIdsRef = useRef<Set<string>>(new Set(accounts.map((a) => a.id)));
  activeAccountIdsRef.current = new Set(accounts.map((a) => a.id));
  const checkingInTokenRef = useRef<Map<string, number>>(new Map());
  const refreshingTokenRef = useRef<Map<string, number>>(new Map());
  const queriedIdsRef = useRef<Set<string>>(new Set());
  const [accountStates, setAccountStates] = useState<Record<string, AccountCheckinState>>({});
  const [refreshingAccountIds, setRefreshingAccountIds] = useState<Record<string, boolean>>({});
  const [checkAllLoading, setCheckAllLoading] = useState(false);
  const [refreshLoading, setRefreshLoading] = useState(false);
  const [showConfigModal, setShowConfigModal] = useState(false);
  const autoCheckinBinding: CheckinAutoCheckinBinding =
    autoCheckin ?? WORKBUDDY_AUTO_CHECKIN_BINDING;
  const [autoCheckinConfig, setAutoCheckinConfig] = useState<CheckinAutoCheckinConfig>(() =>
    autoCheckinBinding.getConfig(),
  );

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    const handleConfigChange = () => {
      void autoCheckinBinding.getConfigAsync().then((nextConfig) => {
        if (!disposed) {
          setAutoCheckinConfig(nextConfig);
        }
      });
    };
    handleConfigChange();
    window.addEventListener(autoCheckinBinding.changedEvent, handleConfigChange);
    void listen(autoCheckinBinding.changedEvent, handleConfigChange)
      .then((stopListening) => {
        if (disposed) {
          stopListening();
        } else {
          unlisten = stopListening;
        }
      })
      .catch((err) => {
        console.warn('[CheckinAutoCheckin] 监听后端签到配置事件失败:', err);
      });
    return () => {
      disposed = true;
      unlisten?.();
      window.removeEventListener(autoCheckinBinding.changedEvent, handleConfigChange);
    };
  }, [autoCheckinBinding]);

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

  const nextAccountToken = useCallback((accountId: string) => {
    const next = ++nextRequestTokenRef.current;
    latestAccountTokenRef.current[accountId] = next;
    return next;
  }, []);

  const isCurrentToken = useCallback(
    (accountId: string, token: number) =>
      activeAccountIdsRef.current.has(accountId) &&
      latestAccountTokenRef.current[accountId] === token,
    [],
  );

  const fetchAccountStatusList = useCallback(
    async (targetAccounts: TAccount[], isFullRefresh: boolean) => {
      if (targetAccounts.length === 0) {
        return;
      }
      const generation = isFullRefresh
        ? ++fetchGenerationRef.current
        : fetchGenerationRef.current;
      if (isFullRefresh) {
        setRefreshLoading(true);
      }

      // 跳过正在执行打卡/兑换的账号，不发多余查询也不重置行状态
      const eligible = targetAccounts.filter(
        (account) => !checkingInTokenRef.current.has(account.id),
      );
      const tokenByAccount = new Map<string, number>();
      for (const account of eligible) {
        tokenByAccount.set(account.id, nextAccountToken(account.id));
        queriedIdsRef.current.add(account.id);
      }

      setAccountStates((prev) => {
        const next = { ...prev };
        for (const account of eligible) {
          if (next[account.id]?.checkingIn) {
            continue;
          }
          next[account.id] = emptyAccountState('loading');
        }
        return next;
      });

      await Promise.allSettled(
        eligible.map(async (account) => {
          const token = tokenByAccount.get(account.id)!;
          try {
            const status = await getCheckinStatus(account.id);
            if (!isCurrentToken(account.id, token) || checkingInTokenRef.current.has(account.id)) {
              return;
            }
            setAccountStates((prev) => ({
              ...prev,
              [account.id]: {
                status,
                uiState: resolveUiState(status),
                checkingIn: false,
                error: null,
                checkinResult: null,
              },
            }));
          } catch (err: unknown) {
            if (!isCurrentToken(account.id, token) || checkingInTokenRef.current.has(account.id)) {
              return;
            }
            const message = err instanceof Error ? err.message : String(err);
            setAccountStates((prev) => ({
              ...prev,
              [account.id]: {
                status: null,
                uiState: 'error',
                checkingIn: false,
                error: message,
                checkinResult: null,
              },
            }));
          }
        }),
      );

      if (isFullRefresh && fetchGenerationRef.current === generation) {
        setRefreshLoading(false);
      }
    },
    [getCheckinStatus, isCurrentToken, nextAccountToken],
  );

  const fetchAllStatus = useCallback(async () => {
    await fetchAccountStatusList(accounts, true);
  }, [accounts, fetchAccountStatusList]);

  useEffect(() => {
    const activeIds = new Set(accounts.map((a) => a.id));
    activeAccountIdsRef.current = activeIds;
    for (const id of Array.from(queriedIdsRef.current)) {
      if (!activeIds.has(id)) {
        queriedIdsRef.current.delete(id);
        delete latestAccountTokenRef.current[id];
        checkingInTokenRef.current.delete(id);
        refreshingTokenRef.current.delete(id);
      }
    }
    setRefreshingAccountIds((prev) => {
      let changed = false;
      const next: Record<string, boolean> = {};
      for (const [id, active] of Object.entries(prev)) {
        if (activeIds.has(id) && active) {
          next[id] = active;
        } else {
          changed = true;
        }
      }
      return changed ? next : prev;
    });
    setAccountStates((prev) => {
      let changed = false;
      const next: Record<string, AccountCheckinState> = {};
      for (const [id, st] of Object.entries(prev)) {
        if (activeIds.has(id)) {
          next[id] = st;
        } else {
          changed = true;
        }
      }
      return changed ? next : prev;
    });

    const newAccounts = accounts.filter((account) => !queriedIdsRef.current.has(account.id));
    if (newAccounts.length > 0) {
      const isInitial = queriedIdsRef.current.size === 0;
      void fetchAccountStatusList(newAccounts, isInitial);
    }
  }, [accounts, fetchAccountStatusList]);

  const refreshOneStatus = useCallback(
    async (accountId: string, keepResult?: CheckinResponse | null) => {
      if (refreshingTokenRef.current.has(accountId)) {
        return null;
      }
      const token = nextAccountToken(accountId);
      refreshingTokenRef.current.set(accountId, token);
      setRefreshingAccountIds((prev) => ({ ...prev, [accountId]: true }));
      try {
        const status = await getCheckinStatus(accountId);
        if (!isCurrentToken(accountId, token)) {
          return null;
        }
        const nextUiState = resolveUiState(status);
        const nextCheckinResult =
          keepResult?.pending_confirmation && nextUiState !== 'pending'
            ? null
            : (keepResult ?? null);
        updateAccountState(accountId, {
          status,
          uiState: nextUiState,
          error: null,
          checkinResult: nextCheckinResult,
          checkingIn: false,
        });
        return status;
      } catch (err: unknown) {
        if (!isCurrentToken(accountId, token)) {
          return null;
        }
        const message = err instanceof Error ? err.message : String(err);
        updateAccountState(accountId, {
          checkingIn: false,
          error: message,
          // 查询失败：不假装未签到；保留上一状态，uiState=error
          uiState: 'error',
          checkinResult: keepResult ?? null,
        });
        return null;
      } finally {
        if (refreshingTokenRef.current.get(accountId) === token) {
          refreshingTokenRef.current.delete(accountId);
          setRefreshingAccountIds((prev) => {
            if (!prev[accountId]) {
              return prev;
            }
            const next = { ...prev };
            delete next[accountId];
            return next;
          });
        }
      }
    },
    [getCheckinStatus, isCurrentToken, nextAccountToken, updateAccountState],
  );

  const handleSingleCheckin = useCallback(
    async (accountId: string) => {
      if (checkingInTokenRef.current.has(accountId)) {
        return;
      }
      const token = nextAccountToken(accountId);
      checkingInTokenRef.current.set(accountId, token);
      updateAccountState(accountId, {
        checkingIn: true,
        error: null,
        checkinResult: null,
      });
      try {
        const result = await performCheckin(accountId);
        if (!isCurrentToken(accountId, token)) {
          return;
        }

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
            if (isCurrentToken(accountId, token)) {
              const merged: CheckinStatusResponse = {
                ...remote,
                today_checked_in: remote.today_checked_in || true,
                active: remote.active !== false,
                pending_confirmation: false,
              };
              updateAccountState(accountId, {
                status: merged,
                uiState: resolveUiState(merged),
                checkinResult: result,
                error: null,
                checkingIn: false,
              });
            }
          } catch {
            if (isCurrentToken(accountId, token)) {
              updateAccountState(accountId, { checkingIn: false, uiState: 'claimed' });
            }
          }
          onCheckinComplete?.();
          return;
        }

        if (result.pending_confirmation) {
          // 对话已提交但热力墙尚未同步：锁定为 pending 只读刷新态，禁止重复触发对话，同时保留既有连续天数与历史日期
          setAccountStates((prev) => {
            const previous = prev[accountId] ?? emptyAccountState();
            const prevStatus = previous.status;
            return {
              ...prev,
              [accountId]: {
                ...previous,
                checkingIn: false,
                checkinResult: result,
                error: null,
                uiState: 'pending',
                status: {
                  today_checked_in: false,
                  active: true,
                  streak_days: result.streak_days ?? prevStatus?.streak_days ?? 0,
                  daily_credit: prevStatus?.daily_credit ?? 0,
                  today_credit: prevStatus?.today_credit ?? null,
                  next_streak_day: prevStatus?.next_streak_day ?? null,
                  is_streak_day: prevStatus?.is_streak_day ?? null,
                  checkin_dates: prevStatus?.checkin_dates ?? null,
                  streak_bonus_days: prevStatus?.streak_bonus_days ?? null,
                  streak_bonus_credit: prevStatus?.streak_bonus_credit ?? null,
                  pending_confirmation: true,
                },
              },
            };
          });
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
        if (!isCurrentToken(accountId, token)) {
          return;
        }
        const message = err instanceof Error ? err.message : String(err);
        updateAccountState(accountId, {
          checkingIn: false,
          error: message,
        });
      } finally {
        if (checkingInTokenRef.current.get(accountId) === token) {
          checkingInTokenRef.current.delete(accountId);
        }
      }
    },
    [
      getCheckinStatus,
      isCurrentToken,
      nextAccountToken,
      onCheckinComplete,
      performCheckin,
      refreshOneStatus,
      updateAccountState,
    ],
  );

  const handleCheckAll = useCallback(async () => {
    setCheckAllLoading(true);
    // 官方：仅 available 且未在执行中的账号可触发
    const available = accounts.filter((a) => {
      const state = accountStates[a.id];
      return state?.uiState === 'available' && !state.checkingIn;
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
  const pendingCount = Object.values(accountStates).filter(
    (s) => s.uiState === 'pending',
  ).length;
  const inactiveCount = Object.values(accountStates).filter(
    (s) => s.uiState === 'inactive',
  ).length;

  return (
    <div className="modal-overlay">
      <div
        ref={dialogRef}
        className="modal-content checkin-modal"
        onClick={(e) => e.stopPropagation()}
      >
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
            <CalendarCheck size={20} /> {t(`${i18nPrefix}.checkin.modalTitle`, '每日签到')} -{' '}
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
              {t(`${i18nPrefix}.checkin.checkedIn`, '已签到')}
            </span>
            <span className="checkin-stat unchecked">
              <XCircle size={14} /> {availableCount}{' '}
              {t(`${i18nPrefix}.checkin.notCheckedIn`, '未签到')}
            </span>
            {pendingCount > 0 && (
              <span className="checkin-stat pending">
                <Clock size={14} /> {pendingCount}{' '}
                {t(`${i18nPrefix}.checkin.pendingStatus`, '待确认')}
              </span>
            )}
            {inactiveCount > 0 && (
              <span className="checkin-stat inactive">
                <Ban size={14} /> {inactiveCount}{' '}
                {t(`${i18nPrefix}.checkin.inactive`, '不可用')}
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
                      `${i18nPrefix}.checkin.autoCheckinEnabledHint`,
                      '自动签到已开启：将在 {{start}} 至 {{end}} 随机签到（点击设置）',
                      {
                        start: autoCheckinConfig.startTime,
                        end: autoCheckinConfig.endTime,
                      },
                    )
                  : t(`${i18nPrefix}.checkin.autoCheckinDisabledHint`, '自动签到未开启（点击设置）')
              }
            >
              <Clock size={14} />
              {autoCheckinConfig.enabled
                ? `${t(`${i18nPrefix}.checkin.autoCheckinLabel`, '自动签到')} (${
                    autoCheckinConfig.startTime
                  }-${autoCheckinConfig.endTime})`
                : t(`${i18nPrefix}.checkin.autoCheckinOff`, '自动签到未开启')}
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
              {t(`${i18nPrefix}.checkin.refreshStatus`, '刷新状态')}
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
              {t(`${i18nPrefix}.checkin.checkAll`, '一键签到')}
            </button>
          </div>
        </div>

        <div className="modal-body checkin-modal-body">
          {bannerNote && (
            <div className="checkin-modal-banner-note" role="note">
              <AlertCircle size={14} /> <span>{bannerNote}</span>
            </div>
          )}
          {accounts.length === 0 ? (
            <div className="checkin-empty">
              {t(`${i18nPrefix}.checkin.noAccounts`, '暂无账号')}
            </div>
          ) : (
            <div className="checkin-account-list">
              {accounts.map((account) => {
                const state = accountStates[account.id];
                const displayEmail = resolveDisplayEmail(account);
                const uiState: CheckinUiState = state?.uiState ?? 'loading';
                const isCheckingIn = state?.checkingIn ?? false;
                const isRefreshingOne = !!refreshingAccountIds[account.id];
                const isClaimed = uiState === 'claimed';
                const isPending = uiState === 'pending';
                const isAvailable = uiState === 'available';
                const isInactive = uiState === 'inactive';
                const isError = uiState === 'error';
                const isLoading = uiState === 'loading' || state === undefined;
                const warningMessage =
                  state?.checkinResult?.warning ||
                  (isPending
                    ? t(
                        `${i18nPrefix}.checkin.pendingConfirmation`,
                        '已提交活跃对话与事件，热力墙尚未同步，请稍后刷新状态确认（不会重复发送对话）。',
                      )
                    : null);

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
                          {t(`${i18nPrefix}.checkin.querying`, '查询中...')}
                        </span>
                      ) : isError ? (
                        <span className="checkin-status-no">
                          <XCircle size={16} />
                          {t(`${i18nPrefix}.checkin.queryFailed`, '状态查询失败')}
                        </span>
                      ) : isClaimed ? (
                        <span className="checkin-status-yes">
                          <CheckCircle size={16} />
                          {t(`${i18nPrefix}.checkin.checkedIn`, '已签到')}
                        </span>
                      ) : isPending ? (
                        <span className="checkin-status-unknown">
                          <Clock size={16} />
                          {t(`${i18nPrefix}.checkin.pendingStatus`, '待确认')}
                        </span>
                      ) : isAvailable ? (
                        <span className="checkin-status-no">
                          <XCircle size={16} />
                          {t(`${i18nPrefix}.checkin.notCheckedIn`, '未签到')}
                        </span>
                      ) : (
                        <span className="checkin-status-unknown">
                          <Ban size={16} />
                          {t(`${i18nPrefix}.checkin.inactive`, '不可用')}
                        </span>
                      )}

                      {showMeta && streakDays > 0 && (
                        <span className="checkin-streak-badge">
                          <Flame size={12} />
                          {t(`${i18nPrefix}.checkin.streakDays`, '{{days}} 天', {
                            days: streakDays,
                          })}
                        </span>
                      )}

                      {showMeta && displayCredit > 0 && (
                        <span className="checkin-credit-badge">
                          <Gift size={12} />+{displayCredit}
                        </span>
                      )}

                      {showMeta && (isStreakDay || (nextStreakDay != null && nextStreakDay > 0)) && (
                        <span
                          className={`checkin-streak-reward ${
                            isStreakDay ? 'streak-today' : ''
                          }`}
                        >
                          <Trophy size={12} />
                          {isStreakDay
                            ? t(
                                `${i18nPrefix}.checkin.streakRewardToday`,
                                '今日可获得大礼包!',
                              )
                            : t(
                                `${i18nPrefix}.checkin.streakRewardCountdown`,
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
                          {t(`${i18nPrefix}.checkin.button.loading`, '签到中...')}
                        </button>
                      ) : isClaimed ? (
                        <button className="btn btn-ghost btn-sm" disabled>
                          <CheckCircle size={14} />
                          {t(`${i18nPrefix}.checkin.claimed`, '已领取')}
                        </button>
                      ) : isPending ? (
                        <button
                          className="btn btn-secondary btn-sm"
                          onClick={() => void refreshOneStatus(account.id, state?.checkinResult)}
                          disabled={isRefreshingOne}
                        >
                          {isRefreshingOne ? (
                            <Loader2 size={14} className="animate-spin" />
                          ) : (
                            <RefreshCw size={14} />
                          )}
                          {t(`${i18nPrefix}.checkin.refreshStatus`, '刷新状态')}
                        </button>
                      ) : isAvailable ? (
                        <button
                          className="btn btn-primary btn-sm"
                          onClick={() => void handleSingleCheckin(account.id)}
                        >
                          <Gift size={14} />
                          {t(`${i18nPrefix}.checkin.button`, '签到')}
                        </button>
                      ) : isError ? (
                        <button
                          className="btn btn-secondary btn-sm"
                          onClick={() => void refreshOneStatus(account.id)}
                          disabled={isRefreshingOne}
                        >
                          {isRefreshingOne ? (
                            <Loader2 size={14} className="animate-spin" />
                          ) : (
                            <RefreshCw size={14} />
                          )}
                          {t(`${i18nPrefix}.checkin.retry`, '重试')}
                        </button>
                      ) : (
                        <button className="btn btn-ghost btn-sm" disabled>
                          <Ban size={14} />
                          {t(`${i18nPrefix}.checkin.inactive`, '不可用')}
                        </button>
                      )}
                    </div>

                    {isInactive && !isError && (
                      <div className="checkin-account-info-msg">
                        {t(
                          `${i18nPrefix}.checkin.inactiveHint`,
                          '签到活动未开启或不适用当前账号',
                        )}
                      </div>
                    )}

                    {state?.checkinResult &&
                      state.checkinResult.success &&
                      (state.checkinResult.already_claimed || state.status?.today_checked_in) && (
                        <div className="checkin-account-success">
                          <CheckCircle size={14} />
                          {state.checkinResult.already_claimed
                            ? t(`${i18nPrefix}.checkin.claimed`, '已领取')
                            : t(`${i18nPrefix}.checkin.success`, '签到成功！连续签到 {{days}} 天', {
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

                    {warningMessage && (
                      <div className="checkin-account-info-msg" role="status">
                        <AlertCircle size={12} /> {warningMessage}
                      </div>
                    )}
                    {!state?.checkinResult?.already_claimed && (state?.checkinResult?.credit != null ||
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
                        {t(`${i18nPrefix}.checkin.recentDates`, '近期签到：')}
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
            title={t(`${i18nPrefix}.checkin.autoCheckinSettings`, '自动签到设置')}
            aria-label={t(`${i18nPrefix}.checkin.autoCheckinSettings`, '自动签到设置')}
          >
            <Settings size={14} />
          </button>
          <button className="btn btn-secondary" onClick={onClose}>
            {t('common.close', '关闭')}
          </button>
        </div>

        {showConfigModal && (
          <autoCheckinBinding.ConfigModal
            config={autoCheckinConfig}
            onSave={async (newConfig: CheckinAutoCheckinConfig) => {
              await autoCheckinBinding.saveConfigAsync(newConfig);
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
import * as workbuddyAiService from '../../services/workbuddyAiService';
import * as workbuddyAiAutoCheckinService from '../../services/workbuddyAiAutoCheckinService';
import * as minimaxCodeService from '../../services/minimaxCodeService';
import * as minimaxCodeAutoCheckinService from '../../services/minimaxCodeAutoCheckinService';
import type { MinimaxCodeAccount } from '../../types/minimaxCode';
import { MinimaxCodeAutoCheckinConfigModal } from './MinimaxCodeAutoCheckinConfigModal';
import { WorkbuddyAiAutoCheckinConfigModal } from './WorkbuddyAiAutoCheckinConfigModal';
import { getAccountDisplayEmail } from '../../utils/codebuddy-suite';
import { getMinimaxCodeAccountDisplayEmail } from '../../types/minimaxCode';
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
      onClose={onClose}
      onCheckinComplete={onCheckinComplete}
    />
  );
}

// 便捷导出：MiniMax Code 签到弹窗
export function MinimaxCodeCheckinModal({
  accounts,
  onClose,
  onCheckinComplete,
}: {
  accounts: MinimaxCodeAccount[];
  onClose: () => void;
  onCheckinComplete?: () => void;
}) {
  // 绑定对象需保持引用稳定，否则共享弹窗的配置监听 effect 会每帧重跑。
  const autoCheckin = useMemo<CheckinAutoCheckinBinding>(
    () => ({
      getConfig: minimaxCodeAutoCheckinService.getMinimaxCodeAutoCheckinConfig,
      getConfigAsync: minimaxCodeAutoCheckinService.getMinimaxCodeAutoCheckinConfigAsync,
      saveConfigAsync: minimaxCodeAutoCheckinService.saveMinimaxCodeAutoCheckinConfigAsync,
      changedEvent:
        minimaxCodeAutoCheckinService.MINIMAX_CODE_AUTO_CHECKIN_CONFIG_CHANGED_EVENT,
      ConfigModal: MinimaxCodeAutoCheckinConfigModal,
    }),
    [],
  );

  return (
    <CodebuddySuiteCheckinModal
      accounts={accounts}
      displayEmailResolver={getMinimaxCodeAccountDisplayEmail}
      getCheckinStatus={async (accountId) => {
        const status = await minimaxCodeService.getCheckinStatusMinimaxCode(accountId);
        return {
          today_checked_in: status.todayCheckedIn,
          active: status.active,
          streak_days: status.streakDays,
          daily_credit: status.points,
          today_credit: null,
          next_streak_day: null,
          is_streak_day: null,
          checkin_dates: null,
          streak_bonus_days: null,
          streak_bonus_credit: null,
        };
      }}
      performCheckin={async (accountId) => {
        const result = await minimaxCodeService.checkinMinimaxCode(accountId);
        return {
          ...result,
          already_claimed: result.alreadyClaimed ?? false,
          streak_days: result.streakDays ?? null,
        } as unknown as CheckinResponse;
      }}
      getDisplayEmail={getAccountDisplayEmail}
      onClose={onClose}
      onCheckinComplete={onCheckinComplete}
      platformLabel="MiniMax Code"
      i18nPrefix="minimaxCode"
      autoCheckin={autoCheckin}
    />
  );
}

// 便捷导出：WorkBuddy AI（国际版）每日活跃打卡弹窗
export function WorkbuddyAiCheckinModal({
  accounts,
  onClose,
  onCheckinComplete,
}: {
  accounts: WorkbuddyAccount[];
  onClose: () => void;
  onCheckinComplete?: () => void;
}) {
  const { t } = useTranslation();
  const autoCheckin = useMemo<CheckinAutoCheckinBinding>(
    () => ({
      getConfig: workbuddyAiAutoCheckinService.getWorkbuddyAiAutoCheckinConfig,
      getConfigAsync: workbuddyAiAutoCheckinService.getWorkbuddyAiAutoCheckinConfigAsync,
      saveConfigAsync: (config) =>
        workbuddyAiAutoCheckinService.saveWorkbuddyAiAutoCheckinConfigAsync(
          config as workbuddyAiAutoCheckinService.WorkbuddyAiAutoCheckinConfig,
        ),
      changedEvent:
        workbuddyAiAutoCheckinService.WORKBUDDY_AI_AUTO_CHECKIN_CONFIG_CHANGED_EVENT,
      ConfigModal: WorkbuddyAiAutoCheckinConfigModal,
    }),
    [],
  );

  const localizeError = useCallback(
    (raw: unknown) =>
      workbuddyAiService.localizeWorkbuddyAiCheckinError(raw, (key, defaultValue) =>
        t(key, defaultValue),
      ),
    [t],
  );

  const getCheckinStatus = useCallback(
    async (accountId: string): Promise<CheckinStatusResponse> => {
      try {
        const status = await workbuddyAiService.getCheckinStatusWorkbuddyAi(accountId);
        return {
          today_checked_in: status.todayCheckedIn,
          active: status.active,
          streak_days: status.streakDays,
          daily_credit: status.dailyCredit,
          today_credit: status.todayCredit ?? null,
          next_streak_day: null,
          is_streak_day: null,
          checkin_dates: status.checkinDates ?? null,
          streak_bonus_days: null,
          streak_bonus_credit: null,
          pending_confirmation: status.pendingConfirmation ?? false,
        };
      } catch (err: unknown) {
        throw new Error(localizeError(err));
      }
    },
    [localizeError],
  );

  const performCheckin = useCallback(
    async (accountId: string): Promise<CheckinResponse> => {
      try {
        const result = await workbuddyAiService.checkinWorkbuddyAi(accountId);
        return {
          success: result.success,
          already_claimed: result.alreadyClaimed,
          inactive: result.inactive ?? false,
          pending_confirmation: result.pendingConfirmation ?? false,
          warning: result.pendingConfirmation
            ? t(
                'workbuddyAi.checkin.pendingConfirmation',
                '已提交活跃对话与事件，热力墙尚未同步，请稍后刷新状态确认（不会重复发送对话）。',
              )
            : result.warning
              ? localizeError(result.warning)
              : null,
          message:
            result.inactive && result.message === 'WORKBUDDY_AI_ENTERPRISE_INACTIVE'
              ? null
              : result.message
                ? localizeError(result.message)
                : null,
          credit: result.credit ?? null,
          streak_days: result.streakDays ?? null,
        };
      } catch (err: unknown) {
        throw new Error(localizeError(err));
      }
    },
    [localizeError, t],
  );

  return (
    <CodebuddySuiteCheckinModal
      accounts={accounts}
      getCheckinStatus={getCheckinStatus}
      performCheckin={performCheckin}
      onClose={onClose}
      onCheckinComplete={onCheckinComplete}
      platformLabel="WorkBuddy AI"
      i18nPrefix="workbuddyAi"
      autoCheckin={autoCheckin}
      bannerNote={t(
        'workbuddyAi.checkin.bannerNote',
        '实验性功能：触发前实时校验 /v3/config 中费率为 x0.00 的免费模型（无合法模型则中止），发送 1 轮极简对话与桌面事件（包含账号 UID、派生设备/会话标识及本机 OS/CPU/内存/时区）以点亮今日成长热力墙；次日积分结算以符合官方活动资格与方案条款为前提。',
      )}
    />
  );
}
