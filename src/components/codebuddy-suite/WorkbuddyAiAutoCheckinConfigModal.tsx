import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { LOCAL_HISTORY_TIMEOUT, useLocalHistory } from '../../hooks/useLocalHistory';
import {
  X,
  Clock,
  AlertCircle,
  Settings,
  History,
  Trash2,
  Play,
  CheckCircle,
  XCircle,
  Ban,
  Loader2,
  ChevronDown,
  ChevronUp,
} from 'lucide-react';
import { useEscCloseTopmost } from '../../hooks/useEscClose';
import {
  WorkbuddyAiAutoCheckinConfig,
  WorkbuddyAiAutoCheckinAccountDetail,
  WorkbuddyAiAutoCheckinLogRecord,
  hasCachedWorkbuddyAiAutoCheckinConfig,
  getWorkbuddyAiAutoCheckinConfig,
  getWorkbuddyAiAutoCheckinConfigAsync,
  isValidTimeString,
  parseTimeToMinutes,
  getWorkbuddyAiAutoCheckinLogsAsync,
  clearWorkbuddyAiAutoCheckinLogs,
  runWorkbuddyAiAutoCheckinCycleIfNeeded,
  WORKBUDDY_AI_AUTO_CHECKIN_LOGS_CHANGED_EVENT,
} from '../../services/workbuddyAiAutoCheckinService';

interface WorkbuddyAiAutoCheckinConfigModalProps {
  config: WorkbuddyAiAutoCheckinConfig;
  onSave: (newConfig: WorkbuddyAiAutoCheckinConfig) => Promise<void>;
  onClose: () => void;
}

