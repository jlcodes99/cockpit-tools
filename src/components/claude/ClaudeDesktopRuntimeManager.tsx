import { useEffect, useState } from 'react';
import { HardDrive, RefreshCw, Trash2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import * as claudeService from '../../services/claudeService';

interface ClaudeDesktopRuntimeManagerProps {
  disabled?: boolean;
  onError: (message: string | null) => void;
  onSuccess: (message: string) => void;
}

export function ClaudeDesktopRuntimeManager({
  disabled = false,
  onError,
  onSuccess,
}: ClaudeDesktopRuntimeManagerProps) {
  const { t } = useTranslation();
  const [confirming, setConfirming] = useState(false);
  const [uninstalling, setUninstalling] = useState(false);
  const [sizeBytes, setSizeBytes] = useState<number | null>(null);

  useEffect(() => {
    let active = true;
    void claudeService.getClaudeDesktopLoginComponentStorage()
      .then((result) => {
        if (active) setSizeBytes(result.sizeBytes);
      })
      .catch(() => {
        if (active) setSizeBytes(null);
      });
    return () => {
      active = false;
    };
  }, [disabled]);

  const formatStorageSize = (bytes: number | null) => {
    if (bytes === null) return t('claude.desktopOAuth.storageCalculating', '计算中…');
    if (bytes < 1024) return `${bytes} B`;
    const units = ['KB', 'MB', 'GB', 'TB'];
    let value = bytes / 1024;
    let unitIndex = 0;
    while (value >= 1024 && unitIndex < units.length - 1) {
      value /= 1024;
      unitIndex += 1;
    }
    return `${value >= 10 ? value.toFixed(0) : value.toFixed(1)} ${units[unitIndex]}`;
  };

  const handleUninstall = async () => {
    if (uninstalling) return;
    setUninstalling(true);
    onError(null);
    try {
      await claudeService.uninstallClaudeDesktopLoginComponent();
      setSizeBytes(0);
      setConfirming(false);
      onSuccess(t('claude.desktopOAuth.uninstallSuccess', 'Claude 登录组件已卸载，已清理本地缓存空间。'));
    } catch (error) {
      onError(t('claude.desktopOAuth.uninstallFailed', '卸载 Claude 登录组件失败：{{error}}', {
        error: String(error).replace(/^Error:\s*/, ''),
      }));
    } finally {
      setUninstalling(false);
    }
  };

  if (confirming) {
    return (
      <div
        className="claude-desktop-runtime-manager claude-desktop-runtime-manager--confirming"
        role="group"
      >
        <div className="claude-desktop-runtime-manager__copy">
          <strong>{t('claude.desktopOAuth.uninstallTitle', '卸载登录组件')}</strong>
          <p>
            {t(
              'claude.desktopOAuth.uninstallConfirm',
              '确定卸载本地 Electron 登录组件吗？下次使用 Claude 登录时需要重新下载。',
            )}
          </p>
        </div>
        <div className="claude-desktop-runtime-manager__actions">
          <button
            type="button"
            className="btn btn-secondary"
            onClick={() => setConfirming(false)}
            disabled={uninstalling}
          >
            {t('common.cancel', '取消')}
          </button>
          <button
            type="button"
            className="btn btn-danger"
            onClick={() => void handleUninstall()}
            disabled={uninstalling}
          >
            {uninstalling && <RefreshCw size={14} className="loading-spinner" />}
            {uninstalling
              ? t('claude.desktopOAuth.uninstalling', '正在卸载...')
              : t('claude.desktopOAuth.uninstall', '卸载并释放空间')}
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="claude-desktop-runtime-manager">
      <div className="claude-desktop-runtime-manager__main">
        <div className="claude-desktop-runtime-manager__icon" aria-hidden="true">
          <HardDrive size={18} />
        </div>
        <div className="claude-desktop-runtime-manager__copy">
          <div className="claude-desktop-runtime-manager__title-row">
            <strong>{t('claude.desktopOAuth.storageTitle', '登录组件缓存')}</strong>
            <span className="claude-desktop-runtime-manager__size">
              {t('claude.desktopOAuth.storageUsage', '占用 {{size}}', {
                size: formatStorageSize(sizeBytes),
              })}
            </span>
          </div>
          <p>
            {t(
              'claude.desktopOAuth.uninstallDesc',
              '卸载会删除本地 Electron 登录组件、下载缓存和未完成登录的临时文件；不会删除已保存的 Claude 账号。',
            )}
          </p>
        </div>
      </div>
      <button
        type="button"
        className="btn btn-secondary claude-desktop-runtime-manager__button"
        onClick={() => {
          onError(null);
          setConfirming(true);
        }}
        disabled={disabled || uninstalling}
      >
        <Trash2 size={14} />
        {t('claude.desktopOAuth.uninstall', '卸载并释放空间')}
      </button>
    </div>
  );
}
