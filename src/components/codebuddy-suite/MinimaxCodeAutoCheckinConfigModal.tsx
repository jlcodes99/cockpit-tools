import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { listen } from '@tauri-apps/api/event';
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
  Loader2,
  ChevronDown,
  ChevronUp,
} from 'lucide-react';
import { useEscClose } from '../../hooks/useEscClose';
import {
  MinimaxCodeAutoCheckinConfig,
  MinimaxCodeAutoCheckinLogRecord,
  parseTimeToMinutes,
  getMinimaxCodeAutoCheckinLogsAsync,
  clearMinimaxCodeAutoCheckinLogs,
  runMinimaxCodeAutoCheckinCycleIfNeeded,
  MINIMAX_CODE_AUTO_CHECKIN_LOGS_CHANGED_EVENT,
} from '../../services/minimaxCodeAutoCheckinService';

interface MinimaxCodeAutoCheckinConfigModalProps {
  config: MinimaxCodeAutoCheckinConfig;
  onSave: (newConfig: MinimaxCodeAutoCheckinConfig) => Promise<void>;
  onClose: () => void;
}

export function MinimaxCodeAutoCheckinConfigModal({
  config,
  onSave,
  onClose,
}: MinimaxCodeAutoCheckinConfigModalProps) {
  const { t } = useTranslation();
  useEscClose(true, onClose);

  const [activeTab, setActiveTab] = useState<'settings' | 'history'>('settings');
  const [enabled, setEnabled] = useState(config.enabled);
  const [startTime, setStartTime] = useState(config.startTime || '06:00');
  const [endTime, setEndTime] = useState(config.endTime || '12:00');
  const [error, setError] = useState<string | null>(null);

  const [logs, setLogs] = useState<MinimaxCodeAutoCheckinLogRecord[]>([]);
  const [logsLoading, setLogsLoading] = useState(true);
  const [logsError, setLogsError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [manualTesting, setManualTesting] = useState(false);
  const [expandedLogIds, setExpandedLogIds] = useState<Record<string, boolean>>({});

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    let requestId = 0;

    const loadLogs = async () => {
      const currentRequestId = ++requestId;
      setLogsLoading(true);
      setLogsError(null);

      try {
        const nextLogs = await getMinimaxCodeAutoCheckinLogsAsync();
        if (!disposed && currentRequestId === requestId) {
          setLogs(nextLogs);
        }
      } catch (err) {
        console.warn('[MinimaxCodeAutoCheckin] 读取后端签到日志失败:', err);
        if (!disposed && currentRequestId === requestId) {
          setLogsError(err instanceof Error ? err.message : String(err));
        }
      } finally {
        if (!disposed && currentRequestId === requestId) {
          setLogsLoading(false);
        }
      }
    };

    void loadLogs();
    const handleLogsChange = () => {
      void loadLogs();
    };
    void listen(MINIMAX_CODE_AUTO_CHECKIN_LOGS_CHANGED_EVENT, handleLogsChange)
      .then((stopListening) => {
        if (disposed) {
          stopListening();
        } else {
          unlisten = stopListening;
        }
      })
      .catch((err) => {
        console.warn('[MinimaxCodeAutoCheckin] 监听后端签到日志事件失败:', err);
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const handleSave = async () => {
    const startMin = parseTimeToMinutes(startTime);
    const endMin = parseTimeToMinutes(endTime);

    if (startMin > endMin) {
      setError(t('minimaxCode.checkin.timeRangeError', '开始时间不能晚于结束时间'));
      return;
    }

    setSaving(true);
    setError(null);
    try {
      await onSave({
        ...config,
        enabled,
        startTime,
        endTime,
        accountSchedules: undefined,
      });
      onClose();
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setError(
        t('minimaxCode.checkin.saveFailed', '自动签到设置保存失败：{{message}}', { message }),
      );
    } finally {
      setSaving(false);
    }
  };

  const handleManualTest = async () => {
    setManualTesting(true);
    setError(null);
    try {
      const result = await runMinimaxCodeAutoCheckinCycleIfNeeded(true);
      setLogs(await getMinimaxCodeAutoCheckinLogsAsync());
      setLogsError(null);
      if (result === 'retry') {
        setError(
          t(
            'minimaxCode.checkin.manualTestPartialFailure',
            '部分账号签到失败，请查看记录；后台稍后会自动重试。',
          ),
        );
      } else if (result === 'waiting') {
        setError(
          t('minimaxCode.checkin.manualTestAlreadyRunning', '自动签到任务正在执行，请稍后查看记录。'),
        );
      }
    } catch (err) {
      console.warn('[MinimaxCodeAutoCheckin] 手动测试自动签到异常:', err);
      const message = err instanceof Error ? err.message : String(err);
      setError(
        t('minimaxCode.checkin.manualTestFailed', '测试执行失败：{{message}}', { message }),
      );
    } finally {
      setManualTesting(false);
    }
  };

  const handleClearLogs = async () => {
    setError(null);
    try {
      await clearMinimaxCodeAutoCheckinLogs();
      setLogs([]);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setError(
        t('minimaxCode.checkin.clearLogsFailed', '清空签到记录失败：{{message}}', { message }),
      );
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
      return `${ms} 毫秒`;
    }
    return `${(ms / 1000).toFixed(2)} 秒`;
  };

  return (
    <div className="modal-overlay auto-checkin-config-overlay" onClick={onClose}>
      <div
        className="modal-content auto-checkin-config-modal"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="modal-header auto-checkin-modal-header">
          <div className="auto-checkin-tabs-nav">
            <button
              className={`auto-checkin-tab-item ${activeTab === 'settings' ? 'active' : ''}`}
              onClick={() => setActiveTab('settings')}
            >
              <Settings size={14} />
              {t('minimaxCode.checkin.tabSettings', '自动签到设置')}
            </button>
            <button
              className={`auto-checkin-tab-item ${activeTab === 'history' ? 'active' : ''}`}
              onClick={() => setActiveTab('history')}
            >
              <History size={14} />
              {t('minimaxCode.checkin.tabHistory', '自动签到记录')}
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
                    {t('minimaxCode.checkin.enableAutoCheckin', '开启自动签到')}
                  </span>
                  <span className="label-desc">
                    {t(
                      'minimaxCode.checkin.enableAutoCheckinDesc',
                      '每天在指定时间段内随机选取一个时间完成自动签到',
                    )}
                  </span>
                </div>
                <label className="toggle-switch">
                  <input
                    type="checkbox"
                    checked={enabled}
                    onChange={(e) => setEnabled(e.target.checked)}
                  />
                  <span className="slider round"></span>
                </label>
              </div>

              <div className={`config-form-section ${!enabled ? 'disabled' : ''}`}>
                <h3 className="section-title">
                  {t('minimaxCode.checkin.randomTimeRange', '随机签到时间段')}
                </h3>

                <div className="time-range-inputs">
                  <div className="time-input-group">
                    <label>{t('minimaxCode.checkin.startTime', '开始时间')}</label>
                    <input
                      type="time"
                      value={startTime}
                      onChange={(e) => {
                        setStartTime(e.target.value);
                        setError(null);
                      }}
                      disabled={!enabled}
                    />
                  </div>

                  <span className="time-separator">{t('common.to', '至')}</span>

                  <div className="time-input-group">
                    <label>{t('minimaxCode.checkin.endTime', '结束时间')}</label>
                    <input
                      type="time"
                      value={endTime}
                      onChange={(e) => {
                        setEndTime(e.target.value);
                        setError(null);
                      }}
                      disabled={!enabled}
                    />
                  </div>
                </div>

                <p className="section-hint">
                  {t(
                    'minimaxCode.checkin.timeRangeHint',
                    '在这个时间段内随机选取一个时间点完成后台自动签到。系统将自动保持运行并检测。',
                  )}
                </p>
              </div>
            </>
          ) : (
            <div className="auto-checkin-history-container">
              <div className="history-toolbar">
                <span className="history-hint">
                  {t(
                    'minimaxCode.checkin.historyRangeHint',
                    '仅保留近 30 天的自动签到记录（共 {{count}} 条）',
                    { count: logs.length },
                  )}
                </span>
                <div className="history-toolbar-actions">
                  <button
                    className="btn btn-secondary btn-xs"
                    onClick={() => void handleManualTest()}
                    disabled={manualTesting}
                    title={t('minimaxCode.checkin.manualTest', '测试触发一次自动签到')}
                  >
                    {manualTesting ? (
                      <Loader2 size={12} className="animate-spin" />
                    ) : (
                      <Play size={12} />
                    )}
                    {t('minimaxCode.checkin.manualTest', '测试执行')}
                  </button>
                  {logs.length > 0 && (
                    <button
                      className="btn btn-secondary btn-xs"
                      onClick={() => void handleClearLogs()}
                      title={t('minimaxCode.checkin.clearLogs', '清空记录')}
                    >
                      <Trash2 size={12} />
                      {t('minimaxCode.checkin.clearLogs', '清空')}
                    </button>
                  )}
                </div>
              </div>

              {logsLoading ? (
                <div className="auto-checkin-empty-history">
                  <Loader2 size={32} className="animate-spin" />
                  <span>{t('minimaxCode.checkin.historyLoading', '正在读取自动签到记录...')}</span>
                </div>
              ) : logsError ? (
                <div className="auto-checkin-empty-history">
                  <XCircle size={32} />
                  <span>
                    {t('minimaxCode.checkin.historyLoadFailed', '读取自动签到记录失败：{{message}}', {
                      message: logsError,
                    })}
                  </span>
                </div>
              ) : logs.length === 0 ? (
                <div className="auto-checkin-empty-history">
                  <Clock size={32} />
                  <span>{t('minimaxCode.checkin.noHistory', '暂无近 30 天的自动签到记录')}</span>
                </div>
              ) : (
                <div className="auto-checkin-history-list">
                  {logs.map((log) => {
                    const isExpanded = !!expandedLogIds[log.id];
                    const isSuccess = log.status === 'success';
                    const isPartial = log.status === 'partial';

                    return (
                      <div key={log.id} className="history-log-item">
                        <div className="history-log-header" onClick={() => toggleExpand(log.id)}>
                          <div className="history-log-main-info">
                            <span className="log-timestamp">{log.timestamp}</span>
                            <span className="log-duration-badge" title="累计签到耗时">
                              <Clock size={11} />
                              {formatDuration(log.durationMs)}
                            </span>
                          </div>

                          <div className="history-log-meta">
                            <span
                              className={`log-status-badge ${
                                isSuccess ? 'success' : isPartial ? 'partial' : 'failed'
                              }`}
                            >
                              {isSuccess ? (
                                <CheckCircle size={12} />
                              ) : (
                                <XCircle size={12} />
                              )}
                              {isSuccess
                                ? `成功 (${log.successCount + log.alreadyCheckedCount}/${log.totalAccounts})`
                                : isPartial
                                  ? `部分成功 (${log.successCount + log.alreadyCheckedCount}/${log.totalAccounts})`
                                  : `未完成`}
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
                                  className={`detail-status ${
                                    item.status === 'success' || item.status === 'already_checked'
                                      ? 'success'
                                      : 'failed'
                                  }`}
                                >
                                  {item.status === 'success'
                                    ? `签到成功 ${item.credit ? `(+${item.credit})` : ''}`
                                    : item.status === 'already_checked'
                                      ? '今日已领'
                                      : item.message || '签到异常'}
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
                disabled={saving}
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