export function WorkbuddyAiAutoCheckinConfigModal({
  config,
  onSave,
  onClose,
}: WorkbuddyAiAutoCheckinConfigModalProps) {
  const { t } = useTranslation();
  const dialogRef = useRef<HTMLDivElement | null>(null);
  useEscCloseTopmost(true, onClose, dialogRef);

  const initialConfig = hasCachedWorkbuddyAiAutoCheckinConfig()
    ? getWorkbuddyAiAutoCheckinConfig()
    : config;
  const dirtyRef = useRef(false);
  const prevConfigPropRef = useRef(config);
  const [activeTab, setActiveTab] = useState<'settings' | 'history'>('settings');
  const [hydrated, setHydrated] = useState(() => hasCachedWorkbuddyAiAutoCheckinConfig());
  const [baseConfig, setBaseConfig] = useState<WorkbuddyAiAutoCheckinConfig>(initialConfig);
  const [enabled, setEnabled] = useState(initialConfig.enabled);
  const [startTime, setStartTime] = useState(initialConfig.startTime || '06:00');
  const [endTime, setEndTime] = useState(initialConfig.endTime || '12:00');
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (hydrated) {
      return;
    }
    let disposed = false;
    void getWorkbuddyAiAutoCheckinConfigAsync()
      .then((loaded) => {
        if (disposed) {
          return;
        }
        setBaseConfig(loaded);
        if (!dirtyRef.current) {
          setEnabled(loaded.enabled);
          setStartTime(loaded.startTime || '06:00');
          setEndTime(loaded.endTime || '12:00');
        }
        setHydrated(true);
      })
      .catch(() => {
        if (!disposed) {
          setHydrated(true);
        }
      });
    return () => {
      disposed = true;
    };
  }, [hydrated]);

  useEffect(() => {
    if (prevConfigPropRef.current === config) {
      return;
    }
    prevConfigPropRef.current = config;
    setBaseConfig(config);
    if (!dirtyRef.current) {
      setEnabled(config.enabled);
      setStartTime(config.startTime || '06:00');
      setEndTime(config.endTime || '12:00');
    }
    if (hasCachedWorkbuddyAiAutoCheckinConfig()) {
      setHydrated(true);
    }
  }, [config]);

  const {
    records: logs,
    loading: logsLoading,
    error: logsError,
    migrationPending,
    refresh: refreshLogs,
    cleared: clearVisibleLogs,
  } = useLocalHistory(
    getWorkbuddyAiAutoCheckinLogsAsync,
    WORKBUDDY_AI_AUTO_CHECKIN_LOGS_CHANGED_EVENT,
  );
  const [saving, setSaving] = useState(false);
  const [manualTesting, setManualTesting] = useState(false);
  const [clearingLogs, setClearingLogs] = useState(false);
  const [expandedLogIds, setExpandedLogIds] = useState<Record<string, boolean>>({});

  const handleSave = async () => {
    if (!hydrated || saving) {
      return;
    }
    if (!isValidTimeString(startTime) || !isValidTimeString(endTime)) {
      setError(t('workbuddyAi.checkin.timeRangeError', '开始时间不能晚于结束时间'));
      return;
    }
    const startMin = parseTimeToMinutes(startTime);
    const endMin = parseTimeToMinutes(endTime);

    if (startMin > endMin) {
      setError(t('workbuddyAi.checkin.timeRangeError', '开始时间不能晚于结束时间'));
      return;
    }

    setSaving(true);
    setError(null);
    try {
      await onSave({
        ...baseConfig,
        enabled,
        startTime,
        endTime,
        accountSchedules: undefined,
      });
      onClose();
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setError(
        t('common.shared.checkin.saveFailed', '自动签到设置保存失败：{{message}}', { message }),
      );
    } finally {
      setSaving(false);
    }
  };

  const handleManualTest = async () => {
    setManualTesting(true);
    setError(null);
    try {
      const result = await runWorkbuddyAiAutoCheckinCycleIfNeeded(true);
      refreshLogs();
      if (result === 'retry_scheduled') {
        setError(
          t(
            'common.shared.checkin.manualTestPartialFailure',
            '部分账号签到失败，请查看记录；后台稍后会自动重试。',
          ),
        );
      } else if (result === 'partial_failed' || result === 'failed' || result === 'retry') {
        setError(
          t(
            'workbuddyAi.checkin.manualTestPartialFailureNoAuto',
            '部分账号打卡失败，请查看记录。',
          ),
        );
      } else if (result === 'already_running') {
        setError(
          t('common.shared.checkin.manualTestAlreadyRunning', '自动签到任务正在执行，请稍后查看记录。'),
        );
      } else if (result === 'no_accounts') {
        setError(t('workbuddyAi.checkin.noAccounts', '暂无账号'));
      }
    } catch (err) {
      console.warn('[WorkbuddyAiAutoCheckin] 手动测试自动活跃异常:', err);
      const message = err instanceof Error ? err.message : String(err);
      setError(
        t('common.shared.checkin.manualTestFailed', '测试执行失败：{{message}}', { message }),
      );
    } finally {
      setManualTesting(false);
    }
  };

  const handleClearLogs = async () => {
    if (clearingLogs) {
      return;
    }
    setClearingLogs(true);
    setError(null);
    try {
      await clearWorkbuddyAiAutoCheckinLogs();
      clearVisibleLogs();
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setError(
        t('common.shared.checkin.clearLogsFailed', '清空签到记录失败：{{message}}', { message }),
      );
    } finally {
      setClearingLogs(false);
    }
  };

  const toggleExpand = (logId: string) => {
    setExpandedLogIds((prev) => ({
      ...prev,
      [logId]: !prev[logId],
    }));
  };

  const formatDuration = (ms: number): string => {
    if (ms < 1000) {
      return `${ms} ms`;
    }
    return `${(ms / 1000).toFixed(2)} s`;
  };

  const resolveBatchBadge = (log: WorkbuddyAiAutoCheckinLogRecord) => {
    const completed = log.successCount + log.alreadyCheckedCount;
    switch (log.status) {
      case 'success':
        return {
          className: 'success',
          icon: <CheckCircle size={12} />,
          label: `${t('workbuddyAi.checkin.done', '已完成')} (${completed}/${log.totalAccounts})`,
        };
      case 'partial':
        return {
          className: 'partial',
          icon: <AlertCircle size={12} />,
          label: t('trae.checkin.logPartial', '部分成功 ({{done}}/{{total}})', {
            done: completed,
            total: log.totalAccounts,
          }),
        };
      case 'inactive':
        return {
          className: 'inactive',
          icon: <Ban size={12} />,
          label: `${t('workbuddyAi.checkin.inactive', '不可用')} (${log.inactiveCount ?? log.totalAccounts}/${log.totalAccounts})`,
        };
      case 'pending':
        return {
          className: 'pending',
          icon: <Clock size={12} />,
          label: `${t('workbuddyAi.checkin.pendingStatus', '待确认')} (${log.pendingCount ?? log.totalAccounts}/${log.totalAccounts})`,
        };
      case 'no_accounts':
        return {
          className: 'inactive',
          icon: <Ban size={12} />,
          label: t('workbuddyAi.checkin.noAccounts', '暂无账号'),
        };
      default:
        return {
          className: 'failed',
          icon: <XCircle size={12} />,
          label: `${t('workbuddyAi.checkin.failedStatus', '未完成')} (${completed}/${log.totalAccounts})`,
        };
    }
  };

  const resolveDetailBadgeClass = (status: WorkbuddyAiAutoCheckinAccountDetail['status']) => {
    if (status === 'success' || status === 'already_checked') {
      return 'success';
    }
    if (status === 'inactive') {
      return 'inactive';
    }
    if (status === 'pending') {
      return 'pending';
    }
    return 'failed';
  };

  const resolveDetailLabel = (item: WorkbuddyAiAutoCheckinAccountDetail): string => {
    if (item.status === 'success') {
      return item.modelId
        ? `${t('workbuddyAi.checkin.checkedIn', '今日已活跃')} (${item.modelId})`
        : t('workbuddyAi.checkin.checkedIn', '今日已活跃');
    }
    if (item.status === 'already_checked') {
      return t('workbuddyAi.checkin.claimed', '今日已活跃');
    }
    if (item.status === 'inactive') {
      return t('workbuddyAi.checkin.inactiveHint', '每日活跃活动未开启或不适用当前账号');
    }
    if (item.status === 'pending') {
      return t('workbuddyAi.checkin.pendingStatus', '待确认');
    }
    if (item.resultCode === 'token_expired') {
      return t('workbuddyAi.checkin.errorTokenExpired', '登录已过期，请重新登录');
    }
    if (item.resultCode === 'no_free_model') {
      return t('workbuddyAi.checkin.errorNoFreeModel', '当前无可用 x0.00 免费模型，已中止');
    }
    return t('workbuddyAi.checkin.errorGeneric', '执行失败，请稍后重试');
  };

  return (
    <div className="modal-overlay auto-checkin-config-overlay">
      <div
        ref={dialogRef}
        className="modal-content auto-checkin-config-modal workbuddy-ai-auto-checkin-modal"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="modal-header auto-checkin-modal-header">
          <div className="auto-checkin-tabs-nav">
            <button
              className={`auto-checkin-tab-item ${activeTab === 'settings' ? 'active' : ''}`}
              onClick={() => {
                setActiveTab('settings');
                setError(null);
              }}
            >
              <Settings size={14} />
              {t('common.shared.checkin.tabSettings', '自动签到设置')}
            </button>
            <button
              className={`auto-checkin-tab-item ${activeTab === 'history' ? 'active' : ''}`}
              onClick={() => {
                setActiveTab('history');
                setError(null);
              }}
            >
              <History size={14} />
              {t('common.shared.checkin.tabHistory', '自动签到记录')}
              {logs.length > 0 && <span className="tab-count-badge">{logs.length}</span>}
            </button>
          </div>

          <button className="modal-close" onClick={onClose}>
            <X size={18} />
          </button>
        </div>

        <div className="modal-body auto-checkin-config-body">
          {activeTab === 'settings' ? (
            <>
              <div className="config-form-item toggle-item">
                <div className="toggle-label">
                  <span className="label-title">
                    {t('workbuddyAi.checkin.enableAutoCheckin', '开启自动活跃打卡')}
                  </span>
                  <span className="label-desc">
                    {t(
                      'workbuddyAi.checkin.enableAutoCheckinDesc',
                      '每天在指定时间窗内实时校验 x0.00 免费模型，发送 1 轮极简对话与桌面事件（含 UID、派生设备/会话标识及本机 OS/CPU/内存/时区）点亮今日活跃；次日结算以符合官方活动资格与方案条款为前提。',
                    )}
                  </span>
                </div>
                <label className="toggle-switch">
                  <input
                    type="checkbox"
                    checked={enabled}
                    disabled={!hydrated}
                    onChange={(e) => {
                      dirtyRef.current = true;
                      setEnabled(e.target.checked);
                      setError(null);
                    }}
                  />
                  <span className="slider round"></span>
                </label>
              </div>

              <div className={`config-form-section ${!enabled || !hydrated ? 'disabled' : ''}`}>
                <h3 className="section-title">
                  {t('workbuddyAi.checkin.randomTimeRange', '随机打卡时间段')}
                </h3>

                <div className="time-range-inputs">
                  <div className="time-input-group">
                    <label>{t('workbuddyAi.checkin.startTime', '开始时间')}</label>
                    <input
                      type="time"
                      value={startTime}
                      onChange={(e) => {
                        dirtyRef.current = true;
                        setStartTime(e.target.value);
                        setError(null);
                      }}
                      disabled={!hydrated || !enabled}
                    />
                  </div>

                  <span className="time-separator">{t('common.to', '至')}</span>

                  <div className="time-input-group">
                    <label>{t('workbuddyAi.checkin.endTime', '结束时间')}</label>
                    <input
                      type="time"
                      value={endTime}
                      onChange={(e) => {
                        dirtyRef.current = true;
                        setEndTime(e.target.value);
                        setError(null);
                      }}
                      disabled={!hydrated || !enabled}
                    />
                  </div>
                </div>

                <p className="section-hint">
                  {t(
                    'workbuddyAi.checkin.timeRangeHint',
                    '应用保持运行时，仅在该时间窗内随机选取时间检查今日活跃状态；若尚未活跃则校验并调用 x0.00 免费模型打卡（无免费模型或超过结束时间则不触发）。',
                  )}
                </p>
              </div>
            </>
          ) : (
            <div className="auto-checkin-history-container">
              <div className="history-toolbar">
                <span className="history-hint">
                  {!logsLoading &&
                    !migrationPending &&
                    !logsError &&
                    t(
                      'common.shared.checkin.historyRangeHint',
                      '仅保留近 30 天的自动签到记录（共 {{count}} 条）',
                      { count: logs.length },
                    )}
                </span>
                <div className="history-toolbar-actions">
                  <button
                    className="btn btn-secondary btn-xs"
                    onClick={() => void handleManualTest()}
                    disabled={manualTesting}
                    title={t('common.shared.checkin.manualTest', '测试触发一次自动签到')}
                  >
                    {manualTesting ? (
                      <Loader2 size={12} className="animate-spin" />
                    ) : (
                      <Play size={12} />
                    )}
                    {t('common.shared.checkin.manualTest', '测试执行')}
                  </button>
                  {logs.length > 0 && (
                    <button
                      className="btn btn-secondary btn-xs"
                      onClick={() => void handleClearLogs()}
                      disabled={clearingLogs}
                      title={t('common.shared.checkin.clearLogs', '清空记录')}
                    >
                      {clearingLogs ? (
                        <Loader2 size={12} className="animate-spin" />
                      ) : (
                        <Trash2 size={12} />
                      )}
                      {t('common.shared.checkin.clearLogs', '清空')}
                    </button>
                  )}
                </div>
              </div>

              {migrationPending && (
                <p className="section-hint" role="status">
                  {t('common.historyRestoring')}
                </p>
              )}
              {logsError && (
                <div className="history-toolbar" role="alert">
                  <span>
                    {t('common.shared.checkin.historyLoadFailed', '读取自动签到记录失败：{{message}}', {
                      message:
                        logsError === LOCAL_HISTORY_TIMEOUT
                          ? t('common.dataQueryBusy')
                          : logsError,
                    })}
                  </span>
                  <button
                    type="button"
                    className="btn btn-secondary btn-xs"
                    onClick={() => refreshLogs(true)}
                  >
                    {t('common.retry')}
                  </button>
                </div>
              )}
              {logsLoading && logs.length === 0 && !migrationPending ? (
                <div className="auto-checkin-empty-history">
                  <Loader2 size={32} className="animate-spin" />
                  <span>{t('common.shared.checkin.historyLoading', '正在读取自动签到记录...')}</span>
                </div>
              ) : logs.length === 0 ? (
                !migrationPending &&
                !logsError && (
                  <div className="auto-checkin-empty-history">
                    <Clock size={32} />
                    <span>{t('common.shared.checkin.noHistory', '暂无近 30 天的自动签到记录')}</span>
                  </div>
                )
              ) : (
                <div className="auto-checkin-history-list">
                  {logs.map((log) => {
                    const isExpanded = !!expandedLogIds[log.id];
                    const badge = resolveBatchBadge(log);

                    return (
                      <div key={log.id} className="history-log-item">
                        <div className="history-log-header" onClick={() => toggleExpand(log.id)}>
                          <div className="history-log-main-info">
                            <span className="log-timestamp">{log.timestamp}</span>
                            <span className="log-duration-badge">
                              <Clock size={11} />
                              {formatDuration(log.durationMs)}
                            </span>
                          </div>

                          <div className="history-log-meta">
                            <span className={`log-status-badge ${badge.className}`}>
                              {badge.icon}
                              {badge.label}
                            </span>
                            {log.details && log.details.length > 0 && (
                              <button className="btn-icon-toggle">
                                {isExpanded ? <ChevronUp size={14} /> : <ChevronDown size={14} />}
                              </button>
                            )}
                          </div>
                        </div>

                        {isExpanded && log.details && log.details.length > 0 && (
                          <div className="history-log-details">
                            {log.details.map((item, idx) => (
                              <div key={idx} className="log-detail-row">
                                <div className="detail-account-info">
                                  <span className="detail-account" title={item.email}>
                                    {item.email}
                                  </span>
                                  {item.time && <span className="detail-time">{item.time}</span>}
                                </div>
                                <span
                                  className={`detail-status ${resolveDetailBadgeClass(item.status)}`}
                                  title={item.message || undefined}
                                >
                                  {resolveDetailLabel(item)}
                                </span>
                              </div>
                            ))}
                          </div>
                        )}
                      </div>
                    );
                  })}
                </div>
              )}
            </div>
          )}
          {error && (
            <div className="config-error-message">
              <AlertCircle size={14} /> {error}
            </div>
          )}
        </div>

        <div className="modal-footer">
          {activeTab === 'settings' ? (
            <>
              <button className="btn btn-secondary" onClick={onClose}>
                {t('common.cancel', '取消')}
              </button>
              <button
                className="btn btn-primary"
                onClick={() => void handleSave()}
                disabled={!hydrated || saving}
              >
                {saving && <Loader2 size={14} className="animate-spin" />}
                {t('common.save', '保存')}
              </button>
            </>
          ) : (
            <button className="btn btn-secondary" onClick={onClose}>
              {t('common.close', '关闭')}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
