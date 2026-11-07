import { type ChangeEvent, type ClipboardEvent, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ArrowDown, ArrowUp, ArrowUpDown, Check, Copy, Download, History, Info, Key, Pencil, QrCode, Save, ScanLine, Search, Trash2, Upload, X } from 'lucide-react';
import { isTauri } from '@tauri-apps/api/core';
import { save, open, confirm } from '@tauri-apps/plugin-dialog';
import { writeTextFile, readTextFile } from '@tauri-apps/plugin-fs';
import jsQR from 'jsqr';
import { buildMfaMigrationQrPages, type MfaMigrationQrPage } from '../utils/mfaMigrationQr';
import {
  MFA_STORAGE_KEY_HISTORY,
  MFA_STORAGE_KEY_SAVED,
  createMfaRecordId,
  dedupeMfaRecordsBySecret,
  filterMfaRecords,
  getMfaOtpToken,
  getMfaTimeRemaining,
  loadMfaHistoryRecords,
  loadSavedMfaRecords,
  normalizeMfaRecord,
  parseGoogleAuthenticatorMigrationBatch,
  parseMfaCredentialInput,
  parseMfaCredentialInputs,
  rememberMfaQuery,
  sortMfaRecords,
  splitMfaAccountName,
  toMfaSecretIdentity,
  type GoogleAuthenticatorMigrationBatch,
  type MfaRecord,
  type MfaSortField,
  type MfaSortState,
  type ParsedMfaCredential,
} from '../utils/mfaVault';

type ListTab = 'saved' | 'history';
type MigrationBatchState = GoogleAuthenticatorMigrationBatch & {
  credentialsByIndex: Record<number, ParsedMfaCredential[]>;
};

