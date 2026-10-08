import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { openUrl } from '@tauri-apps/plugin-opener';
import { useTranslation } from 'react-i18next';
import {
  BookmarkPlus,
  CheckCircle2,
  ExternalLink,
  Eye,
  EyeOff,
  KeyRound,
  Play,
  Trash2,
  Copy,
  Pencil,
  X,
} from 'lucide-react';
import {
  listModelProviderModels,
  queryModelProviderUsage,
  type ModelProviderModel,
  type ModelProviderUsageSummary,
} from '../services/modelProviderUsageService';
import {
  APIKEY_FUN_GLOBAL_ENDPOINT,
  APIKEY_FUN_REGISTER_URL,
  APIKEY_FUN_SOURCE_TAG,
  buildApiKeyFunProviderBaseUrl,
} from '../utils/apikeyFunLinks';
import {
  dispatchApiKeyFunPrefillEvent,
  getApiKeyFunPrefillPage,
  type ApiKeyFunPrefillTarget,
} from '../utils/apiKeyFunPrefill';
import { ApiKeyFunKeyConfigModal } from '../components/apiKeyFun/ApiKeyFunKeyConfigModal';
import { useSponsorStore } from '../stores/useSponsorStore';
import apiKeyFunIcon from '../assets/icons/apikey-fun.png';
import './ApiKeyFunPage.css';

type ManagedApiKey = {
  id: string;
  key: string;
  name: string;
  createdAt: number;
  lastUsedAt: number;
  lastStatus?: 'ok' | 'bad' | 'unknown';
  lastRemaining?: string;
  addedToCodexAt?: number;
  codexProviderName?: string;
  addedToClaudeAt?: number;
  claudeAccountName?: string;
};

const APIKEY_FUN_KEYS_STORAGE_KEY = 'apikey_fun_managed_keys';
const APIKEY_FUN_AUTO_QUERY_DELAY_MS = 650;

function maskKey(value: string): string {
  const trimmed = value.trim();
  if (!trimmed) return '';
  if (trimmed.length <= 10) return `${trimmed.slice(0, 3)}****`;
  return `${trimmed.slice(0, 6)}****${trimmed.slice(-4)}`;
}

function formatNumber(value?: number | null, suffix = ''): string {
  if (typeof value !== 'number' || !Number.isFinite(value)) return '--';
  const formatted = new Intl.NumberFormat(undefined, {
    maximumFractionDigits: value >= 100 ? 0 : 4,
  }).format(value);
  return suffix ? `${formatted} ${suffix}` : formatted;
}

function usagePrimaryValue(summary: ModelProviderUsageSummary | null): string {
  if (!summary) return '--';
  const unit = summary.unit ?? '';
  if (summary.quotaUnlimited) return 'Unlimited';
  if (typeof summary.remaining === 'number') return formatNumber(summary.remaining, unit);
  if (typeof summary.quotaRemaining === 'number') return formatNumber(summary.quotaRemaining, unit);
  if (typeof summary.balance === 'number') return formatNumber(summary.balance, unit);
  return '--';
}

function usageValidityTone(summary: ModelProviderUsageSummary | null): 'ok' | 'bad' | 'unknown' {
  if (!summary || typeof summary.isValid !== 'boolean') return 'unknown';
  return summary.isValid ? 'ok' : 'bad';
}

function providerErrorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  return String(error ?? 'UNKNOWN_ERROR');
}

