import { useEffect, useState } from 'react';
import { Check, Copy, History, Key, ShieldCheck, Trash2, Save, Search, Download, Upload } from 'lucide-react';
import { save, open, confirm } from '@tauri-apps/plugin-dialog';
import { writeTextFile, readTextFile } from '@tauri-apps/plugin-fs';
import { useTranslation } from 'react-i18next';
import * as OTPAuth from 'otpauth';
import './TwoFactorAuthPage.css';

interface OTPRecord {
  id: string;
  secret: string;
  remark: string;
  createdAt: number;
}

const STORAGE_KEY_SAVED = 'agtools.two_factor_auth.saved.v2';
const STORAGE_KEY_HISTORY = 'agtools.two_factor_auth.history.v2';
const MAX_HISTORY = 30;

function createUniqueId(): string {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
    return crypto.randomUUID();
  }
  return `2fa-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
}

function normalizeSecret(secret: string): string {
  return secret.replace(/\s+/g, '').replace(/-/g, '');
}

function extractSecretOnly(dirty: string): string {
  // Replace anything that is not A-Za-z or 2-7
  return dirty.replace(/[^A-Za-z2-7]/g, '').toUpperCase();
}

function formatTime(ms: number): string {
  const date = new Date(ms);
  const MM = String(date.getMonth() + 1).padStart(2, '0');
  const DD = String(date.getDate()).padStart(2, '0');
  const HH = String(date.getHours()).padStart(2, '0');
  const mm = String(date.getMinutes()).padStart(2, '0');
  return `${MM}-${DD} ${HH}:${mm}`;
}

function getOtpToken(secret: string, autoClean = false): string {
  const normalized = autoClean ? extractSecretOnly(secret) : normalizeSecret(secret);
  if (!normalized) return '';

  try {
    const totp = new OTPAuth.TOTP({
      secret: OTPAuth.Secret.fromBase32(normalized),
      period: 30,
      digits: 6,
    });
    return totp.generate();
  } catch {
    return '';
  }
}

function loadRecords(key: string): OTPRecord[] {
  try {
    const raw = localStorage.getItem(key);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (Array.isArray(parsed)) return parsed as OTPRecord[];
  } catch {}
  return [];
}

function saveRecords(key: string, records: OTPRecord[]) {
  try {
    localStorage.setItem(key, JSON.stringify(records));
  } catch {}
}

export function TwoFactorAuthPage() {
  const { t } = useTranslation();
  
  // Use lazy initialization so we load data immediately instead of overwriting on first use effect
  const [saved, setSaved] = useState<OTPRecord[]>(() => loadRecords(STORAGE_KEY_SAVED));
  const [history, setHistory] = useState<OTPRecord[]>(() => loadRecords(STORAGE_KEY_HISTORY));
  
  // The actual inputs typed by the user
  const [querySecret, setQuerySecret] = useState('');
  const [queryRemark, setQueryRemark] = useState('');
  
  // The actively generated token data, only updated on "Query"
  const [activeQuery, setActiveQuery] = useState<{ secret: string, remark: string } | null>(null);

  const [activeListTab, setActiveListTab] = useState<'saved' | 'history'>('saved');
  const [copiedId, setCopiedId] = useState<string | null>(null);

  const [timeRemaining, setTimeRemaining] = useState(() => {
    const now = Math.floor(Date.now() / 1000);
    return 30 - (now % 30);
  });

  useEffect(() => {
    saveRecords(STORAGE_KEY_SAVED, saved);
  }, [saved]);

  useEffect(() => {
    saveRecords(STORAGE_KEY_HISTORY, history);
  }, [history]);

  useEffect(() => {
    const timer = window.setInterval(() => {
      const now = Math.floor(Date.now() / 1000);
      const value = 30 - (now % 30);
      setTimeRemaining(value === 0 ? 30 : value);
    }, 1000);
    return () => window.clearInterval(timer);
  }, []);

  const handleQuery = () => {
    const rawSecret = querySecret.trim();
    if (!extractSecretOnly(rawSecret)) return;

    setActiveQuery({ secret: rawSecret, remark: queryRemark });

    // Push explicitly to History and deduplicate on the top
    setHistory(prev => {
      // Don't duplicate the very last item if it's identical
      if (prev.length > 0 && prev[0].secret === rawSecret) {
        return prev;
      }
      
      const filtered = prev.filter(p => p.secret !== rawSecret);
      const newRecord: OTPRecord = {
        id: createUniqueId(),
        secret: rawSecret,
        remark: queryRemark,
        createdAt: Date.now()
      };
      return [newRecord, ...filtered].slice(0, MAX_HISTORY);
    });
  };

  const handleCopyCode = async (id: string, token: string) => {
    if (!token) return;
    try {
      await navigator.clipboard.writeText(token);
      setCopiedId(id);
      setTimeout(() => setCopiedId(null), 1200);
    } catch {}
  };

  const handleSaveToFavorites = () => {
    const rawSecret = querySecret.trim();
    if (!extractSecretOnly(rawSecret)) return;
    
    setSaved(prev => {
      const exists = prev.find(p => p.secret === rawSecret);
      if (exists) {
        // Just update remark if it exists
        return prev.map(p => p.id === exists.id ? { ...p, remark: queryRemark || p.remark } : p);
      }
      return [{
        id: createUniqueId(),
        secret: rawSecret,
        remark: queryRemark,
        createdAt: Date.now()
      }, ...prev];
    });
    
    // Auto clear input after saving
    setQuerySecret('');
    setQueryRemark('');
    setActiveQuery(null);
  };

  const handleLoadFromHistory = (record: OTPRecord) => {
    setQuerySecret(record.secret);
    setQueryRemark(record.remark);
    // Auto generate/query immediately when clicked from history
    setActiveQuery({ secret: record.secret, remark: record.remark });
  };

  const confirmDeleteSaved = async (id: string, secret: string) => {
    try {
      const yes = await confirm(t('twoFactorAuth.confirmDeleteSavedMsg', `确定要永久删除 \n{{secret}}\n 这条认证记录吗？`).replace('{{secret}}', secret), { title: t('twoFactorAuth.confirmDeleteSavedTitle', '删除确认'), kind: 'warning' });
      if (yes) setSaved(prev => prev.filter(x => x.id !== id));
    } catch {
      if (window.confirm(t('twoFactorAuth.confirmDeleteSavedFallback', '确定要永久删除这条认证记录吗？'))) setSaved(prev => prev.filter(x => x.id !== id));
    }
  };

  const confirmDeleteHistory = async (id: string, secret: string) => {
    try {
      const yes = await confirm(t('twoFactorAuth.confirmDeleteHistoryMsg', `确定要删除 \n{{secret}}\n 的查询历史吗？`).replace('{{secret}}', secret), { title: t('twoFactorAuth.confirmDeleteHistoryTitle', '删除历史'), kind: 'info' });
      if (yes) setHistory(prev => prev.filter(x => x.id !== id));
    } catch {
      if (window.confirm(t('twoFactorAuth.confirmDeleteHistoryFallback', '确定要删除这条查询历史吗？'))) setHistory(prev => prev.filter(x => x.id !== id));
    }
  };

  const confirmClearAllHistory = async () => {
    try {
      const yes = await confirm(t('twoFactorAuth.confirmClearAllMsg', '确定要清空全部近期查询历史吗？'), { title: t('twoFactorAuth.confirmClearAllTitle', '清空确认'), kind: 'warning' });
      if (yes) setHistory([]);
    } catch {
      if (window.confirm(t('twoFactorAuth.confirmClearAllMsg', '确定要清空全部近期查询历史吗？'))) setHistory([]);
    }
  };

  // Calculate the currently active token dynamically

  const handleExportSaved = async () => {
    if (saved.length === 0) return;
    try {
      const dataStr = JSON.stringify(saved, null, 2);
      const defaultFilename = `2fa_saved_export_${new Date().toISOString().split('T')[0]}.json`;

      // Try Tauri desktop file dialog first
      try {
        const filePath = await save({
          filters: [{ name: 'JSON', extensions: ['json'] }],
          defaultPath: defaultFilename,
        });

        if (filePath) {
          await writeTextFile(filePath, dataStr);
        }
        return; // Success, don't execute fallback
      } catch (e) {
        // Not in Tauri or user cancelled, fallback to web download
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

  const handleImportSaved = async () => {
    try {
      let dataStr = '';
      
      try {
        const selected = await open({
          multiple: false,
          filters: [{ name: 'JSON', extensions: ['json'] }]
        });
        
        if (selected) {
          // Tauri open can return string or string[]
          const filePath = Array.isArray(selected) ? selected[0] : selected;
          dataStr = await readTextFile(filePath);
        } else {
          return; // Cancelled
        }
      } catch (e) {
        console.warn('Tauri open failed, falling back to web upload', e);
        // Fallback for purely web/vite dev server
        await new Promise<void>((resolve, reject) => {
          const input = document.createElement('input');
          input.type = 'file';
          input.accept = '.json,application/json';
          input.onchange = (ev: any) => {
            const file = ev.target.files[0];
            if (!file) { resolve(); return; }
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
      if (!Array.isArray(parsed)) throw new Error('Invalid format: root must be an array');

      setSaved(prev => {
        const updated = [...prev];
        for (const item of parsed) {
          if (item && typeof item === 'object' && item.secret) {
            const rawSecret = String(item.secret).trim();
            const remark = item.remark || '';
            const createdAt = item.createdAt || Date.now();
            
            const existingIdx = updated.findIndex(p => p.secret === rawSecret);
            if (existingIdx >= 0) {
              // Merge/Update remark
              updated[existingIdx] = { 
                ...updated[existingIdx], 
                remark: remark || updated[existingIdx].remark 
              };
            } else {
              updated.unshift({
                id: createUniqueId(),
                secret: rawSecret,
                remark,
                createdAt
              });
            }
          }
        }
        return updated;
      });

    } catch (err) {
      console.error('Import error:', err);
      setQuerySecret(''); // Trigger re-render to alert maybe? 
      alert(t('twoFactorAuth.importErrorMsg', '导入失败，请检查文件格式是否为您之前导出的格式 (JSON Array)。'));
    }
  };

  const currentToken = activeQuery ? getOtpToken(activeQuery.secret, true) : '';

  const renderTableRows = (records: OTPRecord[], isHistory: boolean) => {
    if (records.length === 0) {
      return (
        <tr>
          <td colSpan={4}>
            <div className="tfa-empty-state">
              {isHistory ? t('twoFactorAuth.emptyHistory', '暂无查询历史') : t('twoFactorAuth.emptySaved', '您尚未保存任何 2FA 认证')}
            </div>
          </td>
        </tr>
      );
    }

    return records.map(record => {
      const token = getOtpToken(record.secret, true);
      const copied = copiedId === record.id;
      const isWarning = timeRemaining <= 5;
      
      return (
        <tr key={record.id}>
          {isHistory ? null : (
            <td className="tfa-remark-cell" title={record.remark || '--'}>{record.remark || '--'}</td>
          )}
          <td className="tfa-secret-cell" title={record.secret}>{record.secret}</td>
          <td>
            {token ? (
              <div style={{ display: 'flex', alignItems: 'center' }}>
                <span className="tfa-code-cell">{token}</span>
                <span className={`tfa-time-badge ${isWarning ? 'warning' : ''}`}>{timeRemaining}s</span>
              </div>
            ) : (
              <span style={{ color: 'var(--text-tertiary)' }}>{t('twoFactorAuth.invalidSecretVal', '无效秘钥')}</span>
            )}
          </td>
          <td style={{ color: 'var(--text-tertiary)', fontSize: '12px' }}>
            {formatTime(record.createdAt)}
          </td>
          <td>
            <div className="tfa-actions">
              <button 
                type="button"
                className={`action-btn ${copied ? 'is-success' : ''}`}
                title={t('twoFactorAuth.actionCopy', '复制验证码')}
                disabled={!token}
                onClick={() => handleCopyCode(record.id, token)}
              >
                {copied ? <Check size={14} /> : <Copy size={14} />}
              </button>
              
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
                title={t('twoFactorAuth.actionDelete', '删除')}
                onClick={() => isHistory ? confirmDeleteHistory(record.id, record.secret) : confirmDeleteSaved(record.id, record.secret)}
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
    <main className="main-content ghcp-accounts-page two-factor-query-page">
      <div className="page-heading" style={{ padding: '24px 24px 0' }}>
        <div>
          <h1>
            <ShieldCheck size={20} />
            <span>{t('twoFactorAuth.pageTitle', '2FA 查询器')}</span>
          </h1>
          <p>{t('twoFactorAuth.pageDescNew', '输入或粘贴 2FA Base32 秘钥即可即时生成验证码，查询系统会自动矫正并去除非标准字符。')}</p>
        </div>
      </div>

      <div className="query-section">
        <h3 style={{ margin: 0, fontSize: '15px', display: 'flex', alignItems: 'center', gap: '6px' }}>
          <Key size={16} /> {t('twoFactorAuth.panelQuery', '功能区 (查询面板)')}
        </h3>
        
        <div className="query-main">
          <div className="query-inputs">
            <div className="form-group" style={{ marginBottom: 0 }}>
              <input 
                type="text" 
                placeholder={t('twoFactorAuth.inputSecretPlaceholder', '在此粘贴 2FA 秘钥 (如: JBSWY3DPEHPK3PXP)')}
                value={querySecret}
                onChange={e => {
                  setQuerySecret(e.target.value);
                  setActiveQuery(null); // Clear current result when editing
                }}
                onKeyDown={e => {
                  if (e.key === 'Enter') handleQuery();
                }}
                style={{ fontSize: '15px', fontFamily: 'var(--font-mono)' }}
              />
            </div>
            
            <div className="form-group" style={{ marginBottom: 0 }}>
              <input 
                type="text" 
                placeholder={t('twoFactorAuth.inputRemarkPlaceholder', '[选填] 给这个秘钥设置一个备注名称')}
                value={queryRemark}
                onChange={e => setQueryRemark(e.target.value)}
                onKeyDown={e => {
                  if (e.key === 'Enter') handleQuery();
                }}
              />
            </div>
            
            <div className="query-actions-row">
              <button 
                className="btn btn-primary" 
                disabled={!extractSecretOnly(querySecret)}
                onClick={handleQuery}
              >
                <Search size={14} />
                {t('twoFactorAuth.btnQuery', '查 询')}
              </button>
              <button 
                className="btn btn-secondary" 
                disabled={!extractSecretOnly(querySecret)}
                onClick={handleSaveToFavorites}
              >
                <Save size={14} />
                {t('twoFactorAuth.btnSaveToFavorites', '保存到列表')}
              </button>
            </div>
          </div>

          <div className="query-result-box">
             {timeRemaining > 0 && currentToken && (
               <span className={`query-result-countdown ${timeRemaining <= 5 ? 'error-text' : ''}`} style={{ color: timeRemaining <= 5 ? 'var(--danger)' : undefined }}>
                 {t('twoFactorAuth.refreshInSeconds', '{{time}} 秒后刷新').replace('{{time}}', timeRemaining.toString())}
               </span>
             )}
             {currentToken ? (
                <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
                  <span className="query-result-code">{currentToken}</span>
                  <button 
                    className="action-btn"
                    onClick={() => handleCopyCode('querying', currentToken)}
                  >
                    {copiedId === 'querying' ? <Check size={18} /> : <Copy size={18} />}
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
              {activeListTab === 'saved' && (
                <>
                  <button className="btn btn-secondary btn-sm" onClick={handleImportSaved} title={t('twoFactorAuth.btnImportTitle', '导入本地的 JSON 文件')}>
                    <Upload size={14} /> {t('twoFactorAuth.btnImport', '导入')}
                  </button>
                  {saved.length > 0 && (
                    <button className="btn btn-secondary btn-sm" onClick={handleExportSaved} title={t('twoFactorAuth.btnExportTitle', '导出已保存为 JSON')}>
                      <Download size={14} /> {t('twoFactorAuth.btnExport', '导出')}
                    </button>
                  )}
                </>
              )}
              {activeListTab === 'history' && history.length > 0 && (
                <button className="btn btn-secondary btn-sm" onClick={confirmClearAllHistory}>
                  {t('twoFactorAuth.btnClear', '清空')}
                </button>
              )}
            </div>
          </div>
          
          <div className="list-content">
            {activeListTab === 'saved' ? (
              <table className="tfa-table">
                <thead>
                  <tr>
                    <th>{t('twoFactorAuth.tableRemark', '备注')}</th>
                    <th>{t('twoFactorAuth.tableSecret', '秘钥')}</th>
                    <th>{t('twoFactorAuth.tableCode', '动态码')}</th>
                    <th style={{ width: '100px' }}>{t('twoFactorAuth.tableAddedTime', '添加时间')}</th>
                    <th style={{ width: '90px' }}>{t('twoFactorAuth.tableActions', '操作')}</th>
                  </tr>
                </thead>
                <tbody>
                  {renderTableRows(saved, false)}
                </tbody>
              </table>
            ) : (
              <table className="tfa-table">
                <thead>
                  <tr>
                    <th>{t('twoFactorAuth.tableSecret', '秘钥')}</th>
                    <th>{t('twoFactorAuth.tableCode', '动态码')}</th>
                    <th style={{ width: '100px' }}>{t('twoFactorAuth.tableQueryTime', '查询时间')}</th>
                    <th style={{ width: '100px' }}>{t('twoFactorAuth.tableActions', '操作')}</th>
                  </tr>
                </thead>
                <tbody>
                  {renderTableRows(history, true)}
                </tbody>
              </table>
            )}
          </div>
        </div>
      </div>
    </main>
  );
}
