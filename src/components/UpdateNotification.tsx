import { useState, useMemo, useCallback } from 'react';
import { X, Download, Sparkles, RefreshCw, Check, XCircle } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { openUrl } from '@tauri-apps/plugin-opener';
import { useEscClose } from '../hooks/useEscClose';
import './UpdateNotification.css';

export interface UpdateInfo {
  latest_version: string;
  current_version: string;
  download_url: string;
  release_notes: string;
  release_notes_zh: string;
}

type UpdateCheckSource = 'auto' | 'manual';
type UpdateCheckStatus = 'has_update' | 'up_to_date' | 'failed';
type UpdateActionState = 'hidden' | 'available' | 'downloading' | 'installing' | 'ready';

export interface UpdateCheckResult {
  source: UpdateCheckSource;
  status: UpdateCheckStatus;
  currentVersion?: string;
  latestVersion?: string;
  error?: string;
}

interface UpdateNotificationProps {
  updateInfo?: UpdateInfo | null;
  checking?: boolean;
  onClose: () => void;
  onRestartUpdate?: () => Promise<void>;
  actionState?: UpdateActionState;
  actionVersion?: string | null;
  actionProgress?: number;
  actionRetryStatus?: string;
  actionError?: string;
  actionErrorDetails?: string;
  skipError?: string;
  onPrimaryAction?: () => Promise<void> | void;
  onCancelUpdate?: () => Promise<void> | void;
  onSkipUpdate?: () => Promise<void> | void;
}