function loadManagedApiKeys(): ManagedApiKey[] {
  try {
    const raw = window.localStorage.getItem(APIKEY_FUN_KEYS_STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((item): item is ManagedApiKey => (
      typeof item?.id === 'string' &&
      typeof item?.key === 'string' &&
      typeof item?.name === 'string' &&
      typeof item?.createdAt === 'number' &&
      typeof item?.lastUsedAt === 'number'
    ));
  } catch {
    return [];
  }
}

function buildManagedKeyName(key: string): string {
  return maskKey(key);
}

function formatManagedKeyTime(timestamp: number): string {
  if (!Number.isFinite(timestamp) || timestamp <= 0) return '--';
  return new Intl.DateTimeFormat(undefined, {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  }).format(new Date(timestamp));
}

function normalizeModelCatalog(values: readonly string[]): string[] {
  const seen = new Set<string>();
  const models: string[] = [];
  values.forEach((value) => {
    const model = value.trim();
    const key = model.toLowerCase();
    if (!model || seen.has(key)) return;
    seen.add(key);
    models.push(model);
  });
  return models;
}

function isClaudeModelId(value: string): boolean {
  const model = value.trim().toLowerCase();
  return model.startsWith('claude-') || model.startsWith('anthropic/claude-');
}

export function ApiKeyFunPage() {
  const { t } = useTranslation();
  const sponsorModule = useSponsorStore((state) => state.state.sponsorModule);
  const [apiKey, setApiKey] = useState('');
  const [showApiKey, setShowApiKey] = useState(false);
  const [usage, setUsage] = useState<ModelProviderUsageSummary | null>(null);
  const [usageError, setUsageError] = useState<string | null>(null);
  const [queryingUsage, setQueryingUsage] = useState(false);
  const [apiKeyModels, setApiKeyModels] = useState<ModelProviderModel[]>([]);
  const [modelsError, setModelsError] = useState<string | null>(null);
  const [queryingModels, setQueryingModels] = useState(false);
  const [saveFlash, setSaveFlash] = useState(false);
  const [managedKeys, setManagedKeys] = useState<ManagedApiKey[]>(() => loadManagedApiKeys());
  const [keyActionState, setKeyActionState] = useState<Record<string, {
    target: ApiKeyFunPrefillTarget;
    status: 'success' | 'error';
    message?: string;
  }>>({});
  const initialManagedKeySelectedRef = useRef(false);

  // 别名编辑状态
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editNameValue, setEditNameValue] = useState('');

  // 复制状态
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [configKey, setConfigKey] = useState<ManagedApiKey | null>(null);
  const [configModels, setConfigModels] = useState<string[]>([]);
  const [configModelsLoading, setConfigModelsLoading] = useState(false);

  const sponsorIntegration = useMemo(
    () => sponsorModule?.sponsors.find((item) => item.id === 'apikey-fun')?.integration ?? null,
    [sponsorModule],
  );
  const providerBaseUrl = useMemo(() => {
    const remoteBaseUrl = sponsorIntegration?.enabled
      ? sponsorIntegration.baseUrl?.trim()
      : '';
    return remoteBaseUrl || buildApiKeyFunProviderBaseUrl(APIKEY_FUN_GLOBAL_ENDPOINT);
  }, [sponsorIntegration]);
  const claudeBaseUrl = useMemo(
    () => providerBaseUrl.replace(/\/+$/, '').replace(/\/v1$/i, ''),
    [providerBaseUrl],
  );
  const maskedApiKey = useMemo(() => maskKey(apiKey), [apiKey]);
  const currentKey = apiKey.trim();
  const currentSavedKey = useMemo(
    () => managedKeys.find((item) => item.key === currentKey),
    [currentKey, managedKeys],
  );
  const apiKeyFunModelCatalog = useMemo(
    () => normalizeModelCatalog(apiKeyModels.map((model) => model.id)),
    [apiKeyModels],
  );
  const modelCatalogText = useMemo(
    () => apiKeyFunModelCatalog.join('\n').toLowerCase(),
    [apiKeyFunModelCatalog],
  );
  const canAddCurrentKeyToCodex = modelCatalogText.includes('gpt');
  const canAddCurrentKeyToClaude = apiKeyFunModelCatalog.some(isClaudeModelId);
  const canShowTargetActions = canAddCurrentKeyToCodex || canAddCurrentKeyToClaude;
  const canPrefillCurrentKey =
    Boolean(currentSavedKey) && !queryingModels && apiKeyFunModelCatalog.length > 0;

  useEffect(() => {
    if (initialManagedKeySelectedRef.current) return;
    initialManagedKeySelectedRef.current = true;
    if (apiKey.trim()) return;
    const firstKey = managedKeys[0]?.key.trim();
    if (firstKey) {
      setApiKey(firstKey);
    }
  }, [apiKey, managedKeys]);

  useEffect(() => {
    window.localStorage.setItem(APIKEY_FUN_KEYS_STORAGE_KEY, JSON.stringify(managedKeys));
  }, [managedKeys]);

  useEffect(() => {
    if (!saveFlash) return undefined;
    const timer = window.setTimeout(() => setSaveFlash(false), 1500);
    return () => window.clearTimeout(timer);
  }, [saveFlash]);

  const openExternal = useCallback((url: string) => {
    try {
      void openUrl(url).catch(() => {
        window.location.href = url;
      });
    } catch {
      window.location.href = url;
    }
  }, []);

  // 自动额度查询
  useEffect(() => {
    const key = apiKey.trim();
    if (!key) {
      setUsage(null);
      setUsageError(null);
      setQueryingUsage(false);
      setApiKeyModels([]);
      setModelsError(null);
      setQueryingModels(false);
      return undefined;
    }

    let cancelled = false;
    setUsageError(null);
    setModelsError(null);
    setApiKeyModels([]);
    setQueryingUsage(true);
    setQueryingModels(true);

    const timer = window.setTimeout(() => {
      void queryModelProviderUsage({
        baseUrl: providerBaseUrl,
        apiKey: key,
        integrationType: 'sub2api',
      })
        .then((nextUsage) => {
          if (cancelled) return;
          const nextStatus = usageValidityTone(nextUsage);
          const nextRemaining = usagePrimaryValue(nextUsage);
          setUsage(nextUsage);
          setManagedKeys((items) => items.map((item) => (
            item.key === key
              ? {
                  ...item,
                  lastUsedAt: Date.now(),
                  lastStatus: nextStatus,
                  lastRemaining: nextRemaining,
                }
              : item
          )));
        })
        .catch((error) => {
          if (cancelled) return;
          setUsage(null);
          setUsageError(
            t('apiKeyFun.error.queryFailed', {
              defaultValue: '额度查询失败：{{error}}',
              error: providerErrorMessage(error),
            }),
          );
          setManagedKeys((items) => items.map((item) => (
            item.key === key
              ? {
                  ...item,
                  lastUsedAt: Date.now(),
                  lastStatus: 'bad',
                  lastRemaining: '--',
                }
              : item
          )));
        })
        .finally(() => {
          if (!cancelled) setQueryingUsage(false);
        });
      void listModelProviderModels({
        baseUrl: providerBaseUrl,
        apiKey: key,
      })
        .then((result) => {
          if (cancelled) return;
          setApiKeyModels(result.models);
          setModelsError(
            result.models.length === 0
              ? t('apiKeyFun.models.emptyFromKey', '当前 API Key 未返回模型列表。')
              : null,
          );
        })
        .catch((error) => {
          if (cancelled) return;
          setApiKeyModels([]);
          setModelsError(
            t('apiKeyFun.models.queryFailed', {
              defaultValue: '模型列表读取失败：{{error}}',
              error: providerErrorMessage(error),
            }),
          );
        })
        .finally(() => {
          if (!cancelled) setQueryingModels(false);
        });
    }, APIKEY_FUN_AUTO_QUERY_DELAY_MS);

    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [apiKey, providerBaseUrl, t]);

  // 保存密钥
  const handleSaveCurrentKey = useCallback(() => {
    const key = apiKey.trim();
    if (!key) {
      setUsageError(t('apiKeyFun.error.missingApiKey', '请输入 API Key。'));
      return;
    }
    const now = Date.now();
    const nextStatus = usageValidityTone(usage);
    const nextRemaining = usagePrimaryValue(usage);
    setManagedKeys((items) => {
      const existing = items.find((item) => item.key === key);
      if (existing) {
        return items.map((item) => (
          item.key === key
            ? {
                ...item,
                name: buildManagedKeyName(key),
                lastUsedAt: now,
                lastStatus: nextStatus,
                lastRemaining: nextRemaining,
              }
            : item
        ));
      }
      return [
        {
          id: `${now}-${Math.random().toString(36).slice(2, 8)}`,
          key,
          name: buildManagedKeyName(key),
          createdAt: now,
          lastUsedAt: now,
          lastStatus: nextStatus,
          lastRemaining: nextRemaining,
        },
        ...items,
      ];
    });
    setUsageError(null);
    setSaveFlash(true);
  }, [apiKey, t, usage]);

  const setManagedKeyAction = useCallback((
    id: string,
    state: { target: ApiKeyFunPrefillTarget; status: 'success' | 'error'; message?: string },
  ) => {
    setKeyActionState((items) => ({ ...items, [id]: state }));
  }, []);

  const handlePrefillTarget = useCallback((
    target: ApiKeyFunPrefillTarget,
    item: ManagedApiKey,
  ) => {
    const key = item.key.trim();
    if (!key) {
      setManagedKeyAction(item.id, {
        target,
        status: 'error',
        message: t('apiKeyFun.error.missingApiKey', '请输入 API Key。'),
      });
      return;
    }
    const page = getApiKeyFunPrefillPage(target);
    const targetName = target === 'codex'
      ? 'Codex'
      : target === 'claude_desktop'
        ? 'Claude'
        : 'Claude CLI';
    window.dispatchEvent(new CustomEvent<typeof page>('app-request-navigate', { detail: page }));
    window.setTimeout(() => {
      dispatchApiKeyFunPrefillEvent({
        target,
        apiKey: key,
        apiKeyName: item.name || buildManagedKeyName(key),
        providerName: 'APIKEY.FUN',
        baseUrl: target === 'codex' ? providerBaseUrl : claudeBaseUrl,
        sourceTag: APIKEY_FUN_SOURCE_TAG,
        modelCatalog: apiKeyFunModelCatalog,
      });
    }, 0);
    setManagedKeyAction(item.id, {
      target,
      status: 'success',
      message: t('apiKeyFun.keyManager.prefillOpened', {
        defaultValue: '已打开 {{target}} 添加弹窗，请确认保存',
        target: targetName,
      }),
    });
  }, [apiKeyFunModelCatalog, claudeBaseUrl, providerBaseUrl, setManagedKeyAction, t]);

  /** 「使用密钥」：按这把 Key 真实返回的模型渲染配置弹窗（只读 + 复制）。 */
  const handleOpenKeyConfig = useCallback(async (item: ManagedApiKey) => {
    setConfigKey(item);
    setConfigModels([]);
    setConfigModelsLoading(true);
    try {
      const { models } = await listModelProviderModels({
        baseUrl: providerBaseUrl,
        apiKey: item.key,
      });
      setConfigModels([...new Set(models.map((model) => model.id.trim()).filter(Boolean))]);
    } catch {
      setConfigModels([]);
    } finally {
      setConfigModelsLoading(false);
    }
  }, [providerBaseUrl]);

  // 切换密钥
  const handleUseManagedKey = useCallback((item: ManagedApiKey) => {
    setApiKey(item.key);
    setUsageError(null);
    setManagedKeys((items) => items.map((nextItem) => (
      nextItem.id === item.id ? { ...nextItem, lastUsedAt: Date.now() } : nextItem
    )));
  }, []);

  // 删除密钥
  const handleDeleteManagedKey = useCallback((id: string) => {
    setManagedKeys((items) => items.filter((item) => item.id !== id));
  }, []);

  // 行内重命名密钥管理
  const handleStartRename = useCallback((item: ManagedApiKey, e: React.MouseEvent) => {
    e.stopPropagation();
    setEditingId(item.id);
    setEditNameValue(item.name);
  }, []);

  const handleSaveRename = useCallback((id: string, e?: React.FormEvent) => {
    e?.preventDefault();
    const trimmed = editNameValue.trim();
    if (!trimmed) return;
    setManagedKeys((items) => items.map((item) => (
      item.id === id ? { ...item, name: trimmed } : item
    )));
    setEditingId(null);
  }, [editNameValue]);

  const handleCancelRename = useCallback(() => {
    setEditingId(null);
  }, []);

  // 复制剪贴板逻辑
  const handleCopyToClipboard = useCallback((text: string, id: string, e?: React.MouseEvent) => {
    e?.stopPropagation();
    if (!text) return;
    navigator.clipboard.writeText(text)
      .then(() => {
        setCopiedId(id);
        setTimeout(() => setCopiedId(null), 2000);
      })
      .catch(() => {
        try {
          const textarea = document.createElement('textarea');
          textarea.value = text;
          textarea.style.position = 'fixed';
          document.body.appendChild(textarea);
          textarea.select();
          document.execCommand('copy');
          document.body.removeChild(textarea);
          setCopiedId(id);
          setTimeout(() => setCopiedId(null), 2000);
        } catch (err) {
          console.error('Failed to copy key', err);
        }
      });
  }, []);

  return (
    <div className="apikey-fun-page">
      <header className="apikey-fun-header-brand">
        <div className="apikey-fun-brand-main">
          <img src={apiKeyFunIcon} alt="" className="apikey-fun-brand-logo" />
          <div className="apikey-fun-brand-text">
            <div className="apikey-fun-eyebrow-container">
              <span className="apikey-fun-eyebrow">{t('apiKeyFun.eyebrow', '中转站')}</span>
            </div>
            <h1>{t('apiKeyFun.title', 'APIKEY.FUN 中转站')}</h1>
            <p>
              {t(
                'apiKeyFun.description',
                'Cockpit 官方合作中转站，为用户提供稳定、开放、高性价比的大模型 API 接入服务。支持 Claude、ChatGPT、Grok、Gemini、智谱 GLM、Kimi、DeepSeek 等主流模型，适合在 Codex、Gemini CLI、Claude Code 及其他开发工具中统一配置使用。通过 Cockpit 专属链接注册，可享受最高充值永久 95 折优惠。',
              )}
            </p>
          </div>
        </div>
        <div className="apikey-fun-brand-actions">
          <button
            className="btn apikey-fun-register-btn"
            onClick={() => openExternal(APIKEY_FUN_REGISTER_URL)}
          >
            <ExternalLink size={15} />
            <span>{t('apiKeyFun.viewNow', '立即查看')}</span>
          </button>
        </div>
      </header>

      <div className="apikey-fun-dashboard-grid">
        <main className="apikey-fun-main-col">
          <section className="apikey-fun-dashboard-panel apikey-fun-config-panel">
            <div className="apikey-fun-panel-head">
              <div>
                <h2>{t('apiKeyFun.queryTitle', '密钥额度查询')}</h2>
              </div>
              <div className="apikey-fun-key-preview">
                <KeyRound size={14} />
                <span>{maskedApiKey || t('apiKeyFun.keyNotSet', '未输入秘钥')}</span>
              </div>
            </div>

            <div className="apikey-fun-form-grid apikey-fun-form-grid-single">
              <label className="apikey-fun-field apikey-fun-field-wide">
                <span>{t('apiKeyFun.apiKeyLabel', 'API Key')}</span>
                <div className="apikey-fun-secret-input">
                  <input
                    value={apiKey}
                    type={showApiKey ? 'text' : 'password'}
                    placeholder={t('apiKeyFun.apiKeyPlaceholder', '粘贴 APIKEY.FUN 控制台创建的 API Key')}
                    onChange={(event) => {
                      setApiKey(event.target.value);
                      setUsageError(null);
                    }}
                  />
                  {apiKey && (
                    <button
                      type="button"
                      className="apikey-fun-icon-button copy-btn"
                      onClick={(e) => handleCopyToClipboard(apiKey, 'input', e)}
                      title={t('apiKeyFun.copyKey', '复制密钥')}
                    >
                      {copiedId === 'input' ? <CheckCircle2 size={16} className="success-icon" /> : <Copy size={16} />}
                    </button>
                  )}
                  {apiKey && (
                    <button
                      type="button"
                      className="apikey-fun-icon-button clear-btn"
                      onClick={() => {
                      setApiKey('');
                      setUsageError(null);
                      setUsage(null);
                      setApiKeyModels([]);
                      setModelsError(null);
                      setQueryingModels(false);
                    }}
                    title={t('apiKeyFun.clearKey', '清空输入')}
                  >
                      <X size={16} />
                    </button>
                  )}
                  <button
                    type="button"
                    className="apikey-fun-icon-button"
                    onClick={() => setShowApiKey((value) => !value)}
                    title={showApiKey ? t('apiKeyFun.hideKey', '隐藏') : t('apiKeyFun.showKey', '显示')}
                  >
                    {showApiKey ? <EyeOff size={16} /> : <Eye size={16} />}
                  </button>
                </div>
              </label>
            </div>

            <div className="apikey-fun-action-row apikey-fun-key-actions">
              <div className="apikey-fun-primary-actions">
                <button className="btn apikey-fun-save-btn" disabled={!currentKey} onClick={handleSaveCurrentKey}>
                  {currentSavedKey ? <CheckCircle2 size={16} /> : <BookmarkPlus size={16} />}
                  <span>
                    {currentSavedKey
                      ? t('apiKeyFun.keyManager.savedButton', '已保存')
                      : t('apiKeyFun.keyManager.saveButton', '保存密钥')}
                  </span>
                </button>
                {saveFlash && (
                  <span className="apikey-fun-save-flash">
                    {t('apiKeyFun.keyManager.saveFlash', '刚刚保存')}
                  </span>
                )}
              </div>
              {canShowTargetActions && (
                <div className="apikey-fun-target-actions">
                  {canAddCurrentKeyToCodex && (
                    <button
                      type="button"
                      className="btn apikey-fun-target-btn"
                      disabled={!canPrefillCurrentKey}
                      onClick={() => {
                        if (currentSavedKey) handlePrefillTarget('codex', currentSavedKey);
                      }}
                    >
                      <span>{t('apiKeyFun.keyManager.addToCodex', '添加到 Codex')}</span>
                    </button>
                  )}
                  {canAddCurrentKeyToClaude && (
                    <button
                      type="button"
                      className="btn apikey-fun-target-btn"
                      disabled={!canPrefillCurrentKey}
                      onClick={() => {
                        if (currentSavedKey) handlePrefillTarget('claude_desktop', currentSavedKey);
                      }}
                    >
                      <span>{t('apiKeyFun.keyManager.addToClaudeDesktop', '添加到 Claude')}</span>
                    </button>
                  )}
                  {canAddCurrentKeyToClaude && (
                    <button
                      type="button"
                      className="btn apikey-fun-target-btn"
                      disabled={!canPrefillCurrentKey}
                      onClick={() => {
                        if (currentSavedKey) handlePrefillTarget('claude_cli', currentSavedKey);
                      }}
                    >
                      <span>{t('apiKeyFun.keyManager.addToClaudeCli', '添加到 Claude CLI')}</span>
                    </button>
                  )}
                </div>
              )}
            </div>

            {usageError && (
              <div className="apikey-fun-message error">
                {usageError}
              </div>
            )}
          </section>

          {/* 数值卡片展示 */}
          <div className="apikey-fun-usage-grid">
            <div className={`apikey-fun-usage-card primary ${queryingUsage ? 'loading' : ''}`}>
              <span>{t('apiKeyFun.usage.remaining', '剩余额度')}</span>
              {queryingUsage ? (
                <div className="apikey-fun-skeleton-text" />
              ) : (
                <strong>{usagePrimaryValue(usage)}</strong>
              )}
            </div>
            <div className={`apikey-fun-usage-card ${queryingUsage ? 'loading' : ''}`}>
              <span>{t('apiKeyFun.usage.used', '已用额度')}</span>
              {queryingUsage ? (
                <div className="apikey-fun-skeleton-text" />
              ) : (
                <strong>{formatNumber(usage?.quotaUsed ?? usage?.totalCost, usage?.unit ?? '')}</strong>
              )}
            </div>
            <div className={`apikey-fun-usage-card ${queryingUsage ? 'loading' : ''}`}>
              <span>{t('apiKeyFun.usage.todayRequests', '今日请求')}</span>
              {queryingUsage ? (
                <div className="apikey-fun-skeleton-text" />
              ) : (
                <strong>{formatNumber(usage?.todayRequests)}</strong>
              )}
            </div>
            <div className={`apikey-fun-usage-card ${queryingUsage ? 'loading' : ''}`}>
              <span>{t('apiKeyFun.usage.todayTokens', '今日 Token')}</span>
              {queryingUsage ? (
                <div className="apikey-fun-skeleton-text" />
              ) : (
                <strong>{formatNumber(usage?.todayTotalTokens)}</strong>
              )}
            </div>
            <div className={`apikey-fun-usage-card ${queryingUsage ? 'loading' : ''}`}>
              <span>{t('apiKeyFun.usage.totalRequests', '总请求')}</span>
              {queryingUsage ? (
                <div className="apikey-fun-skeleton-text" />
              ) : (
                <strong>{formatNumber(usage?.totalRequests)}</strong>
              )}
            </div>
            <div className={`apikey-fun-usage-card ${queryingUsage ? 'loading' : ''}`}>
              <span>{t('apiKeyFun.usage.totalTokens', '总 Token')}</span>
              {queryingUsage ? (
                <div className="apikey-fun-skeleton-text" />
              ) : (
                <strong>{formatNumber(usage?.totalTotalTokens)}</strong>
              )}
            </div>
          </div>

          <section className="apikey-fun-dashboard-panel apikey-fun-models-panel">
            <div className="apikey-fun-panel-head">
              <div>
                <h2>{t('apiKeyFun.models.title', '可用模型')}</h2>
              </div>
              <div className="apikey-fun-model-count">
                {queryingModels
                  ? t('apiKeyFun.models.loading', '读取中')
                  : t('apiKeyFun.models.count', {
                      defaultValue: '{{count}} 个模型',
                      count: apiKeyFunModelCatalog.length,
                    })}
              </div>
            </div>
            {queryingModels ? (
              <div className="apikey-fun-model-empty">
                {t('apiKeyFun.models.loadingDesc', '正在从当前 API Key 读取模型列表...')}
              </div>
            ) : modelsError ? (
              <div className="apikey-fun-model-empty">
                {modelsError}
              </div>
            ) : apiKeyFunModelCatalog.length === 0 ? (
              <div className="apikey-fun-model-empty">
                {currentKey
                  ? t('apiKeyFun.models.emptyFromKey', '当前 API Key 未返回模型列表。')
                  : t('apiKeyFun.models.empty', '输入 API Key 后读取可用模型。')}
              </div>
            ) : (
              <div className="apikey-fun-model-list">
                {apiKeyFunModelCatalog.map((model) => (
                  <span className="apikey-fun-model-chip" key={model}>{model}</span>
                ))}
              </div>
            )}
          </section>
        </main>

        <aside className="apikey-fun-sidebar-col">
          <section className="apikey-fun-dashboard-panel apikey-fun-manager-panel">
            <div className="apikey-fun-panel-head">
              <div>
                <h2>{t('apiKeyFun.keyManager.title', '密钥管理')}</h2>
                <p>{t('apiKeyFun.keyManager.desc', '保存常用 API Key，点击即可切换并自动查询额度。')}</p>
              </div>
            </div>
            {managedKeys.length === 0 ? (
              <div className="apikey-fun-empty-keys">
                <KeyRound size={16} />
                <span>{t('apiKeyFun.keyManager.empty', '暂无保存的密钥。')}</span>
              </div>
            ) : (
              <div className="apikey-fun-key-list">
                {managedKeys.map((item) => {
                  const isEditing = editingId === item.id;
                  const actionState = keyActionState[item.id];
                  return (
                    <div className={`apikey-fun-key-item ${item.key === currentKey ? 'active' : ''} ${isEditing ? 'editing' : ''}`} key={item.id}>
                      {isEditing ? (
                        <form className="apikey-fun-rename-form" onSubmit={(e) => handleSaveRename(item.id, e)}>
                          <input
                            ref={(el) => el?.focus()}
                            className="apikey-fun-rename-input"
                            value={editNameValue}
                            placeholder={t('apiKeyFun.keyManager.renamePlaceholder', '输入新别名...')}
                            onChange={(e) => setEditNameValue(e.target.value)}
                            onBlur={() => handleSaveRename(item.id)}
                            onKeyDown={(e) => {
                              if (e.key === 'Escape') handleCancelRename();
                            }}
                          />
                        </form>
                      ) : (
                        <button className="apikey-fun-key-select" onClick={() => handleUseManagedKey(item)}>
                          <span className="apikey-fun-key-name-row">
                            <span className="name-text">{item.name}</span>
                            <span
                              className="edit-icon-btn"
                              title={t('apiKeyFun.keyManager.editAlias', '修改别名')}
                              onClick={(e) => handleStartRename(item, e)}
                            >
                              <Pencil size={12} />
                            </span>
                          </span>
                          <span className="apikey-fun-key-meta">
                            <small>
                              {item.lastRemaining
                                ? t('apiKeyFun.keyManager.lastRemaining', {
                                    defaultValue: '上次余额 {{value}}',
                                    value: item.lastRemaining,
                                  })
                                : t('apiKeyFun.keyManager.notQueried', '未查询')}
                            </small>
                            <small>
                              {t('apiKeyFun.keyManager.createdAt', {
                                defaultValue: '添加于 {{time}}',
                                time: formatManagedKeyTime(item.createdAt),
                              })}
                            </small>
                            {actionState?.message && (
                              <small className={`apikey-fun-key-action-state ${actionState.status}`}>
                                {actionState.message}
                              </small>
                            )}
                          </span>
                        </button>
                      )}
                      
                      <div className="apikey-fun-key-item-actions">
                        <button
                          type="button"
                          className="apikey-fun-key-copy"
                          title={t('apiKeyFun.keyManager.useKey', '使用密钥')}
                          onClick={(e) => {
                            e.stopPropagation();
                            void handleOpenKeyConfig(item);
                          }}
                        >
                          <Play size={14} />
                        </button>
                        <button
                          type="button"
                          className="apikey-fun-key-copy"
                          onClick={(e) => handleCopyToClipboard(item.key, item.id, e)}
                          title={t('apiKeyFun.copyKey', '复制密钥')}
                        >
                          {copiedId === item.id ? (
                            <CheckCircle2 size={14} className="success-icon" />
                          ) : (
                            <Copy size={14} />
                          )}
                        </button>
                        <button
                          type="button"
                          className="apikey-fun-key-delete"
                          disabled={isEditing}
                          onClick={(e) => {
                            e.stopPropagation();
                            handleDeleteManagedKey(item.id);
                          }}
                          title={t('apiKeyFun.keyManager.deleteButton', '删除')}
                        >
                          <Trash2 size={14} />
                        </button>
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
          </section>
        </aside>
      </div>
      {configKey && (
        <ApiKeyFunKeyConfigModal
          keyName={configKey.name}
          apiKey={configKey.key}
          models={configModels}
          modelsLoading={configModelsLoading}
          onClose={() => setConfigKey(null)}
        />
      )}
    </div>
  );
}
