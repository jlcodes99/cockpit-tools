import { type ChangeEvent, type ClipboardEvent, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ArrowDown, ArrowUp, Check, Copy, Download, History, Key, Pencil, Trash2, Upload, X } from 'lucide-react';
import { save, open, confirm } from '@tauri-apps/plugin-dialog';
import { writeTextFile, readTextFile } from '@tauri-apps/plugin-fs';
import jsQR from 'jsqr';
import {
  MFA_STORAGE_KEY_HISTORY,
  MFA_STORAGE_KEY_SAVED,
  createMfaRecordId,
  dedupeMfaRecordsBySecret,
  getMfaOtpToken,
  getMfaTimeRemaining,
  loadMfaHistoryRecords,
  loadSavedMfaRecords,
  normalizeMfaRecord,
  parseMfaCredentialInput,
  toMfaSecretIdentity,
  type MfaRecord,
  type ParsedMfaCredential,
} from '../utils/mfaVault';

type SortDirection = 'asc' | 'desc';
type ListTab = 'saved' | 'history';
const MAX_HISTORY = 50;

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

  const [activeQuery, setActiveQuery] = useState<ParsedMfaCredential | null>(null);
  const [activeListTab, setActiveListTab] = useState<ListTab>('saved');

  const [savedTimeSort, setSavedTimeSort] = useState<SortDirection>('asc');
  const [historyTimeSort, setHistoryTimeSort] = useState<SortDirection>('asc');

  const [editingAccountId, setEditingAccountId] = useState<string | null>(null);
  const [editingAccountName, setEditingAccountName] = useState('');

  const [copiedId, setCopiedId] = useState<string | null>(null);
  const uploadInputRef = useRef<HTMLInputElement | null>(null);

  const [timeRemaining, setTimeRemaining] = useState(() => getMfaTimeRemaining());

  useEffect(() => {
    localStorage.setItem(MFA_STORAGE_KEY_SAVED, JSON.stringify(records));
  }, [records]);

  useEffect(() => {
    localStorage.setItem(MFA_STORAGE_KEY_HISTORY, JSON.stringify(historyRecords));
  }, [historyRecords]);

  useEffect(() => {
    const timer = window.setInterval(() => {
      setTimeRemaining(getMfaTimeRemaining());
    }, 1000);
    return () => window.clearInterval(timer);
  }, []);

  const toggleSortDirection = (value: SortDirection): SortDirection => (
    value === 'asc' ? 'desc' : 'asc'
  );

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

    setHistoryRecords(prev => {
      const next: MfaRecord = {
        id: createMfaRecordId(),
        accountName: parsed.accountName,
        secret: parsed.secret,
        remark: '',
        time: Date.now(),
      };
      const nextIdentity = toMfaSecretIdentity(next.secret);
      const filtered = prev.filter(record => toMfaSecretIdentity(record.secret) !== nextIdentity);
      return [next, ...filtered].slice(0, MAX_HISTORY);
    });
  };

  const parseAndQuery = (rawInput: string, invalidMessage?: string): ParsedMfaCredential | null => {
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
    const parsed = parseMfaCredentialInput(inputValue);

    if (!parsed) {
      setInputError(t('mfaVault.invalidOtpAuthInput'));
      return;
    }

    const finalAccountName = parsed.accountName || activeQuery?.accountName || '';

    setRecords(prev => {
      const parsedIdentity = toMfaSecretIdentity(parsed.secret);
      const existsIndex = prev.findIndex(record => toMfaSecretIdentity(record.secret) === parsedIdentity);
      if (existsIndex >= 0) {
        return prev.map((record, idx) => (
          idx === existsIndex
            ? {
                ...record,
                accountName: record.accountName || finalAccountName,
              }
            : record
        ));
      }

      const newRecord: MfaRecord = {
        id: createMfaRecordId(),
        accountName: finalAccountName,
        secret: parsed.secret,
        remark: '',
        time: Date.now(),
      };
      return [newRecord, ...prev];
    });

    setInputError('');
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
    try {
      const exportList = records.map(record => ({
        accountName: record.accountName,
        secret: record.secret,
        time: record.time,
      }));

      const dataStr = JSON.stringify(exportList, null, 2);
      const defaultFilename = `twofa_saved_export_${new Date().toISOString().split('T')[0]}.json`;

      try {
        const filePath = await save({
          filters: [{ name: 'JSON', extensions: ['json'] }],
          defaultPath: defaultFilename,
        });

        if (filePath) {
          await writeTextFile(filePath, dataStr);
        }
        return;
      } catch (e) {
        console.warn('Tauri save failed, falling back to web download', e);
      }

      const blob = new Blob([dataStr], { type: 'application/json' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = defaultFilename;
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
      URL.revokeObjectURL(url);
    } catch {}
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
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')} ${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
  };

  const currentToken = activeQuery ? getMfaOtpToken(activeQuery.secret) : '';
  const isWarning = timeRemaining <= 5;

  const renderRows = (source: MfaRecord[], isHistory: boolean) => {
    if (source.length === 0) {
      return (
        <tr>
          <td colSpan={5}>
            <div className="tfa-empty-state">
              {isHistory ? t('twoFactorAuth.emptyHistory', '暂无查询历史') : t('mfaVault.noData', '暂无保存的凭证数据')}
            </div>
          </td>
        </tr>
      );
    }

    const sortDirection = isHistory ? historyTimeSort : savedTimeSort;

    return [...source]
      .sort((a, b) => (
        sortDirection === 'asc'
          ? a.time - b.time
          : b.time - a.time
      ))
      .map(record => {
        const token = getMfaOtpToken(record.secret);
        const displayAccount = record.accountName || '--';

        return (
          <tr key={record.id}>
            <td title={displayAccount} style={{ fontWeight: 500 }}>
              {!isHistory && editingAccountId === record.id ? (
                <div className="mfa-vault-account-edit-wrap">
                  <div className="mfa-vault-account-edit-row">
                    <input
                      type="text"
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
                  <span className="mfa-vault-account-text">{displayAccount}</span>
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
              <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
                {token ? (
                  <>
                    <span className="tfa-code-cell">{token}</span>
                    <span className={`tfa-time-badge ${isWarning ? 'warning' : ''}`}>{timeRemaining}s</span>
                  </>
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
            <td style={{ color: 'var(--text-tertiary)', fontSize: '12px' }}>
              {formatTime(record.time)}
            </td>
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
    <>
      <div className="query-section">
        <h3 style={{ margin: 0, fontSize: '15px', display: 'flex', alignItems: 'center', gap: '6px' }}>
          <Key size={16} /> {t('twoFactorAuth.panelQuery', '功能区 (查询面板)')}
        </h3>

        <div className="query-main" style={{ alignItems: 'stretch' }}>
          <div className="query-inputs">
            <div className="form-group" style={{ marginBottom: 0 }}>
              <input
                type="text"
                placeholder={t('mfaVault.inputOtpAuthPlaceholder')}
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

            <div className="query-actions-row">
              <button
                className="btn btn-primary"
                onClick={handleQuery}
                disabled={!inputValue.trim() || recognizingImage}
              >
                {t('twoFactorAuth.btnQuery', '查 询')}
              </button>
              <button
                className="btn btn-secondary"
                onClick={handleSave}
                disabled={!inputValue.trim() || recognizingImage}
              >
                {t('twoFactorAuth.btnSaveToFavorites', '保存到列表')}
              </button>
              <button
                className="btn btn-secondary"
                onClick={openUploadDialog}
                disabled={recognizingImage}
                title={t('mfaVault.uploadQrImageTitle')}
              >
                <Upload size={14} /> {recognizingImage ? t('mfaVault.recognizing') : t('mfaVault.uploadQrImage')}
              </button>
            </div>

            <input
              ref={uploadInputRef}
              type="file"
              accept="image/*"
              onChange={handleUploadQrImage}
              style={{ display: 'none' }}
            />

            <div className="mfa-vault-manual-hint">
              {t('mfaVault.otpauthInputHint')}
              {' '}
              {t('mfaVault.pasteQrImageHint')}
            </div>
            {inputError && <div className="mfa-vault-inline-error">{inputError}</div>}
          </div>

          <div className="query-result-box">
            {timeRemaining > 0 && currentToken && (
              <span
                className={`query-result-countdown ${timeRemaining <= 5 ? 'error-text' : ''}`}
                style={{ color: timeRemaining <= 5 ? 'var(--danger)' : undefined }}
              >
                {t('twoFactorAuth.refreshInSeconds', '{{time}} 秒后刷新').replace('{{time}}', timeRemaining.toString())}
              </span>
            )}
            {currentToken ? (
              <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
                <span className="query-result-code">{currentToken}</span>
                <button
                  className="action-btn"
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
      </div>

      <div className="lists-section">
        <div className="list-panel">
          <div className="list-header">
            <div className="list-tabs">
              <button
                className={`list-tab ${activeListTab === 'saved' ? 'active' : ''}`}
                onClick={() => setActiveListTab('saved')}
              >
                {t('twoFactorAuth.tabSaved', '★ 已保存')}
              </button>
              <button
                className={`list-tab ${activeListTab === 'history' ? 'active' : ''}`}
                onClick={() => setActiveListTab('history')}
              >
                <History size={16} /> {t('twoFactorAuth.tabHistory', '近期查询')}
              </button>
            </div>

            <div className="list-actions">
              {activeListTab === 'saved' ? (
                <>
                  <button className="btn btn-secondary btn-sm" onClick={handleImport}>
                    <Upload size={14} /> {t('mfaVault.import', '导入')}
                  </button>
                  {records.length > 0 ? (
                    <button className="btn btn-secondary btn-sm" onClick={handleExport}>
                      <Download size={14} /> {t('mfaVault.export', '导出')}
                    </button>
                  ) : null}
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

          <div className="list-content">
            {activeListTab === 'saved' ? (
              <table className="tfa-table">
                <thead>
                  <tr>
                    <th style={{ width: '24%' }}>{t('mfaVault.accountName', '账号名')}</th>
                    <th>{t('twoFactorAuth.tableSecret', '秘钥')}</th>
                    <th>{t('mfaVault.dynamicCode', '动态码')}</th>
                    <th style={{ width: '15%' }}>
                      <span className="tfa-sort-header">
                        {t('mfaVault.time', '时间')}
                        <button
                          type="button"
                          className="tfa-sort-toggle"
                          title={t('instances.sort.toggleDirection', '切换排序方向')}
                          aria-label={t('instances.sort.toggleDirection', '切换排序方向')}
                          onClick={() => setSavedTimeSort(prev => toggleSortDirection(prev))}
                        >
                          {savedTimeSort === 'asc' ? <ArrowUp size={13} /> : <ArrowDown size={13} />}
                        </button>
                      </span>
                    </th>
                    <th style={{ width: '100px' }}>{t('mfaVault.actions', '操作')}</th>
                  </tr>
                </thead>
                <tbody>{renderRows(records, false)}</tbody>
              </table>
            ) : (
              <table className="tfa-table">
                <thead>
                  <tr>
                    <th style={{ width: '24%' }}>{t('mfaVault.accountName', '账号名')}</th>
                    <th>{t('twoFactorAuth.tableSecret', '秘钥')}</th>
                    <th>{t('mfaVault.dynamicCode', '动态码')}</th>
                    <th style={{ width: '15%' }}>
                      <span className="tfa-sort-header">
                        {t('mfaVault.time', '时间')}
                        <button
                          type="button"
                          className="tfa-sort-toggle"
                          title={t('instances.sort.toggleDirection', '切换排序方向')}
                          aria-label={t('instances.sort.toggleDirection', '切换排序方向')}
                          onClick={() => setHistoryTimeSort(prev => toggleSortDirection(prev))}
                        >
                          {historyTimeSort === 'asc' ? <ArrowUp size={13} /> : <ArrowDown size={13} />}
                        </button>
                      </span>
                    </th>
                    <th style={{ width: '120px' }}>{t('mfaVault.actions', '操作')}</th>
                  </tr>
                </thead>
                <tbody>{renderRows(historyRecords, true)}</tbody>
              </table>
            )}
          </div>
        </div>
      </div>
    </>
  );
}