async function saveExportFile(content: string, filename: string, extension: string, mime: string) {
  if (isTauri()) {
    const path = await save({ filters: [{ name: extension.toUpperCase(), extensions: [extension] }], defaultPath: filename });
    if (path) await writeTextFile(path, content);
    return;
  }
  const url = URL.createObjectURL(new Blob([content], { type: mime }));
  const link = document.createElement('a');
  link.href = url;
  link.download = filename;
  document.body.appendChild(link);
  link.click();
  link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

async function decodeQrTextFromImage(file: Blob): Promise<string | null> {
  const imageUrl = URL.createObjectURL(file);
  try {
    const image = await new Promise<HTMLImageElement>((resolve, reject) => {
      const img = new Image();
      img.onload = () => resolve(img);
      img.onerror = reject;
      img.src = imageUrl;
    });

    const maxSide = 2200;
    const scale = Math.min(1, maxSide / Math.max(image.naturalWidth, image.naturalHeight));
    const width = Math.max(1, Math.round(image.naturalWidth * scale));
    const height = Math.max(1, Math.round(image.naturalHeight * scale));

    const canvas = document.createElement('canvas');
    canvas.width = width;
    canvas.height = height;

    const context = canvas.getContext('2d', { willReadFrequently: true });
    if (!context) return null;

    context.drawImage(image, 0, 0, width, height);
    const imageData = context.getImageData(0, 0, width, height);
    const result = jsQR(imageData.data, imageData.width, imageData.height, {
      inversionAttempts: 'attemptBoth',
    });
    const value = result?.data?.trim();
    return value || null;
  } finally {
    URL.revokeObjectURL(imageUrl);
  }
}

export function MfaVaultManager() {
  const { t } = useTranslation();

  const [records, setRecords] = useState<MfaRecord[]>(() => loadSavedMfaRecords());
  const [historyRecords, setHistoryRecords] = useState<MfaRecord[]>(() => loadMfaHistoryRecords());

  const [inputValue, setInputValue] = useState('');
  const [inputError, setInputError] = useState('');
  const [recognizingImage, setRecognizingImage] = useState(false);
  const [migrationBatches, setMigrationBatches] = useState<Record<string, MigrationBatchState>>({});
  const [activeMigrationBatchId, setActiveMigrationBatchId] = useState<string | null>(null);

  const [activeQuery, setActiveQuery] = useState<ParsedMfaCredential | null>(null);
  const [activeListTab, setActiveListTab] = useState<ListTab>('saved');
  const [searchQuery, setSearchQuery] = useState('');
  const [exportError, setExportError] = useState('');
  const [qrPages, setQrPages] = useState<MfaMigrationQrPage[]>([]);
  const [qrPageIndex, setQrPageIndex] = useState(0);
  const [qrExportScope, setQrExportScope] = useState<'selected' | 'all'>('all');
  const [savingQr, setSavingQr] = useState(false);
  const qrDialogRef = useRef<HTMLDialogElement | null>(null);

  const [savedSort, setSavedSort] = useState<MfaSortState>({ field: 'time', direction: 'asc' });
  const [historySort, setHistorySort] = useState<MfaSortState>({ field: 'time', direction: 'asc' });
  const activeSort = activeListTab === 'saved' ? savedSort : historySort;
  const visibleRecords = useMemo(() => (
    activeListTab === 'saved'
      ? sortMfaRecords(filterMfaRecords(records, searchQuery), savedSort)
      : sortMfaRecords(filterMfaRecords(historyRecords, searchQuery), historySort)
  ), [activeListTab, records, historyRecords, savedSort, historySort, searchQuery]);

  const [selectedSavedIds, setSelectedSavedIds] = useState<Set<string>>(() => new Set());
  const selectedSavedRecords = useMemo(
    () => filterMfaRecords(records, searchQuery).filter(record => selectedSavedIds.has(record.id)),
    [records, selectedSavedIds, searchQuery],
  );
  const visibleSelectedCount = activeListTab === 'saved'
    ? visibleRecords.filter(record => selectedSavedIds.has(record.id)).length : 0;
  const allSavedSelected = visibleRecords.length > 0 && visibleSelectedCount === visibleRecords.length;
  const hasPartialSelection = visibleSelectedCount > 0 && !allSavedSelected;
  const selectAllCheckboxRef = useRef<HTMLInputElement | null>(null);
  const [batchDeleting, setBatchDeleting] = useState(false);
  const batchDeletePendingRef = useRef(false);

  const [editingAccountId, setEditingAccountId] = useState<string | null>(null);
  const [editingAccountName, setEditingAccountName] = useState('');

  const [copiedId, setCopiedId] = useState<string | null>(null);
  const uploadInputRef = useRef<HTMLInputElement | null>(null);

  const [timeRemaining, setTimeRemaining] = useState(() => getMfaTimeRemaining());

  const activeMigrationBatch = activeMigrationBatchId
    ? migrationBatches[activeMigrationBatchId] || null
    : null;
  const activeMigrationPartCount = activeMigrationBatch
    ? Object.keys(activeMigrationBatch.credentialsByIndex).length
    : 0;
  const activeMigrationIsComplete = !!activeMigrationBatch
    && activeMigrationPartCount >= activeMigrationBatch.batchSize;

  useEffect(() => {
    const dialog = qrDialogRef.current;
    if (!dialog) return;
    if (qrPages.length > 0 && !dialog.open) dialog.showModal();
    if (qrPages.length === 0 && dialog.open) dialog.close();
  }, [qrPages]);

  useEffect(() => {
    localStorage.setItem(MFA_STORAGE_KEY_SAVED, JSON.stringify(records));
  }, [records]);

  useEffect(() => {
    localStorage.setItem(MFA_STORAGE_KEY_HISTORY, JSON.stringify(historyRecords));
  }, [historyRecords]);

  useEffect(() => {
    if (selectAllCheckboxRef.current) {
      selectAllCheckboxRef.current.indeterminate = hasPartialSelection;
    }
  }, [activeListTab, hasPartialSelection]);

  useEffect(() => {
    const timer = window.setInterval(() => {
      setTimeRemaining(getMfaTimeRemaining());
    }, 1000);
    return () => window.clearInterval(timer);
  }, []);

  const handleSort = (field: MfaSortField) => {
    const setSort = activeListTab === 'saved' ? setSavedSort : setHistorySort;
    setSort(previous => ({
      field,
      direction: previous.field === field && previous.direction === 'asc' ? 'desc' : 'asc',
    }));
  };

  const toggleSavedSelection = (id: string) => {
    if (batchDeletePendingRef.current) return;
    setSelectedSavedIds(previous => {
      const next = new Set(previous);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const toggleSelectAllSaved = () => {
    if (batchDeletePendingRef.current) return;
    setSelectedSavedIds(allSavedSelected ? new Set() : new Set(visibleRecords.map(record => record.id)));
  };

  const handleSearchChange = (value: string) => {
    if (batchDeletePendingRef.current) return;
    setSearchQuery(value);
    setSelectedSavedIds(new Set());
  };

  const clearSavedSelection = () => {
    if (!batchDeletePendingRef.current) setSelectedSavedIds(new Set());
  };

  const handleCopy = async (id: string, text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopiedId(id);
      setTimeout(() => setCopiedId(null), 1200);
    } catch {}
  };

  const applyQueryResult = (parsed: ParsedMfaCredential) => {
    setActiveQuery(parsed);
    setInputError('');
    setHistoryRecords(rememberMfaQuery(parsed));
  };

  const collectMigrationBatch = (batch: GoogleAuthenticatorMigrationBatch) => {
    const batchId = String(batch.batchId);
    setMigrationBatches(prev => {
      const current = prev[batchId];
      return {
        ...prev,
        [batchId]: {
          ...batch,
          batchSize: Math.max(current?.batchSize || 0, batch.batchSize),
          credentialsByIndex: {
            ...(current?.credentialsByIndex || {}),
            [batch.batchIndex]: batch.credentials,
          },
        },
      };
    });
    setActiveMigrationBatchId(batchId);
  };

  const clearActiveMigrationBatch = () => {
    if (!activeMigrationBatchId) return;
    setMigrationBatches(prev => {
      const next = { ...prev };
      delete next[activeMigrationBatchId];
      return next;
    });
    setActiveMigrationBatchId(null);
    setInputValue('');
    setActiveQuery(null);
    setInputError('');
  };

  const parseAndQuery = (rawInput: string, invalidMessage?: string): ParsedMfaCredential | null => {
    const migrationBatch = parseGoogleAuthenticatorMigrationBatch(rawInput);
    if (migrationBatch) collectMigrationBatch(migrationBatch);

    const parsed = parseMfaCredentialInput(rawInput);
    if (!parsed) {
      setInputError(invalidMessage || t('mfaVault.invalidOtpAuthInput'));
      return null;
    }

    applyQueryResult(parsed);
    return parsed;
  };

  const handleQuery = () => {
    parseAndQuery(inputValue);
  };

  const handleSave = () => {
    const migrationBatch = parseGoogleAuthenticatorMigrationBatch(inputValue);
    const collectedMigrationBatch = migrationBatch
      ? migrationBatches[String(migrationBatch.batchId)]
      : null;
    const parsedCredentials = collectedMigrationBatch
      ? Object.values(collectedMigrationBatch.credentialsByIndex).flat()
      : parseMfaCredentialInputs(inputValue);

    if (parsedCredentials.length === 0) {
      setInputError(t('mfaVault.invalidOtpAuthInput'));
      return;
    }

    setRecords(prev => {
      let nextRecords = [...prev];
      for (const parsed of parsedCredentials) {
        const finalAccountName = parsed.accountName || activeQuery?.accountName || '';
        const parsedIdentity = toMfaSecretIdentity(parsed.secret);
        const existsIndex = nextRecords.findIndex(record => toMfaSecretIdentity(record.secret) === parsedIdentity);
        if (existsIndex >= 0) {
          nextRecords[existsIndex] = {
            ...nextRecords[existsIndex],
            accountName: nextRecords[existsIndex].accountName || finalAccountName,
          };
        } else {
          nextRecords = [{
            id: createMfaRecordId(),
            accountName: finalAccountName,
            secret: parsed.secret,
            remark: '',
            time: Date.now(),
          }, ...nextRecords];
        }
      }
      return dedupeMfaRecordsBySecret(nextRecords);
    });

    setInputError('');

    if (migrationBatch && collectedMigrationBatch
      && Object.keys(collectedMigrationBatch.credentialsByIndex).length >= collectedMigrationBatch.batchSize) {
      setMigrationBatches(prev => {
        const next = { ...prev };
        delete next[String(migrationBatch.batchId)];
        return next;
      });
      setActiveMigrationBatchId(current => current === String(migrationBatch.batchId) ? null : current);
    }
  };

  const handleLoadFromHistory = (record: MfaRecord) => {
    setInputValue(record.secret);
    setActiveQuery({
      accountName: record.accountName,
      secret: record.secret,
    });
    setInputError('');
  };

  const startEditAccountName = (record: MfaRecord) => {
    setEditingAccountId(record.id);
    setEditingAccountName(record.accountName || '');
  };

  const cancelEditAccountName = () => {
    setEditingAccountId(null);
    setEditingAccountName('');
  };

  const saveEditAccountName = () => {
    if (!editingAccountId) return;
    setRecords(prev => prev.map(record => (
      record.id === editingAccountId
        ? { ...record, accountName: editingAccountName.trim() }
        : record
    )));
    cancelEditAccountName();
  };

  const confirmDeleteSaved = async (id: string, secret: string) => {
    try {
      const yes = await confirm(
        t('twoFactorAuth.confirmDeleteSavedMsg', '确定要永久删除 \n{{secret}}\n 这条认证记录吗？').replace('{{secret}}', secret),
        { title: t('twoFactorAuth.confirmDeleteSavedTitle', '删除确认'), kind: 'warning' }
      );
      if (yes) {
        setRecords(prev => prev.filter(item => item.id !== id));
        if (editingAccountId === id) cancelEditAccountName();
      }
    } catch {
      if (window.confirm(t('twoFactorAuth.confirmDeleteSavedFallback', '确定要永久删除这条认证记录吗？'))) {
        setRecords(prev => prev.filter(item => item.id !== id));
        if (editingAccountId === id) cancelEditAccountName();
      }
    }
  };

  const confirmDeleteSelectedSaved = async () => {
    if (batchDeletePendingRef.current || selectedSavedRecords.length === 0) return;
    const idsToDelete = new Set(selectedSavedRecords.map(record => record.id));
    const message = t('messages.batchDeleteConfirm', { count: idsToDelete.size });
    batchDeletePendingRef.current = true;
    setBatchDeleting(true);

    try {
      let confirmed: boolean;
      try {
        confirmed = await confirm(message, {
          title: t('twoFactorAuth.confirmDeleteSavedTitle', '删除确认'),
          kind: 'warning',
        });
      } catch {
        confirmed = window.confirm(message);
      }
      if (!confirmed) return;

      setRecords(previous => previous.filter(record => !idsToDelete.has(record.id)));
      setSelectedSavedIds(previous => new Set([...previous].filter(id => !idsToDelete.has(id))));
      setEditingAccountId(current => current && idsToDelete.has(current) ? null : current);
    } finally {
      batchDeletePendingRef.current = false;
      setBatchDeleting(false);
    }
  };

  const confirmDeleteHistory = async (id: string, secret: string) => {
    try {
      const yes = await confirm(
        t('twoFactorAuth.confirmDeleteHistoryMsg', '确定要删除 \n{{secret}}\n 的查询历史吗？').replace('{{secret}}', secret),
        { title: t('twoFactorAuth.confirmDeleteHistoryTitle', '删除历史'), kind: 'info' }
      );
      if (yes) {
        setHistoryRecords(prev => prev.filter(item => item.id !== id));
      }
    } catch {
      if (window.confirm(t('twoFactorAuth.confirmDeleteHistoryFallback', '确定要删除这条查询历史吗？'))) {
        setHistoryRecords(prev => prev.filter(item => item.id !== id));
      }
    }
  };

  const confirmClearAllHistory = async () => {
    try {
      const yes = await confirm(t('twoFactorAuth.confirmClearAllMsg', '确定要清空全部近期查询历史吗？'), {
        title: t('twoFactorAuth.confirmClearAllTitle', '清空确认'),
        kind: 'warning'
      });
      if (yes) setHistoryRecords([]);
    } catch {
      if (window.confirm(t('twoFactorAuth.confirmClearAllMsg', '确定要清空全部近期查询历史吗？'))) {
        setHistoryRecords([]);
      }
    }
  };

  const handleExport = async () => {
    if (records.length === 0) return;
    setExportError('');
    try {
      const exportList = records.map(record => ({
        accountName: record.accountName,
        secret: record.secret,
        remark: record.remark || '',
        time: record.time,
      }));

      const dataStr = JSON.stringify(exportList, null, 2);
      const defaultFilename = `twofa_saved_export_${new Date().toISOString().split('T')[0]}.json`;

      await saveExportFile(dataStr, defaultFilename, 'json', 'application/json');
    } catch { setExportError(t('mfaVault.exportFailed')); }
  };

  const handleExportMigration = () => {
    setExportError('');
    const selected = selectedSavedRecords.length > 0;
    const source = selected ? selectedSavedRecords : records;
    if (!source.length) return;
    try {
      const pages = buildMfaMigrationQrPages(source);
      setQrExportScope(selected ? 'selected' : 'all');
      setQrPageIndex(0);
      setQrPages(pages);
    } catch (error) {
      const key = error instanceof Error ? error.message : '';
      const supportedKeys = ['mfaVault.migrationInvalidRecord', 'mfaVault.migrationRecordTooLarge',
        'mfaVault.migrationUnsupportedParameters', 'mfaVault.migrationQrFailed'];
      setExportError(t(supportedKeys.includes(key) ? key : 'mfaVault.migrationQrFailed'));
    }
  };

  const handleSaveMigrationImage = async () => {
    const page = qrPages[qrPageIndex];
    if (!page || savingQr) return;
    setSavingQr(true);
    setExportError('');
    try {
      await saveExportFile(page.svg, `twofa_migration_${qrPageIndex + 1}_of_${qrPages.length}.svg`, 'svg', 'image/svg+xml');
    } catch { setExportError(t('mfaVault.exportFailed')); }
    finally { setSavingQr(false); }
  };

  const handleImport = async () => {
    try {
      let dataStr = '';
      try {
        const selected = await open({
          multiple: false
        });

        if (selected) {
          const filePath = Array.isArray(selected) ? selected[0] : selected;
          dataStr = await readTextFile(filePath);
        } else {
          return;
        }
      } catch (e) {
        await new Promise<void>((resolve, reject) => {
          const input = document.createElement('input');
          input.type = 'file';
          input.accept = '*/*';
          input.onchange = (ev: Event) => {
            const target = ev.target as HTMLInputElement;
            const file = target.files?.[0];
            if (!file) {
              resolve();
              return;
            }
            const reader = new FileReader();
            reader.onload = (res) => {
              dataStr = res.target?.result as string;
              resolve();
            };
            reader.onerror = reject;
            reader.readAsText(file);
          };
          input.click();
        });
      }

      if (!dataStr) return;

      const parsed = JSON.parse(dataStr);
      if (!Array.isArray(parsed)) throw new Error('Invalid format');

      setRecords(prev => {
        const incoming = parsed
          .map(normalizeMfaRecord)
          .filter((item): item is MfaRecord => !!item);
        return dedupeMfaRecordsBySecret([...incoming, ...prev]);
      });
    } catch (err) {
      console.error('Import error:', err);
      alert(t('mfaVault.importErrorMsg', '导入失败，请检查文件内容格式。'));
    }
  };

  const handlePasteImage = async (event: ClipboardEvent<HTMLInputElement>) => {
    const items = Array.from(event.clipboardData?.items || []);
    const imageItem = items.find(item => item.type.startsWith('image/'));
    if (!imageItem) return;

    event.preventDefault();
    const imageFile = imageItem.getAsFile();
    if (!imageFile) return;

    await handleDecodeAndQueryFromImage(imageFile);
  };

  const handleDecodeAndQueryFromImage = async (file: Blob) => {
    setRecognizingImage(true);
    setInputError('');
    try {
      const decodedText = await decodeQrTextFromImage(file);
      if (!decodedText) {
        setInputError(t('mfaVault.qrDecodeFailed'));
        return;
      }

      setInputValue(decodedText);
      const parsed = parseAndQuery(decodedText, t('mfaVault.qrNotOtpAuth'));
      if (!parsed) return;
    } catch {
      setInputError(t('mfaVault.qrDecodeFailed'));
    } finally {
      setRecognizingImage(false);
    }
  };

  const handleUploadQrImage = async (event: ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    event.target.value = '';
    if (!file) return;
    await handleDecodeAndQueryFromImage(file);
  };

  const openUploadDialog = () => {
    if (!recognizingImage) {
      uploadInputRef.current?.click();
    }
  };

  const formatTime = (ms: number) => {
    const d = new Date(ms);
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')} ${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}:${String(d.getSeconds()).padStart(2, '0')}`;
  };

  const currentToken = activeQuery ? getMfaOtpToken(activeQuery.secret) : '';
  const isWarning = timeRemaining <= 5;

  const renderRows = (source: MfaRecord[], isHistory: boolean) => {
    if (source.length === 0) {
      return (
        <tr>
          <td colSpan={isHistory ? 5 : 6}>
            <div className="tfa-empty-state">
              {searchQuery.trim() ? t('mfaVault.noSearchResults') : isHistory
                ? t('twoFactorAuth.emptyHistory', '暂无查询历史') : t('mfaVault.noData', '暂无保存的凭证数据')}
            </div>
          </td>
        </tr>
      );
    }

    return source.map(record => {
      const token = getMfaOtpToken(record.secret);
      const displayAccount = record.accountName || '--';
      const accountParts = splitMfaAccountName(record.accountName);

      return (
        <tr key={record.id} className={!isHistory && selectedSavedIds.has(record.id) ? 'tfa-selected' : undefined}>
          {!isHistory && (
            <td className="tfa-select-column">
              <label className="tfa-select-control">
                <input
                  type="checkbox"
                  className="tfa-select-checkbox"
                  aria-label={displayAccount}
                  checked={selectedSavedIds.has(record.id)}
                  disabled={batchDeleting}
                  onChange={() => toggleSavedSelection(record.id)}
                />
              </label>
            </td>
          )}
          <td title={displayAccount} style={{ fontWeight: 500 }}>
            {!isHistory && editingAccountId === record.id ? (
              <div className="mfa-vault-account-edit-wrap">
                <div className="mfa-vault-account-edit-row">
                  <input
                    type="text"
                    title={t('mfaVault.editAccountHint')}
                    value={editingAccountName}
                    onChange={e => setEditingAccountName(e.target.value)}
                    onKeyDown={e => {
                      if (e.key === 'Enter') {
                        e.preventDefault();
                        saveEditAccountName();
                      } else if (e.key === 'Escape') {
                        e.preventDefault();
                        cancelEditAccountName();
                      }
                    }}
                    autoFocus
                  />
                  <button
                    type="button"
                    className="action-btn"
                    title={t('mfaVault.saveAccountName')}
                    onClick={saveEditAccountName}
                  >
                    <Check size={14} />
                  </button>
                  <button
                    type="button"
                    className="action-btn"
                    title={t('mfaVault.cancelEditAccountName')}
                    onClick={cancelEditAccountName}
                  >
                    <X size={14} />
                  </button>
                </div>
              </div>
            ) : (
              <div className="mfa-vault-account-cell">
                <span className="mfa-account-avatar" aria-hidden="true"><Key size={20} /></span>
                <div className="mfa-account-details">
                  <div className="mfa-account-name-row">
                    <span className="mfa-vault-account-text">{accountParts.issuer || accountParts.account || '--'}</span>
                    {!isHistory ? (
                      <button
                        type="button"
                        className="action-btn"
                        title={t('mfaVault.editAccountName')}
                        onClick={() => startEditAccountName(record)}
                      >
                        <Pencil size={14} />
                      </button>
                    ) : null}
                  </div>
                  {accountParts.issuer && <span className="mfa-account-secondary">{accountParts.account}</span>}
                </div>
              </div>
            )}
          </td>
          <td className="tfa-secret-cell" title={record.secret}>
            <div className="tfa-secret-wrap">
              <span className="tfa-secret-value">{record.secret}</span>
              <button
                type="button"
                className="action-btn"
                title={t('mfaVault.copySecret', '复制秘钥')}
                onClick={() => handleCopy(`${record.id}-secret`, record.secret)}
              >
                {copiedId === `${record.id}-secret` ? <Check size={14} className="is-success" /> : <Copy size={14} />}
              </button>
            </div>
          </td>
          <td>
            <div className="tfa-code-wrap">
              {token ? (
                <span className="tfa-code-cell">{token}</span>
              ) : (
                <span style={{ color: 'var(--text-tertiary)' }}>
                  {t('mfaVault.invalidSecretVal', '无效秘钥')}
                </span>
              )}
              <button
                type="button"
                className="action-btn"
                title={t('mfaVault.copyCode', '复制验证码')}
                disabled={!token}
                onClick={() => handleCopy(`${record.id}-code`, token)}
              >
                {copiedId === `${record.id}-code` ? <Check size={14} className="is-success" /> : <Copy size={14} />}
              </button>
            </div>
          </td>
          <td className="tfa-time-cell">{formatTime(record.time)}</td>
          <td>
            <div className="tfa-actions">
              {isHistory ? (
                <button
                  type="button"
                  className="action-btn"
                  title={t('twoFactorAuth.actionReload', '重新加载到查询器')}
                  onClick={() => handleLoadFromHistory(record)}
                >
                  <History size={14} />
                </button>
              ) : null}
              <button
                type="button"
                className="action-btn is-danger"
                title={t('mfaVault.delete', '删除')}
                onClick={() => (isHistory ? confirmDeleteHistory(record.id, record.secret) : confirmDeleteSaved(record.id, record.secret))}
              >
                <Trash2 size={14} />
              </button>
            </div>
          </td>
        </tr>
      );
    });
  };

  return (
    <div className="mfa-vault-workspace">
      <section className="query-section" aria-label={t('twoFactorAuth.panelQuery', '功能区 (查询面板)')}>
        <div className="mfa-query-header">
          <span className="mfa-query-icon"><Key size={20} aria-hidden="true" /></span>
          <h1>{t('twoFactorAuth.pageTitle')}</h1>
          <button type="button" className="mfa-storage-info" aria-label={t('twoFactorAuth.pageDescNew')}>
            <Info size={16} aria-hidden="true" />
          </button>
          <span className="mfa-storage-tooltip" role="tooltip">{t('twoFactorAuth.pageDescNew')}</span>
        </div>
        <div className="query-main">
          <div className="query-inputs">
            <div className="query-input-header">
              <label htmlFor="mfa-credential-input">{t('twoFactorAuth.tableSecret', '秘钥')}</label>
              <div className="query-actions-row">
                <button
                  className="btn btn-secondary"
                  onClick={openUploadDialog}
                  disabled={recognizingImage}
                  title={t('mfaVault.uploadQrImageTitle')}
                >
                  <Upload size={14} aria-hidden="true" />
                  {recognizingImage ? t('mfaVault.recognizing') : t('mfaVault.uploadQrImage')}
                </button>
                <button
                  className="btn btn-secondary"
                  onClick={handleSave}
                  disabled={!inputValue.trim() || recognizingImage}
                >
                  <Save size={14} aria-hidden="true" /> {t('twoFactorAuth.btnSaveToFavorites')}
                </button>
              </div>
            </div>
            <div className="query-toolbar">
              <div className="form-group">
                <input
                  id="mfa-credential-input"
                  type="text"
                  aria-label={t('mfaVault.inputOtpAuthPlaceholder')}
                  placeholder={t('mfaVault.inputOtpAuthPlaceholder')}
                  title={`${t('mfaVault.otpauthInputHint')} ${t('mfaVault.pasteQrImageHint')}`}
                  value={inputValue}
                  onChange={e => {
                    setInputValue(e.target.value);
                    if (inputError) setInputError('');
                  }}
                  onPaste={handlePasteImage}
                  onKeyDown={e => {
                    if (e.key === 'Enter') handleQuery();
                  }}
                  style={{ fontFamily: 'var(--font-mono)' }}
                />
              </div>

              <button
                className="btn btn-primary"
                onClick={handleQuery}
                disabled={!inputValue.trim() || recognizingImage}
              >
                <Search size={18} aria-hidden="true" /> {t('twoFactorAuth.btnQuery', '查 询')}
              </button>
            </div>

            <input
              ref={uploadInputRef}
              type="file"
              accept="image/*"
              onChange={handleUploadQrImage}
              style={{ display: 'none' }}
            />

            {activeMigrationBatch && (
              <div className="mfa-vault-migration-row">
                <span className="mfa-vault-manual-hint">
                  {t('mfaVault.migrationBatchProgress', 'Google Authenticator 迁移批次：已识别 {{scanned}}/{{total}} 张二维码。')
                    .replace('{{scanned}}', activeMigrationPartCount.toString())
                    .replace('{{total}}', activeMigrationBatch.batchSize.toString())}
                  {!activeMigrationIsComplete && (
                    <> {t('mfaVault.migrationBatchIncomplete', '批次未完成，仍可保存已扫描账号。')}</>
                  )}
                </span>
                <div className="query-actions-row">
                  <button className="btn btn-secondary btn-sm" onClick={openUploadDialog} disabled={recognizingImage}>
                    <Upload size={14} /> {t('mfaVault.continueMigrationScan', '继续扫描')}
                  </button>
                  <button className="btn btn-secondary btn-sm" onClick={handleSave} disabled={recognizingImage}>
                    {activeMigrationIsComplete
                      ? t('mfaVault.saveCompleteMigrationBatch', '保存完整批次')
                      : t('mfaVault.savePartialMigrationBatch', '保存已扫描账号')}
                  </button>
                  <button className="btn btn-secondary btn-sm" onClick={clearActiveMigrationBatch}>
                    {t('mfaVault.clearMigrationBatch', '清空批次')}
                  </button>
                </div>
              </div>
            )}
            {inputError && <div className="mfa-vault-inline-error">{inputError}</div>}
          </div>

          <div className="query-result-box">
            {!currentToken && <ScanLine className="query-result-icon" size={26} aria-hidden="true" />}
            <span className="query-result-label">{t('mfaVault.dynamicCode', '动态码')}</span>
            {timeRemaining > 0 && currentToken && (
              <span
                className={`query-result-countdown ${timeRemaining <= 5 ? 'error-text' : ''}`}
                style={{ color: timeRemaining <= 5 ? 'var(--danger)' : undefined }}
              >
                {t('twoFactorAuth.refreshInSeconds', '{{time}} 秒后刷新').replace('{{time}}', timeRemaining.toString())}
              </span>
            )}
            {currentToken ? (
              <div className="query-result-value">
                <span className="query-result-code">{currentToken}</span>
                <button
                  className="action-btn"
                  title={t('mfaVault.copyCode', '复制验证码')}
                  onClick={() => handleCopy('query-code', currentToken)}
                >
                  {copiedId === 'query-code' ? <Check size={18} /> : <Copy size={18} />}
                </button>
              </div>
            ) : (
              <span className="query-result-empty">
                {t('twoFactorAuth.emptyQueryData', '暂未查询数据')}
              </span>
            )}
          </div>
        </div>
      </section>

      <div className="lists-section">
        <div className="list-panel">
          <div className="list-header">
            <div className="list-tabs">
              <button
                className={`list-tab ${activeListTab === 'saved' ? 'active' : ''}`}
                aria-pressed={activeListTab === 'saved'}
                onClick={() => setActiveListTab('saved')}
              >
                {t('twoFactorAuth.tabSaved', '★ 已保存')}
                <span className="list-tab-count">{records.length}</span>
              </button>
              <button
                className={`list-tab ${activeListTab === 'history' ? 'active' : ''}`}
                aria-pressed={activeListTab === 'history'}
                onClick={() => setActiveListTab('history')}
              >
                <History size={16} /> {t('twoFactorAuth.tabHistory', '近期查询')}
                <span className="list-tab-count">{historyRecords.length}</span>
              </button>
            </div>

          </div>
          <div className="mfa-list-toolbar">
            <label className="mfa-list-search">
              <Search size={18} aria-hidden="true" />
              <input type="search" value={searchQuery} disabled={batchDeleting}
                aria-label={t('mfaVault.searchPlaceholder')} placeholder={t('mfaVault.searchPlaceholder')}
                onChange={event => handleSearchChange(event.target.value)} />
            </label>
            <div className="list-actions">
              {activeListTab === 'saved' ? (
                <>
                  <button className="btn btn-secondary btn-sm" onClick={handleExportMigration} disabled={!records.length || batchDeleting}>
                    <QrCode size={16} aria-hidden="true" /> {t('mfaVault.exportMigrationQr')}
                  </button>
                  <button className="btn btn-secondary btn-sm" onClick={handleExport} disabled={!records.length}
                    title={t('mfaVault.exportBackupHint')}>
                    <Download size={14} aria-hidden="true" /> {t('mfaVault.exportBackup')}
                  </button>
                  <button className="btn btn-secondary btn-sm" onClick={handleImport}>
                    <Upload size={14} aria-hidden="true" /> {t('mfaVault.importBackup')}
                  </button>
                  {selectedSavedRecords.length > 0 && (
                      <button
                        type="button"
                        className="btn btn-secondary btn-sm"
                        onClick={clearSavedSelection}
                        disabled={batchDeleting}
                      >
                        {t('messages.clearSelection', '取消选择')}
                      </button>
                  )}
                  <button
                    type="button"
                    className="btn btn-secondary btn-sm"
                    onClick={confirmDeleteSelectedSaved}
                    disabled={batchDeleting || selectedSavedRecords.length === 0}
                  >
                    <Trash2 size={14} aria-hidden="true" />
                    {t('mfaVault.batchDelete')}{selectedSavedRecords.length > 0 ? ` (${selectedSavedRecords.length})` : ''}
                  </button>
                </>
              ) : (
                historyRecords.length > 0 ? (
                  <button className="btn btn-secondary btn-sm" onClick={confirmClearAllHistory}>
                    {t('twoFactorAuth.btnClear', '清空')}
                  </button>
                ) : null
              )}
            </div>
          </div>
          {exportError && qrPages.length === 0 && <p className="mfa-export-error" role="alert">{exportError}</p>}

          <div className="list-content">
            <table className="tfa-table">
              <thead>
                <tr>
                  {activeListTab === 'saved' && (
                    <th className="tfa-select-column" scope="col">
                      <label className="tfa-select-control" title={t('common.selectAll', '全选')}>
                        <input
                          ref={selectAllCheckboxRef}
                          type="checkbox"
                          className="tfa-select-checkbox"
                          aria-label={t('common.selectAll', '全选')}
                          checked={allSavedSelected}
                          disabled={batchDeleting || visibleRecords.length === 0}
                          onChange={toggleSelectAllSaved}
                        />
                      </label>
                    </th>
                  )}
                  <th
                    className="tfa-account-column"
                    scope="col"
                    aria-sort={activeSort.field === 'accountName' ? (activeSort.direction === 'asc' ? 'ascending' : 'descending') : 'none'}
                  >
                    <button
                      type="button"
                      className={`tfa-sort-toggle ${activeSort.field === 'accountName' ? 'active' : ''}`}
                      title={t('instances.sort.toggleDirection', '切换排序方向')}
                      onClick={() => handleSort('accountName')}
                    >
                      {t('mfaVault.accountName', '账号名')}
                      {activeSort.field !== 'accountName' ? (
                        <ArrowUpDown size={14} aria-hidden="true" />
                      ) : activeSort.direction === 'asc' ? (
                        <ArrowUp size={14} aria-hidden="true" />
                      ) : (
                        <ArrowDown size={14} aria-hidden="true" />
                      )}
                    </button>
                  </th>
                  <th scope="col">{t('twoFactorAuth.tableSecret', '秘钥')}</th>
                  <th className="tfa-code-column" scope="col">
                    <div className="tfa-code-heading">
                      {t('mfaVault.dynamicCode', '动态码')}
                      <span className={`tfa-time-badge ${isWarning ? 'warning' : ''}`}>{timeRemaining}s</span>
                    </div>
                  </th>
                  <th className="tfa-time-column" scope="col"
                    aria-sort={activeSort.field === 'time' ? (activeSort.direction === 'asc' ? 'ascending' : 'descending') : 'none'}>
                    <button type="button" className={`tfa-sort-toggle ${activeSort.field === 'time' ? 'active' : ''}`}
                      title={t('instances.sort.toggleDirection', '切换排序方向')} onClick={() => handleSort('time')}>
                      {t('mfaVault.time', '时间')}
                      {activeSort.field !== 'time' ? <ArrowUpDown size={14} aria-hidden="true" />
                        : activeSort.direction === 'asc' ? <ArrowUp size={14} aria-hidden="true" />
                          : <ArrowDown size={14} aria-hidden="true" />}
                    </button>
                  </th>
                  <th className="tfa-actions-column" scope="col">{t('mfaVault.actions', '操作')}</th>
                </tr>
              </thead>
              <tbody>{renderRows(visibleRecords, activeListTab === 'history')}</tbody>
            </table>
          </div>
        </div>
      </div>
      <dialog className="mfa-migration-dialog" ref={qrDialogRef} aria-labelledby="mfa-migration-title"
        onCancel={() => setQrPages([])} onClose={() => setQrPages([])}>
        {qrPages[qrPageIndex] && <>
          <div className="mfa-migration-header">
            <h2 id="mfa-migration-title">{t('mfaVault.migrationTitle')}</h2>
            <button type="button" className="action-btn" onClick={() => setQrPages([])} aria-label={t('common.close')} autoFocus>
              <X size={20} aria-hidden="true" />
            </button>
          </div>
          <p className="mfa-migration-summary">{t(qrExportScope === 'selected'
            ? 'mfaVault.migrationSelectedSummary' : 'mfaVault.migrationAllSummary', {
            count: qrPages.reduce((sum, page) => sum + page.count, 0),
          })}</p>
          <p className="mfa-migration-warning">{t('mfaVault.migrationWarning')}</p>
          <img className="mfa-migration-image" src={qrPages[qrPageIndex].dataUrl}
            alt={t('mfaVault.migrationImageAlt', { current: qrPageIndex + 1, total: qrPages.length })} />
          <p className="mfa-migration-page">{t('mfaVault.migrationPage', {
            current: qrPageIndex + 1, total: qrPages.length, count: qrPages[qrPageIndex].count,
          })}</p>
          <div className="mfa-migration-actions">
            <button className="btn btn-secondary" disabled={qrPageIndex === 0 || savingQr}
              onClick={() => setQrPageIndex(index => index - 1)}>{t('mfaVault.migrationPrevious')}</button>
            <button className="btn btn-secondary" disabled={qrPageIndex === qrPages.length - 1 || savingQr}
              onClick={() => setQrPageIndex(index => index + 1)}>{t('mfaVault.migrationNext')}</button>
            <button className="btn btn-primary" disabled={savingQr} onClick={handleSaveMigrationImage}>
              <Download size={16} aria-hidden="true" /> {t('mfaVault.migrationSaveImage')}
            </button>
          </div>
          {exportError && <p className="mfa-export-error" role="alert">{exportError}</p>}
        </>}
      </dialog>
    </div>
  );
}
