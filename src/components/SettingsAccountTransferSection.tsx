import {
  ChangeEvent,
  Dispatch,
  ReactNode,
  SetStateAction,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import { Archive, ChevronLeft, Download, FolderOpen, RefreshCw, Trash2, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { createPortal } from 'react-dom';
import { listen } from '@tauri-apps/api/event';
import { open, save } from '@tauri-apps/plugin-dialog';
import { ExportJsonModal } from './ExportJsonModal';
import { useExportJsonModal } from '../hooks/useExportJsonModal';
import { useEscClose } from '../hooks/useEscClose';
import {
  AccountTransferImportProgress,
  AccountTransferImportProgressDetail,
} from '../services/accountTransferService';
import {
  DataTransferImportResult,
  DataTransferSelection,
  exportDataTransferJson,
  getDataTransferFileNameBase,
  importDataTransferJson,
} from '../services/dataTransferService';
import {
  AUTO_BACKUP_STATE_CHANGED_EVENT,
  BACKUP_DIRECTORY_MIGRATION_PROGRESS_EVENT,
  BackupDirectoryChangeResult,
  BackupDirectoryMigrationPreview,
  BackupDirectoryMigrationProgress,
  BackupUsageSummary,
  AutoBackupFileEntry,
  AutoBackupMode,
  AutoBackupSettings,
  autoBackupModeToSelection,
  cancelBackupDirectoryChange,
  cleanupAutoBackupFiles,
  cleanupBehaviorBackups,
  changeBackupDirectory,
  copyAutoBackupFile,
  createManagedBackup,
  deleteAutoBackupFile,
  extractAutoBackupPlatformJson,
  getAutoBackupSettings,
  getBackupUsage,
  getSelectionFromAutoBackupSettings,
  listAutoBackupFiles,
  normalizeAutoBackupPlatforms,
  openAutoBackupDir,
  previewBackupDirectoryChange,
  readAutoBackupFile,
  saveAutoBackupSettings,
  selectionToAutoBackupMode,
} from '../services/scheduledBackupService';
import { ALL_PLATFORM_IDS, PlatformId } from '../types/platform';
import { getPlatformLabel } from '../utils/platformMeta';
import { presentWindowsOperationError } from '../utils/windowsOperationDialog';

type TransferFeedbackTone = 'loading' | 'success' | 'error';
type FeedbackSetter = Dispatch<SetStateAction<TransferFeedback | null>>;

interface TransferFeedback {
  tone: TransferFeedbackTone;
  text: string;
}

const DEFAULT_TRANSFER_SELECTION: DataTransferSelection = {
  includeAccounts: true,
  includeConfig: true,
};

const BACKUP_FILE_NAME_REGEX =
  /^cockpit_(auto|manual)_backup_(full|accounts|config)_\d{4}-\d{2}-\d{2}_\d{2}-\d{2}-\d{2}\.(json|zip)$/i;

const BACKUP_USAGE_SOURCE_ORDER = [
  'scheduled',
  'claude',
  'codex',
  'workbuddy',
  'codebuddy',
  'trae',
] as const;

function normalizeError(error: unknown): string {
  return String(error).replace(/^Error:\s*/, '');
}

function normalizeImportErrorMessage(
  rawError: string,
  fallbackMessage: string,
  t: (key: string, options?: Record<string, unknown>) => string,
): string {
  if (rawError === 'transfer_selection_required') {
    return t('settings.transfer.errors.selectionRequired');
  }
  if (rawError === 'accounts_section_required') {
    return t('settings.transfer.errors.accountsOnlyFile');
  }
  if (rawError === 'selected_sections_missing') {
    return t('settings.transfer.errors.selectedSectionsMissing');
  }
  if (rawError === 'unsupported_legacy_account_json') {
    return t('settings.transfer.errors.unsupportedLegacyFile');
  }
  if (rawError === 'invalid_bundle_version') {
    return t('settings.transfer.errors.unsupportedVersion');
  }
  if (rawError.startsWith('invalid_')) {
    return fallbackMessage;
  }
  return rawError;
}

function renderToBody(node: ReactNode) {
  if (typeof window === 'undefined' || typeof document === 'undefined') {
    return null;
  }
  return createPortal(node, document.body);
}

function hasSelection(selection: DataTransferSelection): boolean {
  return selection.includeAccounts || selection.includeConfig;
}

function firstAccountImportError(result: DataTransferImportResult): string | null {
  return result.account_result?.details.find((detail) => detail.error)?.error ?? null;
}

function parseManagedBackupMeta(fileName: string): {
  trigger: 'auto' | 'manual' | null;
  mode: AutoBackupMode | null;
} {
  const match = BACKUP_FILE_NAME_REGEX.exec(fileName);
  if (!match) {
    return {
      trigger: null,
      mode: null,
    };
  }
  const [, trigger, mode] = match;
  return {
    trigger: trigger === 'auto' || trigger === 'manual' ? trigger : null,
    mode: mode === 'full' || mode === 'accounts' || mode === 'config' ? mode : null,
  };
}

function formatFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(bytes >= 10 * 1024 ? 0 : 1)} KB`;
  if (bytes < 1024 * 1024 * 1024) {
    return `${(bytes / (1024 * 1024)).toFixed(bytes >= 10 * 1024 * 1024 ? 0 : 1)} MB`;
  }
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

function stripBackupExtension(fileName: string): string {
  return fileName.replace(/\.(json|zip)$/i, '');
}

function buildBackupJsonFileName(fileName: string): string {
  return `${stripBackupExtension(fileName)}.json`;
}

function buildPlatformBackupFileName(fileName: string, platform: PlatformId): string {
  return `${stripBackupExtension(fileName)}_${platform}.json`;
}

function createBackupMigrationId(): string {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
    return crypto.randomUUID();
  }
  return `backup-migration-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

function backupMigrationPercent(progress: BackupDirectoryMigrationProgress | null): number {
  if (!progress) return 0;
  if (progress.phase === 'completed') return 100;
  if (progress.phase === 'cleaning') return 97;
  if (progress.phase === 'switching') return 94;
  if (progress.phase === 'verifying') return 90;
  if (progress.phase === 'scanning') return 2;
  if (progress.total_size_bytes <= 0) return progress.phase === 'copying' ? 85 : 0;
  return Math.min(85, Math.max(5, 5 + Math.round(
    (progress.processed_size_bytes / progress.total_size_bytes) * 80,
  )));
}

interface SettingsAccountTransferSectionProps {
  directoryEntryOnly?: boolean;
}

export function SettingsAccountTransferSection({
  directoryEntryOnly = false,
}: SettingsAccountTransferSectionProps = {}) {
  const { t } = useTranslation();
  const importFileInputRef = useRef<HTMLInputElement | null>(null);
  const modalBodyRef = useRef<HTMLDivElement | null>(null);
  const progressListRef = useRef<HTMLDivElement | null>(null);
  const backupMigrationIdRef = useRef<string | null>(null);

  const [showExportOptionsModal, setShowExportOptionsModal] = useState(false);
  const [showImportModal, setShowImportModal] = useState(false);
  const [showBackupManagerModal, setShowBackupManagerModal] = useState(false);
  const [importing, setImporting] = useState(false);
  const [jsonInput, setJsonInput] = useState('');
  const [feedback, setFeedback] = useState<TransferFeedback | null>(null);
  const [backupFeedback, setBackupFeedback] = useState<TransferFeedback | null>(null);
  const [importProgress, setImportProgress] = useState<AccountTransferImportProgress | null>(null);
  const [exportSelection, setExportSelection] = useState<DataTransferSelection>({
    ...DEFAULT_TRANSFER_SELECTION,
  });
  const [importSelection, setImportSelection] = useState<DataTransferSelection>({
    ...DEFAULT_TRANSFER_SELECTION,
  });
  const [backupSettings, setBackupSettings] = useState<AutoBackupSettings | null>(null);
  const [backupFiles, setBackupFiles] = useState<AutoBackupFileEntry[]>([]);
  const [backupUsage, setBackupUsage] = useState<BackupUsageSummary | null>(null);
  const [backupLoading, setBackupLoading] = useState(true);
  const [backupSaving, setBackupSaving] = useState(false);
  const [backupRunning, setBackupRunning] = useState(false);
  const [backupDeletingFile, setBackupDeletingFile] = useState<string | null>(null);
  const [backupImportingFile, setBackupImportingFile] = useState<string | null>(null);
  const [backupDownloadingFile, setBackupDownloadingFile] = useState<string | null>(null);
  const [backupDownloadingPlatform, setBackupDownloadingPlatform] = useState<string | null>(null);
  const [backupPlatformFilter, setBackupPlatformFilter] = useState<PlatformId | 'all'>('all');
  const [backupRetentionInput, setBackupRetentionInput] = useState('15');
  const [backupDirectorySaving, setBackupDirectorySaving] = useState(false);
  const [backupCleanupRunning, setBackupCleanupRunning] = useState(false);
  const [backupMigrationPreview, setBackupMigrationPreview] =
    useState<BackupDirectoryMigrationPreview | null>(null);
  const [backupMigrationProgress, setBackupMigrationProgress] =
    useState<BackupDirectoryMigrationProgress | null>(null);
  const [backupMigrationResult, setBackupMigrationResult] =
    useState<BackupDirectoryChangeResult | null>(null);
  const [backupMigrationError, setBackupMigrationError] = useState<string | null>(null);
  const [backupMigrationCanceling, setBackupMigrationCanceling] = useState(false);
  const [showBackupMigrationConfirm, setShowBackupMigrationConfirm] = useState(false);
  const [showBackupMigrationProgress, setShowBackupMigrationProgress] = useState(false);

  const setExportFailed = useCallback(
    (error: unknown) => {
      setFeedback({
        tone: 'error',
        text: t('messages.exportFailed', {
          error: normalizeError(error),
        }),
      });
    },
    [t],
  );

  const exportModal = useExportJsonModal({
    exportFilePrefix: 'cockpit_data_backup',
    exportJsonByIds: async () => exportDataTransferJson(exportSelection),
    onError: setExportFailed,
  });

  const toggleSelection = useCallback(
    (
      current: DataTransferSelection,
      setSelection: Dispatch<SetStateAction<DataTransferSelection>>,
      key: keyof DataTransferSelection,
    ) => {
      setSelection({
        ...current,
        [key]: !current[key],
      });
    },
    [],
  );

  const handleExport = useCallback(() => {
    setFeedback(null);
    setShowExportOptionsModal(true);
  }, []);

  const closeExportOptionsModal = useCallback(() => {
    if (exportModal.preparing) return;
    setShowExportOptionsModal(false);
  }, [exportModal.preparing]);

  const handleConfirmExport = useCallback(async () => {
    if (!hasSelection(exportSelection)) {
      setFeedback({
        tone: 'error',
        text: t('settings.transfer.errors.selectionRequired'),
      });
      return;
    }
    setFeedback(null);
    setShowExportOptionsModal(false);
    await exportModal.startExport(['data'], getDataTransferFileNameBase(exportSelection));
  }, [exportModal, exportSelection, t]);

  const closeImportModal = useCallback(() => {
    if (importing) return;
    setShowImportModal(false);
    setJsonInput('');
    setImportProgress(null);
    setFeedback(null);
  }, [importing]);

  const closeBackupManagerModal = useCallback(() => {
    if (
      backupSaving ||
      backupRunning ||
      backupImportingFile ||
      backupDeletingFile ||
      backupDirectorySaving
    ) return;
    setShowBackupManagerModal(false);
  }, [
    backupDeletingFile,
    backupDirectorySaving,
    backupImportingFile,
    backupRunning,
    backupSaving,
  ]);

  useEscClose(showExportOptionsModal, closeExportOptionsModal);
  useEscClose(showImportModal, closeImportModal);
  useEscClose(showBackupManagerModal, closeBackupManagerModal);

  const calcProgressPercent = useCallback((progress: AccountTransferImportProgress | null) => {
    if (!progress || progress.total_platforms <= 0) {
      return 0;
    }
    return Math.round((progress.completed_platforms / progress.total_platforms) * 100);
  }, []);

  const getDetailStatusText = useCallback(
    (detail: AccountTransferImportProgressDetail) => {
      if (detail.status === 'running') return t('common.shared.import.progress.statusRunning');
      if (detail.status === 'success') return t('common.success');
      if (detail.status === 'failed') return t('common.failed');
      if (detail.status === 'pending') return t('common.shared.import.progress.statusPending');
      if (detail.status === 'skipped') return t('common.shared.import.progress.statusSkipped');
      return '-';
    },
    [t],
  );

  const formatDetailLine = useCallback(
    (detail: AccountTransferImportProgressDetail) => {
      const platformLabel = getPlatformLabel(detail.platform, t);
      if (detail.status === 'running') {
        return `⏳ ${platformLabel} ${t('common.shared.import.progress.statusRunning')}`;
      }
      if (detail.status === 'failed') {
        const suffix = detail.error ? `（${detail.error}）` : '';
        return `❌ ${platformLabel} ${detail.imported_count}/${detail.expected_count}${suffix}`;
      }
      if (detail.status === 'success') {
        return `✅ ${platformLabel} ${detail.imported_count}/${detail.expected_count}`;
      }
      if (detail.status === 'pending') {
        return `🕒 ${platformLabel} ${t('common.shared.import.progress.statusPending')}`;
      }
      return `➖ ${platformLabel} ${t('common.shared.import.progress.statusSkipped')}`;
    },
    [t],
  );

  const buildImportFeedback = useCallback(
    (result: DataTransferImportResult): TransferFeedback => {
      const parts: string[] = [];
      const accountError = firstAccountImportError(result);

      if (result.imported_account_count > 0) {
        parts.push(
          t('settings.transfer.feedback.accountsImported', {
            count: result.imported_account_count,
          }),
        );
      }

      if (result.config_result?.applied) {
        parts.push(t('settings.transfer.feedback.configImported'));
      }

      if (result.warnings.includes('accounts_section_missing')) {
        parts.push(t('settings.transfer.feedback.accountsSectionMissing'));
      }

      if (result.warnings.includes('config_section_missing')) {
        parts.push(t('settings.transfer.feedback.configSectionMissing'));
      }

      if ((result.config_result?.unresolved_account_ref_count ?? 0) > 0) {
        parts.push(
          t('settings.transfer.feedback.unresolvedRefs', {
            count: result.config_result?.unresolved_account_ref_count ?? 0,
          }),
        );
      }

      if ((result.config_result?.disabled_task_count ?? 0) > 0) {
        parts.push(
          t('settings.transfer.feedback.disabledTasks', {
            count: result.config_result?.disabled_task_count ?? 0,
          }),
        );
      }

      if (result.config_result?.needs_restart) {
        parts.push(t('settings.transfer.feedback.restartRequired'));
      }

      if (accountError) {
        parts.push(
          t('settings.transfer.feedback.partialAccountFailure', {
            error: accountError,
          }),
        );
      }

      if (parts.length === 0) {
        parts.push(t('common.success'));
      }

      return {
        tone: accountError ? 'error' : 'success',
        text: parts.join('；'),
      };
    },
    [t],
  );

  const executeImportContent = useCallback(
    async (
      content: string,
      selection: DataTransferSelection,
      setTargetFeedback: FeedbackSetter,
      options: {
        clearJsonInput?: boolean;
        showProgress?: boolean;
      } = {},
    ) => {
      const trimmed = content.trim();
      if (!trimmed) {
        setTargetFeedback({
          tone: 'error',
          text: t('common.shared.token.empty'),
        });
        return false;
      }

      if (!hasSelection(selection)) {
        setTargetFeedback({
          tone: 'error',
          text: t('settings.transfer.errors.selectionRequired'),
        });
        return false;
      }

      const showProgress = options.showProgress !== false;

      setImporting(true);
      setImportProgress(null);
      setTargetFeedback({
        tone: 'loading',
        text: t('common.shared.import.importing'),
      });

      try {
        const result = await importDataTransferJson(trimmed, {
          ...selection,
          onAccountProgress: showProgress
            ? (progress) => {
                setImportProgress(progress);
              }
            : undefined,
        });

        const hasConfigImported = result.config_result?.applied === true;
        const hasAccountsImported = result.imported_account_count > 0;

        if (!hasAccountsImported && !hasConfigImported) {
          if ((result.account_result?.platform_failed_count ?? 0) > 0) {
            const firstError = firstAccountImportError(result) ?? t('common.failed');
            setTargetFeedback({
              tone: 'error',
              text: t('common.shared.import.failedMsg', {
                error: firstError,
              }),
            });
            return false;
          }

          setTargetFeedback({
            tone: 'error',
            text: t('modals.import.noAccountsFound'),
          });
          return false;
        }

        setTargetFeedback(buildImportFeedback(result));
        if (options.clearJsonInput) {
          setJsonInput('');
        }
        return true;
      } catch (error) {
        const rawError = normalizeError(error);
        setTargetFeedback({
          tone: 'error',
          text: t('common.shared.import.failedMsg', {
            error: normalizeImportErrorMessage(rawError, t('messages.jsonRequired'), t),
          }),
        });
        return false;
      } finally {
        setImporting(false);
        if (!showProgress) {
          setImportProgress(null);
        }
      }
    },
    [buildImportFeedback, t],
  );

  const handleImportContent = useCallback(
    async (content: string) => {
      await executeImportContent(content, importSelection, setFeedback, {
        clearJsonInput: true,
        showProgress: true,
      });
    },
    [executeImportContent, importSelection],
  );

  const handlePickImportFile = useCallback(() => {
    importFileInputRef.current?.click();
  }, []);

  const handleImportFileChange = useCallback(
    async (event: ChangeEvent<HTMLInputElement>) => {
      const file = event.target.files?.[0];
      event.target.value = '';
      if (!file) return;
      const content = await file.text();
      await handleImportContent(content);
    },
    [handleImportContent],
  );

  const renderSelectionControls = useCallback(
    (
      selection: DataTransferSelection,
      setSelection: Dispatch<SetStateAction<DataTransferSelection>>,
      disabled: boolean,
    ) => (
      <div className="settings-transfer-selection-group">
        <div className="settings-transfer-selection-title">{t('settings.transfer.selectionTitle')}</div>
        <label className="settings-transfer-selection-item">
          <input
            type="checkbox"
            checked={selection.includeAccounts}
            onChange={() => toggleSelection(selection, setSelection, 'includeAccounts')}
            disabled={disabled}
          />
          <div className="settings-transfer-selection-copy">
            <div className="settings-transfer-selection-name">
              {t('settings.transfer.accountsOptionTitle')}
            </div>
            <div className="settings-transfer-selection-desc">
              {t('settings.transfer.accountsOptionDesc')}
            </div>
          </div>
        </label>
        <label className="settings-transfer-selection-item">
          <input
            type="checkbox"
            checked={selection.includeConfig}
            onChange={() => toggleSelection(selection, setSelection, 'includeConfig')}
            disabled={disabled}
          />
          <div className="settings-transfer-selection-copy">
            <div className="settings-transfer-selection-name">
              {t('settings.transfer.configOptionTitle')}
            </div>
            <div className="settings-transfer-selection-desc">
              {t('settings.transfer.configOptionDesc')}
            </div>
          </div>
        </label>
      </div>
    ),
    [t, toggleSelection],
  );

  const loadAutoBackupState = useCallback(
    async (silent = false) => {
      if (!silent) {
        setBackupLoading(true);
      }
      try {
        const [settings, files, usage] = await Promise.all([
          getAutoBackupSettings(),
          listAutoBackupFiles(),
          getBackupUsage(),
        ]);
        setBackupSettings(settings);
        setBackupFiles(files);
        setBackupUsage(usage);
        setBackupRetentionInput(String(settings.retention_days));
      } catch (error) {
        setBackupFeedback({
          tone: 'error',
          text: t('settings.transfer.backup.feedback.loadFailed', {
            error: normalizeError(error),
          }),
        });
      } finally {
        if (!silent) {
          setBackupLoading(false);
        }
      }
    },
    [t],
  );

  useEffect(() => {
    void loadAutoBackupState();
  }, [loadAutoBackupState]);

  useEffect(() => {
    const handleStateChanged = () => {
      void loadAutoBackupState(true);
    };
    window.addEventListener(AUTO_BACKUP_STATE_CHANGED_EVENT, handleStateChanged);
    return () => {
      window.removeEventListener(AUTO_BACKUP_STATE_CHANGED_EVENT, handleStateChanged);
    };
  }, [loadAutoBackupState]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<BackupDirectoryMigrationProgress>(
      BACKUP_DIRECTORY_MIGRATION_PROGRESS_EVENT,
      (event) => {
        if (event.payload.migration_id !== backupMigrationIdRef.current) return;
        setBackupMigrationProgress(event.payload);
      },
    ).then((cleanup) => {
      if (disposed) {
        cleanup();
      } else {
        unlisten = cleanup;
      }
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const openBackupManagerModal = useCallback(() => {
    setBackupFeedback(null);
    setShowBackupManagerModal(true);
    void loadAutoBackupState();
  }, [loadAutoBackupState]);

  const persistAutoBackupSettings = useCallback(
    async (nextSettings: {
      enabled: boolean;
      selection: DataTransferSelection;
      retentionDays: number;
    }) => {
      setBackupSaving(true);
      try {
        const saved = await saveAutoBackupSettings(nextSettings);
        setBackupSettings(saved);
        setBackupRetentionInput(String(saved.retention_days));
        setBackupFeedback(null);
        const files = await listAutoBackupFiles();
        setBackupFiles(files);
      } catch (error) {
        setBackupFeedback({
          tone: 'error',
          text: t('settings.transfer.backup.feedback.saveSettingsFailed', {
            error: normalizeError(error),
          }),
        });
      } finally {
        setBackupSaving(false);
      }
    },
    [t],
  );

  const handleBackupEnabledChange = useCallback(
    async (enabled: boolean) => {
      if (!backupSettings) return;
      await persistAutoBackupSettings({
        enabled,
        selection: getSelectionFromAutoBackupSettings(backupSettings),
        retentionDays: backupSettings.retention_days,
      });
    },
    [backupSettings, persistAutoBackupSettings],
  );

  const handleBackupModeChange = useCallback(
    async (mode: AutoBackupMode) => {
      if (!backupSettings) return;
      await persistAutoBackupSettings({
        enabled: backupSettings.enabled,
        selection: autoBackupModeToSelection(mode),
        retentionDays: backupSettings.retention_days,
      });
    },
    [backupSettings, persistAutoBackupSettings],
  );

  const commitBackupRetentionDays = useCallback(
    async (rawValue: string) => {
      if (!backupSettings) return;
      const parsed = Number.parseInt(rawValue.trim(), 10);
      if (!Number.isFinite(parsed) || parsed < 1 || parsed > 365) {
        setBackupRetentionInput(String(backupSettings.retention_days));
        setBackupFeedback({
          tone: 'error',
          text: t('settings.transfer.backup.errors.invalidRetention'),
        });
        return;
      }

      await persistAutoBackupSettings({
        enabled: backupSettings.enabled,
        selection: getSelectionFromAutoBackupSettings(backupSettings),
        retentionDays: parsed,
      });

      try {
        const deletedFiles = await cleanupAutoBackupFiles(parsed);
        if (deletedFiles.length > 0) {
          setBackupFeedback({
            tone: 'success',
            text: t('settings.transfer.backup.feedback.cleanupDone', {
              count: deletedFiles.length,
            }),
          });
        }
        const files = await listAutoBackupFiles();
        setBackupFiles(files);
      } catch (error) {
        setBackupFeedback({
          tone: 'error',
          text: t('settings.transfer.backup.feedback.cleanupFailed', {
            error: normalizeError(error),
          }),
        });
      }
    },
    [backupSettings, persistAutoBackupSettings, t],
  );

  const handleRunBackupNow = useCallback(async () => {
    if (!backupSettings) return;
    const selection = getSelectionFromAutoBackupSettings(backupSettings);
    setBackupRunning(true);
    setBackupFeedback({
      tone: 'loading',
      text: t('settings.transfer.backup.feedback.running'),
    });
    const runBackup = async () => {
      const result = await createManagedBackup({
        trigger: 'manual',
        selection,
        retentionDays: backupSettings.retention_days,
        markAsLastRun: true,
      });
      await loadAutoBackupState(true);
      const feedbackKey =
        result.deleted_files.length > 0
          ? 'settings.transfer.backup.feedback.runSuccessWithCleanup'
          : 'settings.transfer.backup.feedback.runSuccess';
      setBackupFeedback({
        tone: 'success',
        text: t(feedbackKey, {
          path: result.path,
          count: result.deleted_files.length,
        }),
      });
    };
    try {
      await runBackup();
    } catch (error) {
      if (
        presentWindowsOperationError({
          error,
          operation: 'backup',
          retry: runBackup,
        })
      ) {
        setBackupFeedback(null);
        return;
      }
      setBackupFeedback({
        tone: 'error',
        text: t('settings.transfer.backup.feedback.runFailed', {
          error: normalizeError(error),
        }),
      });
    } finally {
      setBackupRunning(false);
    }
  }, [backupSettings, loadAutoBackupState, t]);

  const handleDeleteBackup = useCallback(
    async (fileName: string) => {
      const confirmed = window.confirm(
        t('settings.transfer.backup.deleteConfirm', {
          name: fileName,
        }),
      );
      if (!confirmed) return;
      setBackupDeletingFile(fileName);
      try {
        await deleteAutoBackupFile(fileName);
        await loadAutoBackupState(true);
        setBackupFeedback({
          tone: 'success',
          text: t('settings.transfer.backup.feedback.deleteSuccess'),
        });
      } catch (error) {
        if (
          presentWindowsOperationError({
            error,
            operation: 'delete_file',
            retry: async () => {
              await deleteAutoBackupFile(fileName);
              await loadAutoBackupState(true);
            },
          })
        ) {
          setBackupFeedback(null);
          return;
        }
        setBackupFeedback({
          tone: 'error',
          text: t('settings.transfer.backup.feedback.deleteFailed', {
            error: normalizeError(error),
          }),
        });
      } finally {
        setBackupDeletingFile(null);
      }
    },
    [loadAutoBackupState, t],
  );

  const handleImportBackup = useCallback(
    async (fileName: string) => {
      setBackupImportingFile(fileName);
      try {
        const content = await readAutoBackupFile(fileName);
        await executeImportContent(content, DEFAULT_TRANSFER_SELECTION, setBackupFeedback, {
          clearJsonInput: false,
          showProgress: false,
        });
      } catch (error) {
        setBackupFeedback({
          tone: 'error',
          text: t('common.shared.import.failedMsg', {
            error: normalizeError(error),
          }),
        });
      } finally {
        setBackupImportingFile(null);
      }
    },
    [executeImportContent, t],
  );

  const handleDownloadBackupJson = useCallback(
    async (fileName: string) => {
      setBackupDownloadingFile(`${fileName}:json`);
      try {
        const content = await readAutoBackupFile(fileName);
        const savedPath = await exportModal.saveJsonFile(content, buildBackupJsonFileName(fileName));
        if (savedPath) {
          setBackupFeedback({
            tone: 'success',
            text: t('settings.transfer.backup.feedback.downloadSuccess', {
              path: savedPath,
            }),
          });
        }
      } catch (error) {
        setBackupFeedback({
          tone: 'error',
          text: t('settings.transfer.backup.feedback.downloadFailed', {
            error: normalizeError(error),
          }),
        });
      } finally {
        setBackupDownloadingFile(null);
      }
    },
    [exportModal, t],
  );

  const handleDownloadBackupZip = useCallback(
    async (fileName: string) => {
      setBackupDownloadingFile(`${fileName}:zip`);
      try {
        const defaultPath = await exportModal.resolveDefaultExportPath(fileName);
        const targetPath = await save({
          defaultPath,
          filters: [{ name: 'ZIP', extensions: ['zip'] }],
        });
        if (!targetPath) return;
        const savedPath = await copyAutoBackupFile(fileName, targetPath);
        setBackupFeedback({
          tone: 'success',
          text: t('settings.transfer.backup.feedback.downloadSuccess', {
            path: savedPath,
          }),
        });
      } catch (error) {
        setBackupFeedback({
          tone: 'error',
          text: t('settings.transfer.backup.feedback.downloadFailed', {
            error: normalizeError(error),
          }),
        });
      } finally {
        setBackupDownloadingFile(null);
      }
    },
    [exportModal, t],
  );

  const handleDownloadBackupPlatform = useCallback(
    async (fileName: string, platform: PlatformId) => {
      setBackupDownloadingPlatform(`${fileName}:${platform}`);
      try {
        const content = await readAutoBackupFile(fileName);
        const platformJson = extractAutoBackupPlatformJson(content, platform);
        const savedPath = await exportModal.saveJsonFile(
          platformJson,
          buildPlatformBackupFileName(fileName, platform),
        );
        if (savedPath) {
          setBackupFeedback({
            tone: 'success',
            text: t('settings.transfer.backup.feedback.downloadSuccess', {
              path: savedPath,
            }),
          });
        }
      } catch (error) {
        setBackupFeedback({
          tone: 'error',
          text: t('settings.transfer.backup.feedback.downloadFailed', {
            error: normalizeError(error),
          }),
        });
      } finally {
        setBackupDownloadingPlatform(null);
      }
    },
    [exportModal, t],
  );

  const handleOpenBackupDir = useCallback(async () => {
    try {
      await openAutoBackupDir();
    } catch (error) {
      if (
        presentWindowsOperationError({
          error,
          operation: 'open_path',
          retry: openAutoBackupDir,
        })
      ) {
        setBackupFeedback(null);
        return;
      }
      setBackupFeedback({
        tone: 'error',
        text: t('settings.transfer.backup.feedback.openFolderFailed', {
          error: normalizeError(error),
        }),
      });
    }
  }, [t]);

  const handleChangeBackupDirectory = useCallback(async () => {
    if (!backupSettings || backupDirectorySaving) return;
    let selectedPath: string | null = null;
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        defaultPath: backupSettings.directory_path || undefined,
      });
      if (!selected || typeof selected !== 'string') return;
      selectedPath = selected;
      if (selected.trim() === backupSettings.directory_path.trim()) return;
      setBackupDirectorySaving(true);
      setBackupFeedback({
        tone: 'loading',
        text: t('settings.transfer.backup.feedback.scanningMigration'),
      });
      const preview = await previewBackupDirectoryChange(selected);
      setBackupMigrationPreview(preview);
      setBackupMigrationError(null);
      setBackupMigrationResult(null);
      setShowBackupMigrationConfirm(true);
      setBackupFeedback(null);
    } catch (error) {
      if (
        presentWindowsOperationError({
          error,
          operation: 'backup',
          retry: selectedPath
            ? async () => {
                const preview = await previewBackupDirectoryChange(selectedPath as string);
                setBackupMigrationPreview(preview);
                setBackupMigrationError(null);
                setBackupMigrationResult(null);
                setShowBackupMigrationConfirm(true);
                setBackupFeedback(null);
              }
            : undefined,
        })
      ) {
        setBackupFeedback(null);
        return;
      }
      setBackupFeedback({
        tone: 'error',
        text: t('settings.transfer.backup.feedback.directoryChangeFailed', {
          error: normalizeError(error),
        }),
      });
    } finally {
      setBackupDirectorySaving(false);
    }
  }, [backupDirectorySaving, backupSettings, loadAutoBackupState, t]);

  const startBackupDirectoryChange = useCallback(
    async (migrateExisting: boolean) => {
      if (!backupMigrationPreview || backupDirectorySaving) return;
      const migrationId = createBackupMigrationId();
      backupMigrationIdRef.current = migrationId;
      setShowBackupMigrationConfirm(false);
      setShowBackupMigrationProgress(true);
      setBackupMigrationResult(null);
      setBackupMigrationError(null);
      setBackupMigrationCanceling(false);
      setBackupDirectorySaving(true);
      setBackupMigrationProgress({
        migration_id: migrationId,
        phase: 'scanning',
        total_file_count: backupMigrationPreview.file_count,
        processed_file_count: 0,
        total_size_bytes: backupMigrationPreview.size_bytes,
        processed_size_bytes: 0,
        current_source: null,
        current_path: null,
        cancellable: migrateExisting,
      });
      try {
        const result = await changeBackupDirectory(
          backupMigrationPreview.new_directory,
          migrateExisting,
          migrationId,
        );
        setBackupMigrationResult(result);
        setBackupMigrationProgress((current) => ({
          migration_id: migrationId,
          phase: 'completed',
          total_file_count: current?.total_file_count ?? result.migrated_file_count,
          processed_file_count: result.migrated_file_count,
          total_size_bytes: current?.total_size_bytes ?? result.migrated_size_bytes,
          processed_size_bytes: result.migrated_size_bytes,
          current_source: null,
          current_path: null,
          cancellable: false,
        }));
        await loadAutoBackupState(true);
      } catch (error) {
        const message = normalizeError(error);
        if (message === 'backup_migration_cancelled') {
          setBackupMigrationProgress((current) => ({
            migration_id: migrationId,
            phase: 'cancelled',
            total_file_count: current?.total_file_count ?? backupMigrationPreview.file_count,
            processed_file_count: current?.processed_file_count ?? 0,
            total_size_bytes: current?.total_size_bytes ?? backupMigrationPreview.size_bytes,
            processed_size_bytes: current?.processed_size_bytes ?? 0,
            current_source: current?.current_source ?? null,
            current_path: current?.current_path ?? null,
            cancellable: false,
          }));
        } else {
          setBackupMigrationError(message);
        }
      } finally {
        backupMigrationIdRef.current = null;
        setBackupMigrationCanceling(false);
        setBackupDirectorySaving(false);
      }
    },
    [backupDirectorySaving, backupMigrationPreview, loadAutoBackupState],
  );

  const handleCancelBackupMigration = useCallback(async () => {
    const migrationId = backupMigrationIdRef.current;
    if (!migrationId || !backupMigrationProgress?.cancellable || backupMigrationCanceling) return;
    setBackupMigrationCanceling(true);
    try {
      await cancelBackupDirectoryChange(migrationId);
    } catch (error) {
      setBackupMigrationError(normalizeError(error));
      setBackupMigrationCanceling(false);
    }
  }, [backupMigrationCanceling, backupMigrationProgress?.cancellable]);

  const closeBackupMigrationProgress = useCallback(() => {
    if (backupDirectorySaving) return;
    setShowBackupMigrationProgress(false);
    setBackupMigrationProgress(null);
    setBackupMigrationResult(null);
    setBackupMigrationError(null);
    setBackupMigrationPreview(null);
  }, [backupDirectorySaving]);

  const handleCleanupBehaviorBackups = useCallback(async () => {
    if (backupCleanupRunning) return;
    const confirmed = window.confirm(t('settings.transfer.backup.behaviorCleanupConfirm'));
    if (!confirmed) return;
    setBackupCleanupRunning(true);
    setBackupFeedback({
      tone: 'loading',
      text: t('settings.transfer.backup.feedback.behaviorCleaning'),
    });
    try {
      const result = await cleanupBehaviorBackups();
      await loadAutoBackupState(true);
      setBackupFeedback({
        tone: 'success',
        text: t('settings.transfer.backup.feedback.behaviorCleanupDone', {
          count: result.deleted_directory_count,
          size: formatFileSize(result.deleted_size_bytes),
        }),
      });
    } catch (error) {
      setBackupFeedback({
        tone: 'error',
        text: t('settings.transfer.backup.feedback.behaviorCleanupFailed', {
          error: normalizeError(error),
        }),
      });
    } finally {
      setBackupCleanupRunning(false);
    }
  }, [backupCleanupRunning, loadAutoBackupState, t]);

  const backupSelection = backupSettings
    ? getSelectionFromAutoBackupSettings(backupSettings)
    : DEFAULT_TRANSFER_SELECTION;
  const backupMode = selectionToAutoBackupMode(backupSelection);
  const backupControlsDisabled =
    backupSaving ||
    backupRunning ||
    backupImportingFile !== null ||
    backupDeletingFile !== null ||
    backupDownloadingFile !== null ||
    backupDownloadingPlatform !== null ||
    backupDirectorySaving ||
    backupCleanupRunning;
  const backupPlatformOptions = useMemo(() => {
    const present = new Set<PlatformId>();
    for (const file of backupFiles) {
      for (const item of normalizeAutoBackupPlatforms(file.platforms)) {
        present.add(item.platform);
      }
    }
    return ALL_PLATFORM_IDS.filter((platform) => present.has(platform));
  }, [backupFiles]);
  const visibleBackupFiles = useMemo(() => {
    if (backupPlatformFilter === 'all') {
      return backupFiles;
    }
    return backupFiles.filter((file) =>
      normalizeAutoBackupPlatforms(file.platforms).some(
        (item) => item.platform === backupPlatformFilter,
      ),
    );
  }, [backupFiles, backupPlatformFilter]);
  const backupUsageEntries = useMemo(() => {
    if (!backupUsage) return [];
    const entriesBySource = new Map(backupUsage.entries.map((entry) => [entry.source, entry]));
    const knownEntries = BACKUP_USAGE_SOURCE_ORDER.map(
      (source): BackupUsageSummary['entries'][number] =>
        entriesBySource.get(source) ?? {
          source,
          file_count: 0,
          size_bytes: 0,
          path: '',
        },
    );
    const knownSources = new Set<string>(BACKUP_USAGE_SOURCE_ORDER);
    return [
      ...knownEntries,
      ...backupUsage.entries.filter((entry) => !knownSources.has(entry.source)),
    ];
  }, [backupUsage]);
  const migrationPercent = backupMigrationPercent(backupMigrationProgress);
  const migrationPhaseLabel = backupMigrationProgress
    ? t(`settings.transfer.backup.migrationProgress.phases.${backupMigrationProgress.phase}`)
    : t('settings.transfer.backup.migrationProgress.phases.scanning');

  useEffect(() => {
    if (backupPlatformFilter === 'all') return;
    if (backupPlatformOptions.includes(backupPlatformFilter)) return;
    setBackupPlatformFilter('all');
  }, [backupPlatformFilter, backupPlatformOptions]);

  const formatBackupTime = useCallback((value: string | number | null | undefined) => {
    if (value == null) {
      return t('settings.transfer.backup.lastRunNever');
    }
    const date = new Date(value);
    if (!Number.isFinite(date.getTime())) {
      return t('settings.transfer.backup.lastRunNever');
    }
    return new Intl.DateTimeFormat(undefined, {
      year: 'numeric',
      month: '2-digit',
      day: '2-digit',
      hour: '2-digit',
      minute: '2-digit',
    }).format(date);
  }, [t]);

  const getBackupModeLabel = useCallback(
    (mode: AutoBackupMode | null) => {
      if (mode === 'accounts') return t('settings.transfer.backup.typeAccounts');
      if (mode === 'config') return t('settings.transfer.backup.typeConfig');
      return t('settings.transfer.backup.typeFull');
    },
    [t],
  );

  const getBackupSourceLabel = useCallback(
    (trigger: 'auto' | 'manual' | null) =>
      trigger === 'auto'
        ? t('settings.transfer.backup.sourceAuto')
        : t('settings.transfer.backup.sourceManual'),
    [t],
  );

  const feedbackNode = feedback ? (
    <div className={`add-feedback ${feedback.tone}`}>{feedback.text}</div>
  ) : null;
  const backupFeedbackNode = backupFeedback ? (
    <div className={`add-feedback ${backupFeedback.tone}`}>{backupFeedback.text}</div>
  ) : null;
  const progressPercent = calcProgressPercent(importProgress);
  const currentPlatformDetail =
    importProgress && importProgress.current_platform
      ? importProgress.details.find((item) => item.platform === importProgress.current_platform) ?? null
      : null;
  const visibleProgressDetails = importProgress
    ? importProgress.details.filter(
        (detail) =>
          detail.expected_count > 0 ||
          detail.imported_count > 0 ||
          detail.status === 'running' ||
          detail.status === 'failed',
      )
    : [];
  const currentImportPlatform = importProgress?.current_platform ?? null;

  useEffect(() => {
    if (!importProgress) return;
    const modalBody = modalBodyRef.current;
    if (!modalBody) return;
    modalBody.scrollTop = modalBody.scrollHeight;
  }, [importProgress]);

  useEffect(() => {
    if (!importProgress) return;
    if (!currentImportPlatform) return;
    const list = progressListRef.current;
    if (!list) return;
    const target = list.querySelector<HTMLElement>(`[data-platform="${currentImportPlatform}"]`);
    if (!target) return;
    target.scrollIntoView({ block: 'nearest' });
  }, [currentImportPlatform, importProgress]);

  return (
    <>
      <div className="group-title">
        {directoryEntryOnly
          ? t('settings.transfer.backup.directoryTitle')
          : t('settings.general.accountManagement')}
      </div>
      <div className="settings-group">
        <div className="settings-row settings-row--align-start">
          <div className="row-label">
            <div className="row-title">
              {directoryEntryOnly
                ? t('settings.transfer.backup.directoryTitle')
                : t('settings.transfer.backup.title')}
            </div>
            <div className="row-desc">
              {directoryEntryOnly
                ? t('settings.transfer.backup.directoryDesc')
                : t('settings.transfer.backup.desc')}
            </div>
            <div className="settings-backup-inline-meta">
              {directoryEntryOnly
                ? backupSettings?.directory_path ?? t('common.loading')
                : backupLoading && !backupSettings
                ? t('common.loading')
                : `${backupSettings?.enabled
                    ? t('settings.transfer.backup.statusEnabled')
                    : t('settings.transfer.backup.statusDisabled')} · ${t(
                    'settings.transfer.backup.lastRunLabel',
                    {
                      time: formatBackupTime(backupSettings?.last_backup_at),
                    },
                  )}`}
            </div>
          </div>
          <div className="row-control">
            <button
              className="btn btn-secondary"
              onClick={() => {
                if (directoryEntryOnly) {
                  void handleChangeBackupDirectory();
                } else {
                  openBackupManagerModal();
                }
              }}
              disabled={
                backupDirectorySaving ||
                backupRunning ||
                backupSaving ||
                backupImportingFile !== null ||
                backupDeletingFile !== null
              }
            >
              {backupLoading || backupDirectorySaving ? (
                <RefreshCw size={16} className="loading-spinner" />
              ) : (
                <FolderOpen size={16} />
              )}
              {directoryEntryOnly
                ? t('settings.transfer.backup.changeDirectory')
                : t('common.open')}
            </button>
          </div>
        </div>

        {!showBackupManagerModal && backupFeedbackNode && (
          <div className="settings-transfer-feedback-wrap">{backupFeedbackNode}</div>
        )}

        {!directoryEntryOnly && (
          <>
            <div className="settings-row">
              <div className="row-label">
                <div className="row-title">{t('settings.transfer.exportTitle')}</div>
                <div className="row-desc">{t('settings.transfer.exportDesc')}</div>
              </div>
              <div className="row-control">
                <button
                  className="btn btn-secondary"
                  onClick={handleExport}
                  disabled={exportModal.preparing}
                >
                  {exportModal.preparing ? <RefreshCw size={16} className="loading-spinner" /> : <Download size={16} />}
                  {t('common.shared.export.title')}
                </button>
              </div>
            </div>

            <div className="settings-row">
              <div className="row-label">
                <div className="row-title">{t('settings.transfer.importTitle')}</div>
                <div className="row-desc">{t('settings.transfer.importDesc')}</div>
              </div>
              <div className="row-control">
                <button
                  className="btn btn-secondary"
                  onClick={() => {
                    setFeedback(null);
                    setShowImportModal(true);
                  }}
                  disabled={importing}
                >
                  {importing ? <RefreshCw size={16} className="loading-spinner" /> : <FolderOpen size={16} />}
                  {t('common.shared.import.label')}
                </button>
              </div>
            </div>

            {!showImportModal && !showExportOptionsModal && feedbackNode && (
              <div className="settings-transfer-feedback-wrap">{feedbackNode}</div>
            )}
          </>
        )}
      </div>

      {showBackupMigrationConfirm && backupMigrationPreview &&
        renderToBody(
          <div className="modal-overlay">
            <div
              className="modal settings-transfer-modal settings-backup-migration-modal"
              role="dialog"
              aria-modal="true"
              aria-labelledby="backup-migration-confirm-title"
            >
              <div className="modal-header">
                <h2 id="backup-migration-confirm-title">
                  {t('settings.transfer.backup.migrationConfirmTitle')}
                </h2>
                <button
                  className="modal-close"
                  onClick={() => {
                    setShowBackupMigrationConfirm(false);
                    setBackupMigrationPreview(null);
                  }}
                  aria-label={t('common.close')}
                >
                  <X />
                </button>
              </div>
              <div className="modal-body settings-transfer-modal-body">
                <p className="settings-transfer-modal-desc">
                  {t('settings.transfer.backup.migrationConfirmDesc')}
                </p>
                <div className="settings-backup-migration-paths">
                  <div>
                    <span>{t('settings.transfer.backup.migrationOldPath')}</span>
                    <code>{backupMigrationPreview.old_directory}</code>
                  </div>
                  <div>
                    <span>{t('settings.transfer.backup.migrationNewPath')}</span>
                    <code>{backupMigrationPreview.new_directory}</code>
                  </div>
                </div>
                <div className="settings-backup-migration-summary">
                  <div>
                    <strong>{formatFileSize(backupMigrationPreview.size_bytes)}</strong>
                    <span>{t('settings.transfer.backup.migrationTotalSize')}</span>
                  </div>
                  <div>
                    <strong>{backupMigrationPreview.file_count}</strong>
                    <span>{t('settings.transfer.backup.migrationTotalFiles')}</span>
                  </div>
                </div>
                {backupMigrationPreview.sources.length > 0 && (
                  <div className="settings-backup-migration-sources">
                    {backupMigrationPreview.sources.map((source) => (
                      <div key={source.source}>
                        <span>
                          {t(`settings.transfer.backup.sources.${source.source}`, {
                            defaultValue: source.source,
                          })}
                        </span>
                        <span>{formatFileSize(source.size_bytes)} · {source.file_count}</span>
                      </div>
                    ))}
                  </div>
                )}
                <div className="settings-backup-migration-note">
                  {t('settings.transfer.backup.migrationSafetyNote')}
                </div>
                <div className="settings-transfer-modal-actions settings-backup-migration-actions">
                  <button
                    className="btn btn-secondary"
                    onClick={() => {
                      setShowBackupMigrationConfirm(false);
                      setBackupMigrationPreview(null);
                    }}
                  >
                    {t('common.cancel')}
                  </button>
                  <button
                    className="btn btn-secondary"
                    onClick={() => void startBackupDirectoryChange(false)}
                  >
                    {t('settings.transfer.backup.switchOnly')}
                  </button>
                  <button
                    className="btn btn-primary"
                    onClick={() => void startBackupDirectoryChange(true)}
                  >
                    {t('settings.transfer.backup.migrateAndSwitch')}
                  </button>
                </div>
              </div>
            </div>
          </div>,
        )}

      {showBackupMigrationProgress && backupMigrationProgress &&
        renderToBody(
          <div className="modal-overlay">
            <div
              className="modal settings-transfer-modal settings-backup-migration-modal"
              role="dialog"
              aria-modal="true"
              aria-labelledby="backup-migration-progress-title"
            >
              <div className="modal-header">
                <h2 id="backup-migration-progress-title">
                  {t('settings.transfer.backup.migrationProgress.title')}
                </h2>
                {!backupDirectorySaving && (
                  <button
                    className="modal-close"
                    onClick={closeBackupMigrationProgress}
                    aria-label={t('common.close')}
                  >
                    <X />
                  </button>
                )}
              </div>
              <div className="modal-body settings-transfer-modal-body">
                <div className="settings-backup-migration-phase">{migrationPhaseLabel}</div>
                <div
                  className="settings-backup-migration-progress-track"
                  role="progressbar"
                  aria-valuemin={0}
                  aria-valuemax={100}
                  aria-valuenow={migrationPercent}
                >
                  <div
                    className="settings-backup-migration-progress-fill"
                    style={{ width: `${migrationPercent}%` }}
                  />
                </div>
                <div className="settings-backup-migration-progress-meta">
                  <strong>{migrationPercent}%</strong>
                  <span>
                    {formatFileSize(backupMigrationProgress.processed_size_bytes)} /{' '}
                    {formatFileSize(backupMigrationProgress.total_size_bytes)}
                  </span>
                  <span>
                    {backupMigrationProgress.processed_file_count} /{' '}
                    {backupMigrationProgress.total_file_count}{' '}
                    {t('settings.transfer.backup.migrationProgress.filesUnit')}
                  </span>
                </div>
                {backupMigrationProgress.current_source && (
                  <div className="settings-backup-migration-current">
                    <span>{t('settings.transfer.backup.migrationProgress.currentSource')}</span>
                    <strong>
                      {t(
                        `settings.transfer.backup.sources.${backupMigrationProgress.current_source}`,
                        { defaultValue: backupMigrationProgress.current_source },
                      )}
                    </strong>
                  </div>
                )}
                {backupMigrationProgress.current_path && (
                  <div className="settings-backup-migration-current-path">
                    {backupMigrationProgress.current_path}
                  </div>
                )}
                {backupMigrationError && (
                  <div className="form-error settings-backup-migration-error" role="alert">
                    {t('settings.transfer.backup.feedback.directoryChangeFailed', {
                      error: backupMigrationError,
                    })}
                  </div>
                )}
                {backupMigrationResult && (
                  <div className="settings-backup-migration-result">
                    <div>
                      {backupMigrationResult.migrated
                        ? t('settings.transfer.backup.migrationProgress.completedSummary', {
                            size: formatFileSize(backupMigrationResult.migrated_size_bytes),
                            count: backupMigrationResult.migrated_file_count,
                          })
                        : t('settings.transfer.backup.migrationProgress.switchOnlyCompleted', {
                            path: backupMigrationResult.new_directory,
                          })}
                    </div>
                    {backupMigrationResult.remaining_paths.length > 0 && (
                      <div className="settings-backup-migration-warning">
                        <strong>
                          {t('settings.transfer.backup.migrationProgress.cleanupPartial')}
                        </strong>
                        {backupMigrationResult.remaining_paths.map((path) => (
                          <code key={path}>{path}</code>
                        ))}
                      </div>
                    )}
                  </div>
                )}
                {backupMigrationProgress.phase === 'cancelled' && (
                  <div className="settings-backup-migration-result">
                    {t('settings.transfer.backup.migrationProgress.cancelledDesc')}
                  </div>
                )}
                <div className="settings-transfer-modal-actions">
                  {backupDirectorySaving && backupMigrationProgress.cancellable ? (
                    <button
                      className="btn btn-secondary"
                      onClick={() => void handleCancelBackupMigration()}
                      disabled={backupMigrationCanceling}
                    >
                      {backupMigrationCanceling && (
                        <RefreshCw size={14} className="loading-spinner" />
                      )}
                      {backupMigrationCanceling
                        ? t('settings.transfer.backup.migrationProgress.cancelling')
                        : t('settings.transfer.backup.migrationProgress.cancelAction')}
                    </button>
                  ) : backupDirectorySaving ? (
                    <button className="btn btn-secondary" disabled>
                      <RefreshCw size={14} className="loading-spinner" />
                      {t('settings.transfer.backup.migrationProgress.finishing')}
                    </button>
                  ) : (
                    <button className="btn btn-primary" onClick={closeBackupMigrationProgress}>
                      {t('common.close')}
                    </button>
                  )}
                </div>
              </div>
            </div>
          </div>,
        )}

      {showExportOptionsModal &&
        renderToBody(
          <div className="modal-overlay">
            <div className="modal settings-transfer-modal" onClick={(event) => event.stopPropagation()}>
              <div className="modal-header">
                <button className="btn btn-secondary icon-only" onClick={closeExportOptionsModal} title={t('common.back', '返回')} aria-label={t('common.back', '返回')}><ChevronLeft size={14} /></button>
                <h2>{t('settings.transfer.exportTitle')}</h2>
                <button
                  className="modal-close"
                  onClick={closeExportOptionsModal}
                  aria-label={t('common.close')}
                >
                  <X />
                </button>
              </div>

              <div className="modal-body settings-transfer-modal-body">
                <p className="settings-transfer-modal-desc">
                  {t('settings.transfer.exportModalDesc')}
                </p>

                {renderSelectionControls(exportSelection, setExportSelection, exportModal.preparing)}

                <div className="settings-transfer-modal-actions">
                  <button
                    className="btn btn-secondary"
                    onClick={closeExportOptionsModal}
                    disabled={exportModal.preparing}
                  >
                    {t('common.cancel')}
                  </button>
                  <button
                    className="btn btn-primary"
                    onClick={() => {
                      void handleConfirmExport();
                    }}
                    disabled={exportModal.preparing || !hasSelection(exportSelection)}
                  >
                    {exportModal.preparing ? (
                      <RefreshCw size={16} className="loading-spinner" />
                    ) : (
                      <Download size={16} />
                    )}
                    {t('settings.transfer.exportConfirm')}
                  </button>
                </div>
              </div>
            </div>
          </div>,
        )}

      {showImportModal &&
        renderToBody(
          <div className="modal-overlay">
            <div className="modal settings-transfer-modal" onClick={(event) => event.stopPropagation()}>
              <div className="modal-header">
                <button className="btn btn-secondary icon-only" onClick={closeImportModal} title={t('common.back', '返回')} aria-label={t('common.back', '返回')}><ChevronLeft size={14} /></button>
                <h2>{t('settings.transfer.importTitle')}</h2>
                <button className="modal-close" onClick={closeImportModal} aria-label={t('common.close')}>
                  <X />
                </button>
              </div>

              <div ref={modalBodyRef} className="modal-body settings-transfer-modal-body">
                <p className="settings-transfer-modal-desc">
                  {t('settings.transfer.importModalDesc')}
                </p>

                {renderSelectionControls(importSelection, setImportSelection, importing)}

                <div className="settings-transfer-import-block">
                  <div className="settings-transfer-import-title">
                    {t('settings.transfer.fromFileTitle')}
                  </div>
                  <div className="settings-transfer-import-desc">
                    {t('settings.transfer.fromFileDesc')}
                  </div>
                  <button
                    className="btn btn-secondary"
                    onClick={handlePickImportFile}
                    disabled={importing || !hasSelection(importSelection)}
                  >
                    {importing ? <RefreshCw size={16} className="loading-spinner" /> : <FolderOpen size={16} />}
                    {t('common.shared.import.pickFile')}
                  </button>
                  <input
                    ref={importFileInputRef}
                    type="file"
                    accept="application/json,.json"
                    style={{ display: 'none' }}
                    onChange={(event) => {
                      void handleImportFileChange(event);
                    }}
                  />
                </div>

                <div className="settings-transfer-json-title">
                  {t('modals.import.orJson')}
                </div>
                <textarea
                  className="export-json-textarea settings-transfer-json-input"
                  spellCheck={false}
                  value={jsonInput}
                  onChange={(event) => setJsonInput(event.target.value)}
                  placeholder={t('settings.transfer.jsonPlaceholder')}
                />

                <div className="settings-transfer-modal-actions">
                  <button className="btn btn-secondary" onClick={closeImportModal} disabled={importing}>
                    {t('common.cancel')}
                  </button>
                  <button
                    className="btn btn-primary"
                    onClick={() => {
                      void handleImportContent(jsonInput);
                    }}
                    disabled={importing || !jsonInput.trim() || !hasSelection(importSelection)}
                  >
                    {importing ? <RefreshCw size={16} className="loading-spinner" /> : <Download size={16} />}
                    {t('modals.import.importBtn')}
                  </button>
                </div>

                {importProgress && (
                  <div className="settings-transfer-progress-wrap">
                    <div className="settings-transfer-progress-bar">
                      <div
                        className="settings-transfer-progress-fill"
                        style={{ width: `${progressPercent}%` }}
                      />
                    </div>
                    <div className="settings-transfer-progress-main">
                      {t('common.shared.import.progress.overallLine', {
                        percent: progressPercent,
                        completedPlatforms: importProgress.completed_platforms,
                        totalPlatforms: importProgress.total_platforms,
                        processedAccounts: importProgress.processed_accounts,
                        totalAccounts: importProgress.total_accounts,
                      })}
                    </div>

                    {importProgress.current_platform && currentPlatformDetail && (
                      <div className="settings-transfer-current-platform">
                        <div className="settings-transfer-current-line">
                          {t('common.shared.import.progress.currentPlatformLine', {
                            platform: getPlatformLabel(importProgress.current_platform, t),
                            count: currentPlatformDetail.expected_count,
                          })}
                        </div>
                        <div className="settings-transfer-current-line">
                          {t('common.shared.import.progress.currentStatusLine', {
                            status:
                              currentPlatformDetail.status === 'running'
                                ? t('common.shared.import.progress.statusProcessing')
                                : getDetailStatusText(currentPlatformDetail),
                          })}
                        </div>
                      </div>
                    )}

                    <div className="settings-transfer-progress-title">
                      {t('common.shared.import.progress.platformDetailsTitle')}
                    </div>
                    <div ref={progressListRef} className="settings-transfer-progress-list">
                      {visibleProgressDetails.map((detail) => (
                        <div
                          key={detail.platform}
                          data-platform={detail.platform}
                          className={`settings-transfer-progress-item settings-transfer-progress-item--${detail.status}`}
                        >
                          <div className="settings-transfer-progress-item-line">
                            {formatDetailLine(detail)}
                          </div>
                        </div>
                      ))}
                    </div>
                  </div>
                )}

                {feedbackNode}
              </div>
            </div>
          </div>,
        )}

      {showBackupManagerModal &&
        renderToBody(
          <div className="modal-overlay">
            <div
              className="modal settings-transfer-modal settings-backup-modal"
              onClick={(event) => event.stopPropagation()}
            >
              <div className="modal-header">
                <button className="btn btn-secondary icon-only" onClick={closeBackupManagerModal} title={t('common.back', '返回')} aria-label={t('common.back', '返回')}><ChevronLeft size={14} /></button>
                <h2>{t('settings.transfer.backup.title')}</h2>
                <button className="modal-close" onClick={closeBackupManagerModal} aria-label={t('common.close')}>
                  <X />
                </button>
              </div>

              <div className="modal-body settings-transfer-modal-body settings-backup-modal-body">
                <p className="settings-transfer-modal-desc">
                  {t('settings.transfer.backup.modalDesc')}
                </p>

                <div className="settings-backup-manager-card">
                  <div className="settings-backup-manager-head">
                    <div className="settings-backup-manager-copy">
                      <div className="settings-backup-manager-title">
                        {t('settings.transfer.backup.autoTitle')}
                      </div>
                      <div className="settings-backup-manager-desc">
                        {t('settings.transfer.backup.autoDesc')}
                      </div>
                    </div>
                    <div className="settings-backup-primary-actions">
                      <label className="switch" aria-label={t('settings.transfer.backup.autoTitle')}>
                        <input
                          type="checkbox"
                          checked={backupSettings?.enabled ?? true}
                          onChange={(event) => {
                            void handleBackupEnabledChange(event.target.checked);
                          }}
                          disabled={backupControlsDisabled || !backupSettings}
                        />
                        <span className="slider" />
                      </label>
                      <button
                        className="btn btn-secondary"
                        onClick={() => {
                          void handleRunBackupNow();
                        }}
                        disabled={backupControlsDisabled || !backupSettings}
                      >
                        {backupRunning ? (
                          <RefreshCw size={16} className="loading-spinner" />
                        ) : (
                          <Download size={16} />
                        )}
                        {backupRunning
                          ? t('settings.transfer.backup.running')
                          : t('settings.transfer.backup.runNow')}
                      </button>
                    </div>
                  </div>
                  <div className="settings-backup-inline-meta">
                    {backupLoading && !backupSettings
                      ? t('common.loading')
                      : `${backupSettings?.enabled
                          ? t('settings.transfer.backup.statusEnabled')
                          : t('settings.transfer.backup.statusDisabled')} · ${t(
                          'settings.transfer.backup.lastRunLabel',
                          {
                            time: formatBackupTime(backupSettings?.last_backup_at),
                          },
                        )}`}
                  </div>
                </div>

                <div className="settings-backup-manager-grid">
                  <div className="settings-backup-manager-card">
                    <div className="settings-backup-manager-title">
                      {t('settings.transfer.backup.typeTitle')}
                    </div>
                    <div className="settings-backup-manager-desc">
                      {t('settings.transfer.backup.typeDesc')}
                    </div>
                    <select
                      className="settings-select"
                      value={backupMode}
                      onChange={(event) => {
                        void handleBackupModeChange(event.target.value as AutoBackupMode);
                      }}
                      disabled={backupControlsDisabled || !backupSettings}
                    >
                      <option value="full">{t('settings.transfer.backup.typeFull')}</option>
                      <option value="accounts">{t('settings.transfer.backup.typeAccounts')}</option>
                      <option value="config">{t('settings.transfer.backup.typeConfig')}</option>
                    </select>
                  </div>

                  <div className="settings-backup-manager-card">
                    <div className="settings-backup-manager-title">
                      {t('settings.transfer.backup.retentionTitle')}
                    </div>
                    <div className="settings-backup-manager-desc">
                      {t('settings.transfer.backup.retentionDesc')}
                    </div>
                    <div className="settings-inline-input settings-backup-retention-input">
                      <input
                        type="number"
                        min={1}
                        max={365}
                        className="settings-select settings-select--input-mode settings-select--with-unit"
                        value={backupRetentionInput}
                        onChange={(event) =>
                          setBackupRetentionInput(event.target.value.replace(/[^\d]/g, ''))
                        }
                        onBlur={() => {
                          void commitBackupRetentionDays(backupRetentionInput);
                        }}
                        onKeyDown={(event) => {
                          if (event.key === 'Enter') {
                            event.preventDefault();
                            void commitBackupRetentionDays(backupRetentionInput);
                          }
                        }}
                        disabled={backupControlsDisabled || !backupSettings}
                      />
                      <span className="settings-input-unit">
                        {t('settings.transfer.backup.retentionUnit')}
                      </span>
                    </div>
                  </div>
                </div>

                <div className="settings-backup-manager-card">
                  <div className="settings-backup-manager-head">
                    <div className="settings-backup-manager-copy">
                      <div className="settings-backup-manager-title">
                        {t('settings.transfer.backup.directoryTitle')}
                      </div>
                      <div className="settings-backup-manager-desc">
                        {t('settings.transfer.backup.directoryDesc')}
                      </div>
                    </div>
                    <button
                      className="btn btn-secondary"
                      onClick={() => {
                        void handleChangeBackupDirectory();
                      }}
                      disabled={backupControlsDisabled}
                    >
                      <FolderOpen size={16} />
                      {backupDirectorySaving
                        ? t('settings.transfer.backup.migrating')
                        : t('settings.transfer.backup.changeDirectory')}
                    </button>
                    <button
                      className="btn btn-secondary"
                      onClick={() => {
                        void handleOpenBackupDir();
                      }}
                      disabled={backupControlsDisabled}
                    >
                      <FolderOpen size={16} />
                      {t('settings.transfer.backup.openFolder')}
                    </button>
                  </div>
                  <div className="settings-backup-path">{backupSettings?.directory_path ?? '-'}</div>
                </div>

                <div className="settings-backup-manager-card">
                  <div className="settings-backup-manager-head">
                    <div className="settings-backup-manager-copy">
                      <div className="settings-backup-manager-title">
                        {t('settings.transfer.backup.usageTitle')}
                      </div>
                      <div className="settings-backup-manager-desc">
                        {t('settings.transfer.backup.usageDesc')}
                      </div>
                    </div>
                    <div className="settings-backup-usage-total">
                      {backupUsage
                        ? `${formatFileSize(backupUsage.total_size_bytes)} · ${backupUsage.total_file_count}`
                        : t('common.loading')}
                    </div>
                  </div>
                  {backupUsage && backupUsage.total_file_count > 0 ? (
                    <div className="settings-backup-usage-list">
                      {backupUsageEntries.map((entry) => (
                        <div className="settings-backup-usage-row" key={`${entry.source}:${entry.path}`}>
                          <span>{t(`settings.transfer.backup.sources.${entry.source}`, {
                            defaultValue: entry.source,
                          })}</span>
                          <span>{formatFileSize(entry.size_bytes)} · {entry.file_count}</span>
                        </div>
                      ))}
                    </div>
                  ) : (
                    <div className="settings-backup-empty-desc">
                      {t('settings.transfer.backup.usageEmpty')}
                    </div>
                  )}
                  <div className="settings-backup-item-actions">
                    <button
                      className="btn btn-secondary btn-sm"
                      onClick={() => {
                        void handleCleanupBehaviorBackups();
                      }}
                      disabled={backupControlsDisabled}
                    >
                      {backupCleanupRunning ? (
                        <RefreshCw size={14} className="loading-spinner" />
                      ) : (
                        <Trash2 size={14} />
                      )}
                      {t('settings.transfer.backup.cleanupBehavior')}
                    </button>
                  </div>
                </div>

                <div className="settings-backup-manager-card">
                  <div className="settings-backup-manager-title">
                    {t('settings.transfer.backup.listTitle')}
                  </div>
                  <div className="settings-backup-manager-desc">
                    {t('settings.transfer.backup.listDesc')}
                  </div>

                  {backupPlatformOptions.length > 0 && (
                    <div className="settings-backup-platform-filter">
                      <span>{t('settings.transfer.backup.platformFilterLabel')}</span>
                      <select
                        className="settings-select"
                        value={backupPlatformFilter}
                        onChange={(event) =>
                          setBackupPlatformFilter(event.target.value as PlatformId | 'all')
                        }
                        disabled={backupControlsDisabled}
                      >
                        <option value="all">{t('settings.transfer.backup.platformFilterAll')}</option>
                        {backupPlatformOptions.map((platform) => (
                          <option key={platform} value={platform}>
                            {getPlatformLabel(platform, t)}
                          </option>
                        ))}
                      </select>
                    </div>
                  )}

                  <div className="settings-backup-list-wrap">
                    {backupLoading ? (
                      <div className="settings-backup-empty">
                        <RefreshCw size={16} className="loading-spinner" />
                        <span>{t('common.loading')}</span>
                      </div>
                    ) : backupFiles.length === 0 ? (
                      <div className="settings-backup-empty">
                        <div className="settings-backup-empty-title">
                          {t('settings.transfer.backup.emptyTitle')}
                        </div>
                        <div className="settings-backup-empty-desc">
                          {t('settings.transfer.backup.emptyDesc')}
                        </div>
                      </div>
                    ) : visibleBackupFiles.length === 0 ? (
                      <div className="settings-backup-empty">
                        <div className="settings-backup-empty-title">
                          {t('settings.transfer.backup.platformEmptyTitle')}
                        </div>
                        <div className="settings-backup-empty-desc">
                          {t('settings.transfer.backup.platformEmptyDesc')}
                        </div>
                      </div>
                    ) : (
                      <div className="settings-backup-list">
                        {visibleBackupFiles.map((file) => {
                          const meta = parseManagedBackupMeta(file.file_name);
                          const isDeleting = backupDeletingFile === file.file_name;
                          const isImportingBackup = backupImportingFile === file.file_name;
                          const platformEntries = normalizeAutoBackupPlatforms(file.platforms);
                          const archiveFileName = file.archive_file_name ?? (
                            file.file_kind === 'zip' ? file.file_name : null
                          );
                          const isDownloadingJson = backupDownloadingFile === `${file.file_name}:json`;
                          const isDownloadingZip = archiveFileName
                            ? backupDownloadingFile === `${archiveFileName}:zip`
                            : false;
                          return (
                            <div key={file.file_name} className="settings-backup-item">
                              <div className="settings-backup-item-head">
                                <div className="settings-backup-item-name">{file.file_name}</div>
                                <div className="settings-backup-item-tags">
                                  <span className="settings-backup-tag">
                                    {getBackupSourceLabel(meta.trigger)}
                                  </span>
                                  <span className="settings-backup-tag">
                                    {getBackupModeLabel(meta.mode)}
                                  </span>
                                  {archiveFileName && (
                                    <span className="settings-backup-tag">
                                      {t('settings.transfer.backup.archiveTag')}
                                    </span>
                                  )}
                                </div>
                              </div>
                              <div className="settings-backup-item-meta">
                                <span>
                                  {t('settings.transfer.backup.fileTime', {
                                    time: formatBackupTime(file.modified_at_ms),
                                  })}
                                </span>
                                <span>
                                  {t('settings.transfer.backup.fileSize', {
                                    size: formatFileSize(file.size_bytes),
                                  })}
                                </span>
                                {file.archive_size_bytes ? (
                                  <span>
                                    {t('settings.transfer.backup.archiveSize', {
                                      size: formatFileSize(file.archive_size_bytes),
                                    })}
                                  </span>
                                ) : null}
                              </div>
                              {platformEntries.length > 0 && (
                                <div className="settings-backup-platforms">
                                  <div className="settings-backup-platforms-title">
                                    {t('settings.transfer.backup.platformsTitle')}
                                  </div>
                                  <div className="settings-backup-platform-list">
                                    {platformEntries.map((item) => {
                                      const isDownloadingPlatform =
                                        backupDownloadingPlatform === `${file.file_name}:${item.platform}`;
                                      return (
                                        <button
                                          key={item.platform}
                                          type="button"
                                          className="settings-backup-platform-pill"
                                          onClick={() => {
                                            void handleDownloadBackupPlatform(file.file_name, item.platform);
                                          }}
                                          disabled={backupControlsDisabled}
                                          title={t('settings.transfer.backup.platformDownloadAction')}
                                        >
                                          {isDownloadingPlatform ? (
                                            <RefreshCw size={13} className="loading-spinner" />
                                          ) : (
                                            <Download size={13} />
                                          )}
                                          <span>{getPlatformLabel(item.platform, t)}</span>
                                          <span className="settings-backup-platform-count">
                                            {item.account_count}
                                          </span>
                                        </button>
                                      );
                                    })}
                                  </div>
                                </div>
                              )}
                              <div className="settings-backup-item-actions">
                                <button
                                  className="btn btn-secondary btn-sm"
                                  onClick={() => {
                                    void handleDownloadBackupJson(file.file_name);
                                  }}
                                  disabled={backupControlsDisabled || isDeleting}
                                >
                                  {isDownloadingJson ? (
                                    <RefreshCw size={14} className="loading-spinner" />
                                  ) : (
                                    <Download size={14} />
                                  )}
                                  {t('settings.transfer.backup.downloadJsonAction')}
                                </button>
                                {archiveFileName && (
                                  <button
                                    className="btn btn-secondary btn-sm"
                                    onClick={() => {
                                      void handleDownloadBackupZip(archiveFileName);
                                    }}
                                    disabled={backupControlsDisabled || isDeleting}
                                  >
                                    {isDownloadingZip ? (
                                      <RefreshCw size={14} className="loading-spinner" />
                                    ) : (
                                      <Archive size={14} />
                                    )}
                                    {t('settings.transfer.backup.downloadZipAction')}
                                  </button>
                                )}
                                <button
                                  className="btn btn-secondary btn-sm"
                                  onClick={() => {
                                    void handleImportBackup(file.file_name);
                                  }}
                                  disabled={backupControlsDisabled || isDeleting}
                                >
                                  {isImportingBackup ? (
                                    <RefreshCw size={14} className="loading-spinner" />
                                  ) : (
                                    <FolderOpen size={14} />
                                  )}
                                  {t('settings.transfer.backup.importAction')}
                                </button>
                                <button
                                  className="btn btn-secondary btn-sm"
                                  onClick={() => {
                                    void handleDeleteBackup(file.file_name);
                                  }}
                                  disabled={backupControlsDisabled || isImportingBackup}
                                >
                                  {isDeleting ? (
                                    <RefreshCw size={14} className="loading-spinner" />
                                  ) : (
                                    <Trash2 size={14} />
                                  )}
                                  {t('settings.transfer.backup.deleteAction')}
                                </button>
                              </div>
                            </div>
                          );
                        })}
                      </div>
                    )}
                  </div>
                </div>

                {backupFeedbackNode}

                <div className="settings-transfer-modal-actions">
                  <button
                    className="btn btn-secondary"
                    onClick={closeBackupManagerModal}
                    disabled={backupControlsDisabled}
                  >
                    {t('common.close')}
                  </button>
                </div>
              </div>
            </div>
          </div>,
        )}

      {exportModal.showModal &&
        renderToBody(
          <ExportJsonModal
            isOpen={exportModal.showModal}
            title={`${t('common.shared.export.title')} JSON`}
            jsonContent={exportModal.jsonContent}
            hidden={exportModal.hidden}
            copied={exportModal.copied}
            saving={exportModal.saving}
            savedPath={exportModal.savedPath}
            canOpenSavedDirectory={exportModal.canOpenSavedDirectory}
            pathCopied={exportModal.pathCopied}
            onClose={exportModal.closeModal}
            onToggleHidden={exportModal.toggleHidden}
            onCopyJson={exportModal.copyJson}
            onSaveJson={exportModal.saveJson}
            onOpenSavedDirectory={exportModal.openSavedDirectory}
            onCopySavedPath={exportModal.copySavedPath}
          />,
        )}
    </>
  );
}