export const UpdateNotification: React.FC<UpdateNotificationProps> = ({
  updateInfo = null,
  checking = false,
  onClose,
  onRestartUpdate,
  actionState = 'hidden',
  actionVersion = null,
  actionProgress = 0,
  actionRetryStatus = '',
  actionError = '',
  actionErrorDetails = '',
  skipError = '',
  onPrimaryAction,
  onCancelUpdate,
  onSkipUpdate,
}) => {
  const { t, i18n } = useTranslation();

  useEscClose(true, onClose);

  const [showErrorDetails, setShowErrorDetails] = useState(false);
  const [isRestarting, setIsRestarting] = useState(false);
  const [isSkipping, setIsSkipping] = useState(false);

  const handleTriggerUpdate = useCallback(async () => {
    if (!onPrimaryAction) {
      return;
    }
    await onPrimaryAction();
  }, [onPrimaryAction]);

  const handleRestartNow = useCallback(async () => {
    setIsRestarting(true);
    try {
      if (onRestartUpdate) {
        await onRestartUpdate();
      } else {
        const { relaunch } = await import('@tauri-apps/plugin-process');
        await relaunch();
      }
    } catch (error) {
      console.error('Failed to relaunch after update:', error);
      setIsRestarting(false);
    }
  }, [onRestartUpdate]);

  const handleRetryDownload = useCallback(() => {
    setShowErrorDetails(false);
    void handleTriggerUpdate();
  }, [handleTriggerUpdate]);

  const handleSkipUpdate = useCallback(async () => {
    if (!onSkipUpdate || isSkipping) {
      return;
    }
    setIsSkipping(true);
    try {
      await onSkipUpdate();
    } finally {
      setIsSkipping(false);
    }
  }, [isSkipping, onSkipUpdate]);

  const handleFallbackDownload = async () => {
    if (updateInfo?.download_url) {
      try {
        await openUrl(updateInfo.download_url);
      } catch {
        window.open(updateInfo.download_url, '_blank');
      }
      onClose();
    }
  };

  // 根据语言选择显示中文还是英文更新日志
  const releaseNotes = useMemo(() => {
    if (!updateInfo) return '';
    const isZh = i18n.language.startsWith('zh');
    return isZh && updateInfo.release_notes_zh
      ? updateInfo.release_notes_zh
      : updateInfo.release_notes;
  }, [updateInfo, i18n.language]);

  // 简单的 Markdown 渲染
  const formattedNotes = useMemo(() => {
    if (!releaseNotes) return null;

    const lines = releaseNotes.split('\n');
    const elements: React.ReactNode[] = [];
    let key = 0;

    for (const line of lines) {
      const trimmed = line.trim();
      if (!trimmed) continue;

      if (trimmed.startsWith('### ')) {
        elements.push(
          <h4 key={key++} className="release-notes-heading">
            {trimmed.slice(4)}
          </h4>
        );
      } else if (trimmed.startsWith('## ')) {
        continue;
      } else if (trimmed.startsWith('- ')) {
        const content = trimmed.slice(2);
        const parts = content.split(/\*\*(.*?)\*\*/g);
        elements.push(
          <li key={key++} className="release-notes-item">
            {parts.map((part, i) =>
              i % 2 === 1 ? <strong key={i}>{part}</strong> : part
            )}
          </li>
        );
      }
    }

    return elements.length > 0 ? (
      <ul className="release-notes-list">{elements}</ul>
    ) : null;
  }, [releaseNotes]);

  const versionMatched = updateInfo ? actionVersion === updateInfo.latest_version : false;
  const isDownloading = Boolean(updateInfo) && actionState === 'downloading' && versionMatched;
  const isInstalling = actionState === 'installing';
  const isDownloaded = Boolean(updateInfo) && actionState === 'ready' && versionMatched;
  const clampedProgress = Math.max(0, Math.min(100, Math.round(actionProgress)));
  const mergedRetryStatus = actionRetryStatus;
  const showActionError =
    Boolean(actionError) && !checking && !isDownloading && !isInstalling;
  const isError = showActionError && !isDownloaded;
  const canSkipByState = actionState === 'available' || isDownloaded;
  const showSkipAction = Boolean(onSkipUpdate)
    && canSkipByState
    && !checking
    && !isDownloading
    && !isInstalling
    && !showActionError;
  const modalTitle = updateInfo
    ? t('update_notification.title')
    : t('settings.about.checkUpdate');

  const handleClose = () => {
    if (isRestarting || isInstalling) {
      return;
    }
    onClose();
  };

  if (!updateInfo) {
    const waitingMessage = checking
      ? mergedRetryStatus || t('settings.about.checking')
      : t('common.loading');

    return (
      <div className="modal-overlay update-overlay" onClick={handleClose}>
        <div className="modal update-modal" onClick={(event) => event.stopPropagation()}>
          <div className="modal-header">
            <h2 className="update-modal-title">
              <span className="update-icon">
                <Sparkles size={18} />
              </span>
              {modalTitle}
            </h2>
            <button
              className="modal-close"
              onClick={handleClose}
              aria-label={t('common.cancel')}
              disabled={isRestarting || isInstalling}
            >
              <X size={18} />
            </button>
          </div>
          <div className="modal-body update-modal-body">
            <div className="update-status update-status-retrying">
              <RefreshCw size={14} className="spin" />
              <span>{waitingMessage}</span>
            </div>
          </div>
          <div className="modal-footer">
            <button
              className="btn btn-secondary"
              onClick={handleClose}
              disabled={isRestarting || isInstalling}
            >
              {t('common.cancel')}
            </button>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="modal-overlay update-overlay" onClick={handleClose}>
      <div className="modal update-modal" onClick={(event) => event.stopPropagation()}>
        <div className="modal-header">
          <h2 className="update-modal-title">
            <span className="update-icon">
              <Sparkles size={18} />
            </span>
            {modalTitle}
          </h2>
          <button
            className="modal-close"
            onClick={handleClose}
            aria-label={t('common.cancel')}
            disabled={isRestarting || isInstalling}
          >
            <X size={18} />
          </button>
        </div>
        <div className="modal-body update-modal-body">
          <div className="update-version">v{updateInfo.latest_version}</div>
          <p className="update-message">
            {t('update_notification.message', { current: updateInfo.current_version })}
          </p>

          {isDownloading && (
            <div className="update-progress-container">
              <div className="update-progress-bar-row">
                <div className="update-progress-bar">
                  <div
                    className="update-progress-fill"
                    style={{ width: `${clampedProgress}%` }}
                  />
                </div>
                {onCancelUpdate && (
                  <button
                    type="button"
                    className="update-progress-cancel"
                    onClick={onCancelUpdate}
                    title={t('update_notification.cancelDownload', 'Cancel download')}
                  >
                    <XCircle size={16} />
                  </button>
                )}
              </div>
              <span className="update-progress-text">
                {t('update_notification.downloading', 'Downloading...')} {clampedProgress}%
              </span>
            </div>
          )}

          {mergedRetryStatus && (
            <div className="update-status update-status-retrying">
              <RefreshCw size={14} className="spin" />
              <span>{mergedRetryStatus}</span>
            </div>
          )}

          {isDownloaded && (
            <div className="update-status update-status-success">
              <Check size={16} />
              <span>
                {t('update_notification.silentReady', {
                  version: updateInfo.latest_version,
                })}
              </span>
            </div>
          )}

          {showActionError && (
            <div className="update-status update-status-error">
              <span>
                {actionError || t('update_notification.autoUpdateFailed', 'Auto-update failed. You can download manually.')}
              </span>
              {actionErrorDetails && (
                <button
                  type="button"
                  className="update-error-toggle"
                  onClick={() => setShowErrorDetails((prev) => !prev)}
                >
                  {showErrorDetails
                    ? t('update_notification.hideErrorDetails', 'Hide details')
                    : t('update_notification.showErrorDetails', 'View details')}
                </button>
              )}
              {showErrorDetails && actionErrorDetails && (
                <span className="update-error-detail">{actionErrorDetails}</span>
              )}
            </div>
          )}

          {skipError && (
            <div className="update-status update-status-error">
              <span>{skipError}</span>
            </div>
          )}

          {formattedNotes && (
            <div className="release-notes">
              <h3 className="release-notes-title">{t('update_notification.whatsNew', "What's New")}</h3>
              <div className="release-notes-content">
                {formattedNotes}
              </div>
            </div>
          )}
        </div>
        <div className="modal-footer">
          <button
            className="btn btn-secondary"
            onClick={handleClose}
            disabled={isRestarting || isInstalling}
          >
            {isDownloading
              ? t('update_notification.later', 'Later')
              : isDownloaded
                ? t('update_notification.later', 'Later')
                : t('common.cancel')}
          </button>
          {showSkipAction && (
            <button
              className="btn btn-ghost"
              onClick={handleSkipUpdate}
              disabled={isSkipping || isRestarting || isInstalling}
            >
              {t('update_notification.skipThisVersion')}
            </button>
          )}
          {isError ? (
            <>
              <button className="btn btn-secondary" onClick={handleRetryDownload}>
                <RefreshCw size={16} />
                {t('update_notification.retryDownload', 'Retry Download')}
              </button>
              <button className="btn btn-primary" onClick={handleFallbackDownload}>
                <Download size={16} />
                {t('update_notification.action')}
              </button>
            </>
          ) : isInstalling ? (
            <button className="btn btn-primary" disabled>
              <RefreshCw size={16} className="spin" />
              {t('update_notification.installing', 'Installing...')}
            </button>
          ) : isRestarting ? (
            <button className="btn btn-primary" disabled>
              <RefreshCw size={16} className="spin" />
              {t('update_notification.restarting', 'Restarting...')}
            </button>
          ) : isDownloaded ? (
            <button className="btn btn-primary" onClick={handleRestartNow}>
              <RefreshCw size={16} />
              {t('update_notification.restartNow', 'Restart')}
            </button>
          ) : (
            <button
              className="btn btn-primary"
              onClick={handleTriggerUpdate}
              disabled={isDownloading || !onPrimaryAction}
            >
              {isDownloading ? (
                <>
                  <RefreshCw size={16} className="spin" />
                  {t('update_notification.downloading', 'Downloading...')}
                </>
              ) : (
                <>
                  <Download size={16} />
                  {t('update_notification.updateNow', 'Update Now')}
                </>
              )}
            </button>
          )}
        </div>
      </div>
    </div>
  );
};
