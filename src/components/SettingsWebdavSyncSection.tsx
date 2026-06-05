import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { ChevronRight, Cloud, Download, RefreshCw, RotateCcw, Trash2, Upload } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import {
  DataTransferImportResult,
  importDataTransferJson,
} from '../services/dataTransferService';
import {
  createManagedBackup,
  getAutoBackupSettings,
  getSelectionFromAutoBackupSettings,
} from '../services/scheduledBackupService';
import {
  WEBDAV_SYNC_STATE_CHANGED_EVENT,
  WebdavBackupFileEntry,
  WebdavSyncSettings,
  deleteWebdavBackupFile,
  getWebdavSyncSettings,
  listWebdavBackupFiles,
  readWebdavBackupFile,
  saveWebdavSyncSettings,
  testWebdavSyncConnection,
  uploadAutoBackupToWebdav,
} from '../services/webdavSyncService';

type WebdavFeedbackTone = 'loading' | 'success' | 'error';

interface WebdavFeedback {
  tone: WebdavFeedbackTone;
  text: string;
}

function normalizeError(error: unknown): string {
  const msg = String(error).replace(/^Error:\s*/, '');
  // 针对坚果云等 WebDAV 服务的祖先目录不存在报错进行友好提示
  if (msg.includes('AncestorsNotFound') || msg.includes('The ancestors of this location does not found')) {
    return `${msg}\n\n提示：检测到祖先目录不存在。如果您使用坚果云 WebDAV，坚果云限制了不能在根目录下直接创建文件夹，请确认您的“远端目录”是否填写正确。例如应填写为已存在的同步文件夹路径，如“我的坚果云/cockpit-tools”；或者先在坚果云网页端创建一个名为“cockpit-tools”的同步文件夹，然后在这里将远端目录填为“cockpit-tools”。`;
  }
  return msg;
}

function formatFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(bytes >= 10 * 1024 ? 0 : 1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(bytes >= 10 * 1024 * 1024 ? 0 : 1)} MB`;
}

function formatRemoteTime(value: string | null | undefined, fallback: string): string {
  if (!value) return fallback;
  const date = new Date(value);
  if (!Number.isFinite(date.getTime())) return value;
  return date.toLocaleString();
}

function buildImportFeedback(
  result: DataTransferImportResult,
  t: (key: string, options?: Record<string, unknown>) => string,
): string {
  const parts: string[] = [];
  if (result.imported_account_count > 0) {
    parts.push(t('settings.transfer.feedback.accountsImported', {
      count: result.imported_account_count,
      defaultValue: '已导入 {{count}} 个账号',
    }));
  }
  if (result.config_result?.applied) {
    parts.push(t('settings.transfer.feedback.configImported', {
      defaultValue: '配置数据已恢复',
    }));
  }
  if (result.config_result?.needs_restart) {
    parts.push(t('settings.transfer.feedback.restartRequired', {
      defaultValue: '部分变更需重启应用后生效',
    }));
  }
  return parts.length > 0
    ? parts.join(' · ')
    : t('settings.webdav.feedback.restoreEmpty', {
        defaultValue: '远端备份未恢复任何数据',
      });
}

export function SettingsWebdavSyncSection() {
  const { t } = useTranslation();

  const [settings, setSettings] = useState<WebdavSyncSettings | null>(null);
  const [enabled, setEnabled] = useState(false);
  const [url, setUrl] = useState('https://dav.jianguoyun.com/dav/');
  const [username, setUsername] = useState('');
  const [passwordInput, setPasswordInput] = useState('');
  const [clearPassword, setClearPassword] = useState(false);
  const [remoteDir, setRemoteDir] = useState('cockpit-tools');
  const [remoteFiles, setRemoteFiles] = useState<WebdavBackupFileEntry[]>([]);
  const [feedback, setFeedback] = useState<WebdavFeedback | null>(null);
  const [isRemoteExpanded, setIsRemoteExpanded] = useState(false);
  const isRemoteExpandedRef = useRef(isRemoteExpanded);

  useEffect(() => {
    isRemoteExpandedRef.current = isRemoteExpanded;
  }, [isRemoteExpanded]);
  const [settingsLoading, setSettingsLoading] = useState(true);
  const [remoteLoading, setRemoteLoading] = useState(false);
  const [testing, setTesting] = useState(false);
  const [uploading, setUploading] = useState(false);
  const [restoringFile, setRestoringFile] = useState<string | null>(null);
  const [deletingFile, setDeletingFile] = useState<string | null>(null);
  const [syncingLatest, setSyncingLatest] = useState(false);

  const applySettings = useCallback((next: WebdavSyncSettings) => {
    setSettings(next);
    setEnabled(next.enabled);
    setUrl(next.url);
    setUsername(next.username);
    setRemoteDir(next.remote_dir);
    setPasswordInput('');
    setClearPassword(false);
  }, []);

  const loadRemoteFiles = useCallback(async () => {
    setRemoteLoading(true);
    try {
      const files = await listWebdavBackupFiles();
      setRemoteFiles(files);
    } catch (error) {
      setRemoteFiles([]);
      setFeedback({
        tone: 'error',
        text: t('settings.webdav.feedback.listFailed', {
          error: normalizeError(error),
          defaultValue: '加载远端备份失败：{{error}}',
        }),
      });
    } finally {
      setRemoteLoading(false);
    }
  }, [t]);

  const loadSettings = useCallback(async () => {
    setSettingsLoading(true);
    try {
      const next = await getWebdavSyncSettings();
      applySettings(next);
      if (next.has_password && next.username.trim() && isRemoteExpandedRef.current) {
        await loadRemoteFiles();
      }
    } catch (error) {
      setFeedback({
        tone: 'error',
        text: t('settings.webdav.feedback.loadSettingsFailed', {
          error: normalizeError(error),
          defaultValue: '加载 WebDAV 设置失败：{{error}}',
        }),
      });
    } finally {
      setSettingsLoading(false);
    }
  }, [applySettings, loadRemoteFiles, t]);

  useEffect(() => {
    void loadSettings();
  }, [loadSettings]);

  useEffect(() => {
    const handleStateChanged = () => {
      void loadSettings();
    };
    window.addEventListener(WEBDAV_SYNC_STATE_CHANGED_EVENT, handleStateChanged);
    return () => window.removeEventListener(WEBDAV_SYNC_STATE_CHANGED_EVENT, handleStateChanged);
  }, [loadSettings]);

  const persistSettings = useCallback(async () => {
    const next = await saveWebdavSyncSettings({
      enabled,
      url,
      username,
      password: passwordInput.trim() ? passwordInput : null,
      clearPassword,
      remoteDir,
    });
    applySettings(next);
    return next;
  }, [applySettings, clearPassword, enabled, passwordInput, remoteDir, url, username]);

  const handleTestAndSave = useCallback(async () => {
    setTesting(true);
    setFeedback({
      tone: 'loading',
      text: t('settings.webdav.feedback.testingAndSaving', {
        defaultValue: '正在保存并测试 WebDAV 连接...',
      }),
    });
    try {
      const next = await persistSettings();
      const result = await testWebdavSyncConnection({
        url,
        username,
        password: passwordInput.trim() ? passwordInput : null,
        clearPassword,
        remoteDir,
      });
      if (result.ok) {
        setFeedback({
          tone: 'success',
          text: t('settings.webdav.feedback.saveAndTestSuccess', {
            defaultValue: '配置已保存，连接测试成功！',
          }),
        });
        if (next.has_password && next.username.trim()) {
          await loadRemoteFiles();
        }
      } else {
        setFeedback({
          tone: 'error',
          text: t('settings.webdav.feedback.saveSuccessTestFailed', {
            message: result.message,
            defaultValue: '配置已保存，但连接测试失败：{{message}}',
          }),
        });
      }
    } catch (error) {
      setFeedback({
        tone: 'error',
        text: t('settings.webdav.feedback.saveAndTestFailed', {
          error: normalizeError(error),
          defaultValue: '保存并测试连接失败：{{error}}',
        }),
      });
    } finally {
      setTesting(false);
    }
  }, [clearPassword, passwordInput, remoteDir, url, username, persistSettings, loadRemoteFiles, t]);

  const handleUploadNow = useCallback(async () => {
    setUploading(true);
    setFeedback({
      tone: 'loading',
      text: t('settings.webdav.feedback.uploading', {
        defaultValue: '正在生成并上传备份...',
      }),
    });
    try {
      const savedSettings = await persistSettings();
      if (!savedSettings.enabled) {
        throw new Error(t('settings.webdav.errors.notEnabled', {
          defaultValue: '请先启用 WebDAV 同步',
        }));
      }
      const backupSettings = await getAutoBackupSettings();
      const backup = await createManagedBackup({
        trigger: 'manual',
        selection: getSelectionFromAutoBackupSettings(backupSettings),
        retentionDays: backupSettings.retention_days,
        markAsLastRun: true,
      });
      const result = await uploadAutoBackupToWebdav(backup.file_name);
      await loadSettings();
      setFeedback({
        tone: 'success',
        text: t('settings.webdav.feedback.uploadSuccess', {
          count: result.uploaded_files.length,
          deleted: result.deleted_files.length,
          defaultValue: '已上传 {{count}} 个文件，清理 {{deleted}} 个远端过期备份',
        }),
      });
    } catch (error) {
      setFeedback({
        tone: 'error',
        text: t('settings.webdav.feedback.uploadFailed', {
          error: normalizeError(error),
          defaultValue: 'WebDAV 上传失败：{{error}}',
        }),
      });
    } finally {
      setUploading(false);
    }
  }, [loadSettings, persistSettings, t]);

  const handleRefreshRemoteFiles = useCallback(async () => {
    setFeedback(null);
    await loadRemoteFiles();
  }, [loadRemoteFiles]);

  const toggleRemoteExpanded = useCallback(() => {
    setIsRemoteExpanded((prev) => {
      const next = !prev;
      if (next) {
        void loadRemoteFiles();
      }
      return next;
    });
  }, [loadRemoteFiles]);

  const handleRestore = useCallback(
    async (file: WebdavBackupFileEntry) => {
      if (file.file_kind !== 'json') return;
      if (!window.confirm(t('settings.webdav.restoreConfirm', {
        name: file.file_name,
        defaultValue: '确认从远端备份 {{name}} 恢复数据？',
      }))) {
        return;
      }
      setRestoringFile(file.file_name);
      setFeedback({
        tone: 'loading',
        text: t('settings.webdav.feedback.restoring', {
          defaultValue: '正在读取远端备份并导入...',
        }),
      });
      try {
        const content = await readWebdavBackupFile(file.file_name);
        const result = await importDataTransferJson(content, {
          includeAccounts: true,
          includeConfig: true,
        });
        setFeedback({
          tone: 'success',
          text: buildImportFeedback(result, t),
        });
        await loadSettings();
      } catch (error) {
        setFeedback({
          tone: 'error',
          text: t('common.shared.import.failedMsg', {
            error: normalizeError(error),
            defaultValue: '导入失败：{{error}}',
          }),
        });
      } finally {
        setRestoringFile(null);
      }
    },
    [loadSettings, t],
  );

  const handleDelete = useCallback(
    async (file: WebdavBackupFileEntry) => {
      if (!window.confirm(t('settings.webdav.deleteConfirm', {
        name: file.file_name,
        defaultValue: '确认删除远端备份 {{name}}？',
      }))) {
        return;
      }
      setDeletingFile(file.file_name);
      try {
        await deleteWebdavBackupFile(file.file_name);
        await loadRemoteFiles();
        setFeedback({
          tone: 'success',
          text: t('settings.webdav.feedback.deleteSuccess', {
            defaultValue: '远端备份已删除',
          }),
        });
      } catch (error) {
        setFeedback({
          tone: 'error',
          text: t('settings.webdav.feedback.deleteFailed', {
            error: normalizeError(error),
            defaultValue: '删除远端备份失败：{{error}}',
          }),
        });
      } finally {
        setDeletingFile(null);
      }
    },
    [loadRemoteFiles, t],
  );

  const handleSyncConfigLatest = useCallback(async () => {
    setSyncingLatest(true);
    setFeedback({
      tone: 'loading',
      text: t('settings.webdav.feedback.syncingLatest', {
        defaultValue: '正在同步远端最新配置...',
      }),
    });
    try {
      const files = await listWebdavBackupFiles();
      setRemoteFiles(files);

      const latestJson = files
        .filter((f) => f.file_kind === 'json')
        .sort((a, b) => {
          const timeA = a.modified_at ? new Date(a.modified_at).getTime() : 0;
          const timeB = b.modified_at ? new Date(b.modified_at).getTime() : 0;
          return timeB - timeA;
        })[0];

      if (!latestJson) {
        setFeedback({
          tone: 'error',
          text: t('settings.webdav.feedback.noRemoteBackup', {
            defaultValue: '远端未发现可同步的备份文件',
          }),
        });
        return;
      }

      if (!window.confirm(t('settings.webdav.syncLatestConfirm', {
        name: latestJson.file_name,
        defaultValue: '确认拉取最新的远端备份 {{name}} 并同步到本地？此操作将覆盖本地现有数据。',
      }))) {
        setFeedback(null);
        return;
      }

      setFeedback({
        tone: 'loading',
        text: t('settings.webdav.feedback.restoring', {
          defaultValue: '正在读取远端备份并导入...',
        }),
      });

      const content = await readWebdavBackupFile(latestJson.file_name);
      const result = await importDataTransferJson(content, {
        includeAccounts: true,
        includeConfig: true,
      });

      setFeedback({
        tone: 'success',
        text: buildImportFeedback(result, t),
      });
      await loadSettings();
    } catch (error) {
      setFeedback({
        tone: 'error',
        text: t('settings.webdav.feedback.syncFailed', {
          error: normalizeError(error),
          defaultValue: '同步最新配置失败：{{error}}',
        }),
      });
    } finally {
      setSyncingLatest(false);
    }
  }, [loadSettings, t]);

  const hasUsableSavedCredential = Boolean(settings?.has_password && settings.username.trim());
  const busy = testing || uploading || remoteLoading || restoringFile !== null || deletingFile !== null || syncingLatest;
  const passwordPlaceholder = settings?.has_password
    ? t('settings.webdav.passwordSavedPlaceholder', {
        defaultValue: '已保存，留空保持不变',
      })
    : t('settings.webdav.passwordPlaceholder', {
        defaultValue: '坚果云第三方应用密码',
      });
  const lastUploadText = useMemo(() => {
    const time = formatRemoteTime(
      settings?.last_upload_at,
      t('settings.webdav.never', { defaultValue: '尚未同步' }),
    );
    return settings?.last_upload_file_name ? `${time} · ${settings.last_upload_file_name}` : time;
  }, [settings?.last_upload_at, settings?.last_upload_file_name, t]);
  const lastDownloadText = useMemo(() => {
    const time = formatRemoteTime(
      settings?.last_download_at,
      t('settings.webdav.never', { defaultValue: '尚未恢复' }),
    );
    return settings?.last_download_file_name ? `${time} · ${settings.last_download_file_name}` : time;
  }, [settings?.last_download_at, settings?.last_download_file_name, t]);

  return (
    <>
      <div className="group-title">{t('settings.webdav.groupTitle', 'WebDAV 同步')}</div>
      <div className="settings-group">
        <div className="settings-row">
          <div className="row-label">
            <div className="row-title">{t('settings.webdav.enabledTitle', 'WebDAV 备份同步')}</div>
            <div className="row-desc">
              {t('settings.webdav.enabledDesc', '默认使用坚果云，也可改为任意 WebDAV 服务。')}
            </div>
          </div>
          <div className="row-control">
            <label className="switch" aria-label={t('settings.webdav.enabledTitle', 'WebDAV 备份同步')}>
              <input
                type="checkbox"
                checked={enabled}
                disabled={settingsLoading || busy}
                onChange={(event) => setEnabled(event.target.checked)}
              />
              <span className="slider" />
            </label>
          </div>
        </div>

        <div className="settings-row">
          <div className="row-label">
            <div className="row-title">{t('settings.webdav.urlTitle', '服务地址')}</div>
            <div className="row-desc">{t('settings.webdav.urlDesc', '坚果云默认地址：https://dav.jianguoyun.com/dav/')}</div>
          </div>
          <div className="row-control row-control--grow">
            <input
              className="settings-select settings-select--input-mode settings-webdav-input"
              value={url}
              disabled={settingsLoading || busy}
              onChange={(event) => setUrl(event.target.value)}
            />
          </div>
        </div>

        <div className="settings-row">
          <div className="row-label">
            <div className="row-title">{t('settings.webdav.usernameTitle', '账号')}</div>
            <div className="row-desc">{t('settings.webdav.usernameDesc', '坚果云使用账号邮箱。')}</div>
          </div>
          <div className="row-control row-control--grow">
            <input
              className="settings-select settings-select--input-mode settings-webdav-input"
              value={username}
              disabled={settingsLoading || busy}
              placeholder={t('settings.webdav.usernamePlaceholder', '邮箱或用户名')}
              onChange={(event) => setUsername(event.target.value)}
            />
          </div>
        </div>

        <div className="settings-row">
          <div className="row-label">
            <div className="row-title">{t('settings.webdav.passwordTitle', '应用密码')}</div>
            <div className="row-desc">{t('settings.webdav.passwordDesc', '保存到本地配置；留空会保留已保存密码。')}</div>
          </div>
          <div className="row-control row-control--grow settings-webdav-password-control">
            <input
              type="password"
              className="settings-select settings-select--input-mode settings-webdav-input"
              value={passwordInput}
              disabled={settingsLoading || busy}
              placeholder={passwordPlaceholder}
              onChange={(event) => setPasswordInput(event.target.value)}
            />
          </div>
        </div>

        <div className="settings-row">
          <div className="row-label">
            <div className="row-title">{t('settings.webdav.remoteDirTitle', '远端目录')}</div>
            <div className="row-desc">{t('settings.webdav.remoteDirDesc', '只管理该目录下的 Cockpit 备份文件。')}</div>
          </div>
          <div className="row-control row-control--grow">
            <input
              className="settings-select settings-select--input-mode settings-webdav-input"
              value={remoteDir}
              disabled={settingsLoading || busy}
              onChange={(event) => setRemoteDir(event.target.value)}
            />
          </div>
        </div>

        <div className="settings-row settings-row--align-start">
          <div className="row-label">
            <div className="row-title">{t('settings.webdav.statusTitle', '同步状态')}</div>
            <div className="row-desc">{t('settings.webdav.statusDesc', '远端恢复需要手动选择备份，不会静默覆盖本地数据。')}</div>
            <div className="settings-backup-inline-meta">
              {t('settings.webdav.lastUpload', {
                time: lastUploadText,
                defaultValue: '最近上传：{{time}}',
              })}
              <br />
              {t('settings.webdav.lastDownload', {
                time: lastDownloadText,
                defaultValue: '最近恢复：{{time}}',
              })}
            </div>
          </div>
          <div className="row-control settings-webdav-actions">
            <button className="btn btn-secondary" disabled={settingsLoading || busy} onClick={() => void handleTestAndSave()}>
              {testing ? <RefreshCw size={16} className="loading-spinner" /> : <Cloud size={16} />}
              {t('settings.webdav.testAction', '测试/保存配置')}
            </button>
            <button
              className="btn btn-primary"
              disabled={settingsLoading || busy || !enabled}
              onClick={() => void handleUploadNow()}
            >
              {uploading ? <RefreshCw size={16} className="loading-spinner" /> : <Upload size={16} />}
              {t('settings.webdav.uploadAction', '生成并上传')}
            </button>
            <button
              className="btn btn-secondary"
              disabled={settingsLoading || busy || !enabled}
              onClick={() => void handleSyncConfigLatest()}
            >
              {syncingLatest ? <RefreshCw size={16} className="loading-spinner" /> : <RefreshCw size={16} />}
              {t('settings.webdav.syncAction', '同步配置')}
            </button>
          </div>
        </div>

        {feedback && (
          <div className="settings-transfer-feedback-wrap">
            <div className={`add-feedback ${feedback.tone}`}>{feedback.text}</div>
          </div>
        )}

        <div
          className="settings-row settings-row--align-start settings-row--no-border settings-webdav-remote-header"
          style={{ cursor: 'pointer', userSelect: 'none' }}
          onClick={toggleRemoteExpanded}
        >
          <div className="row-label">
            <div className="row-title" style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
              <span className={`settings-webdav-chevron ${isRemoteExpanded ? 'expanded' : ''}`}>
                <ChevronRight size={16} />
              </span>
              {t('settings.webdav.remoteListTitle', '远端备份')}
            </div>
            <div className="row-desc">{t('settings.webdav.remoteListDesc', '仅显示匹配 Cockpit 自动/手动备份命名的 JSON 与 ZIP 文件。')}</div>
          </div>
          <div className="row-control" onClick={(e) => e.stopPropagation()}>
            {isRemoteExpanded && (
              <button
                className="btn btn-secondary"
                disabled={settingsLoading || busy || !hasUsableSavedCredential}
                onClick={() => void handleRefreshRemoteFiles()}
              >
                {remoteLoading ? <RefreshCw size={16} className="loading-spinner" /> : <RotateCcw size={16} />}
                {t('common.refresh', '刷新')}
              </button>
            )}
          </div>
        </div>

        {isRemoteExpanded && (
          <div className="settings-webdav-list-wrap">
            {remoteLoading ? (
              <div className="settings-backup-empty">
                <div className="settings-backup-empty-title">{t('common.loading', '加载中...')}</div>
              </div>
            ) : remoteFiles.length === 0 ? (
              <div className="settings-backup-empty">
                <div className="settings-backup-empty-title">{t('settings.webdav.emptyTitle', '暂无远端备份')}</div>
                <div className="settings-backup-empty-desc">{t('settings.webdav.emptyDesc', '保存设置后可以生成并上传一份备份。')}</div>
              </div>
            ) : (
              <div className="settings-backup-list">
                {remoteFiles.map((file) => {
                  const isRestoring = restoringFile === file.file_name;
                  const isDeleting = deletingFile === file.file_name;
                  return (
                    <div className="settings-backup-item" key={file.file_name}>
                      <div className="settings-backup-item-head">
                        <div>
                          <div className="settings-backup-item-name">{file.file_name}</div>
                          <div className="settings-backup-item-tags">
                            <span className="settings-backup-tag">{file.file_kind.toUpperCase()}</span>
                          </div>
                        </div>
                      </div>
                      <div className="settings-backup-item-meta">
                        <span>
                          {t('settings.transfer.backup.fileTime', {
                            time: formatRemoteTime(file.modified_at, t('settings.webdav.unknownTime', '未知')),
                            defaultValue: '时间：{{time}}',
                          })}
                        </span>
                        <span>
                          {t('settings.transfer.backup.fileSize', {
                            size: formatFileSize(file.size_bytes),
                            defaultValue: '大小：{{size}}',
                          })}
                        </span>
                      </div>
                      <div className="settings-backup-item-actions">
                        <button
                          className="btn btn-secondary"
                          disabled={busy || file.file_kind !== 'json'}
                          onClick={() => void handleRestore(file)}
                        >
                          {isRestoring ? <RefreshCw size={16} className="loading-spinner" /> : <Download size={16} />}
                          {t('settings.webdav.restoreAction', '恢复')}
                        </button>
                        <button
                          className="btn btn-secondary"
                          disabled={busy}
                          onClick={() => void handleDelete(file)}
                        >
                          {isDeleting ? <RefreshCw size={16} className="loading-spinner" /> : <Trash2 size={16} />}
                          {t('settings.transfer.backup.deleteAction', '删除')}
                        </button>
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        )}
      </div>
    </>
  );
}
