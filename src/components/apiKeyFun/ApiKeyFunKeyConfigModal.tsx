import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Check, Copy, RefreshCw, X } from 'lucide-react';
import { SingleSelectDropdown } from '../SingleSelectDropdown';
import {
  APIKEY_FUN_CLIENT_LABELS,
  APIKEY_FUN_FAMILIES,
  buildApiKeyFunClientPreview,
  detectApiKeyFunPlatform,
  resolveApiKeyFunFamily,
  resolveDefaultModel,
  type ApiKeyFunClientId,
  type ApiKeyFunFamily,
  type ApiKeyFunFamilyPreset,
} from '../../utils/apikeyFunClientConfigs';
import './ApiKeyFunKeyConfigModal.css';

export interface ApiKeyFunKeyConfigModalProps {
  keyName: string;
  apiKey: string;
  models: readonly string[];
  modelsLoading: boolean;
  onClose: () => void;
}

const ALL_CLIENTS: ApiKeyFunClientId[] = [
  'codex_app',
  'codex_cli',
  'claude_code',
  'claude_desktop',
];

/** 中转站「使用密钥」弹窗：按分组渲染客户端配置，支持一键写入本机。 */
export function ApiKeyFunKeyConfigModal({
  keyName,
  apiKey,
  models,
  modelsLoading,
  onClose,
}: ApiKeyFunKeyConfigModalProps) {
  const { t } = useTranslation();
  const platform = useMemo(() => detectApiKeyFunPlatform(), []);
  const detectedFamily = useMemo<ApiKeyFunFamilyPreset | null>(
    () => resolveApiKeyFunFamily(models),
    [models],
  );
  const [familyId, setFamilyId] = useState<ApiKeyFunFamily | ''>('');
  const family = useMemo<ApiKeyFunFamilyPreset | null>(() => {
    if (familyId) {
      return APIKEY_FUN_FAMILIES.find((item) => item.id === familyId) ?? null;
    }
    return detectedFamily;
  }, [detectedFamily, familyId]);
  const availableClients = useMemo<ApiKeyFunClientId[]>(
    () => (family ? [...family.clients] : ALL_CLIENTS),
    [family],
  );
  const [client, setClient] = useState<ApiKeyFunClientId>('codex_app');
  useEffect(() => {
    if (availableClients.length === 0) return;
    if (!availableClients.includes(client)) {
      setClient(availableClients[0]);
    }
  }, [availableClients, client]);

  const [model, setModel] = useState('');
  useEffect(() => {
    if (!family) return;
    setModel(resolveDefaultModel(family, models));
  }, [family, models]);

  const [copiedLabel, setCopiedLabel] = useState<string | null>(null);

  const preview = useMemo(() => {
    if (!family) return null;
    return buildApiKeyFunClientPreview({
      family,
      client,
      apiKey,
      model: model || family.fallbackModel,
      platform,
    });
  }, [apiKey, client, family, model, platform]);

  const modelOptions = useMemo(
    () =>
      [...models]
        .map((item) => item.trim())
        .filter(Boolean)
        .map((item) => ({ value: item, label: item })),
    [models],
  );

  const handleCopy = async (label: string, text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopiedLabel(label);
      window.setTimeout(() => setCopiedLabel((prev) => (prev === label ? null : prev)), 1600);
    } catch {
      setCopiedLabel(null);
    }
  };

  return (
    <div className="modal-overlay apikey-fun-config-overlay" role="presentation">
      <div
        className="modal-content apikey-fun-config-modal"
        role="dialog"
        aria-modal="true"
        aria-label={keyName || t('apiKeyFun.config.title', '使用 API 密钥')}
      >
        <header className="apikey-fun-config-head">
          <div>
            <h2>{t('apiKeyFun.config.title', '使用 API 密钥')}</h2>
          </div>
          <button type="button" className="icon-button" onClick={onClose} aria-label={t('common.close', '关闭')}>
            <X size={16} />
          </button>
        </header>

        <div className="apikey-fun-config-body">
          {modelsLoading && !family ? (
            <div className="apikey-fun-config-loading">
              <RefreshCw size={18} className="loading-spinner" />
              <strong>{t('apiKeyFun.models.loading', '读取中')}</strong>
              <span>{t('apiKeyFun.models.loadingDesc', '正在从当前 API Key 读取模型列表...')}</span>
            </div>
          ) : (
          <>
          <div className="apikey-fun-config-meta">
            <span className="apikey-fun-config-inline-field">
              {detectedFamily ? (
                <span className="apikey-fun-config-detected">
                  {detectedFamily.label}
                </span>
              ) : (
                <SingleSelectDropdown
                  className="apikey-fun-config-select"
                  value={familyId}
                  options={APIKEY_FUN_FAMILIES.map((item) => ({ value: item.id, label: item.label }))}
                  placeholder={t('apiKeyFun.config.family', '分组')}
                  onChange={(value) => setFamilyId(value as ApiKeyFunFamily)}
                />
              )}
            </span>
            <span className="apikey-fun-config-divider" aria-hidden="true" />
            <span className="apikey-fun-config-inline-field">
              <span className="apikey-fun-config-inline-label">
                {t('apiKeyFun.models.title', '可用模型')}
              </span>
              {modelOptions.length > 0 ? (
                <SingleSelectDropdown
                  className="apikey-fun-config-select"
                  value={model}
                  options={modelOptions}
                  placeholder={t('apiKeyFun.models.title', '可用模型')}
                  onChange={setModel}
                />
              ) : (
                <span className="apikey-fun-config-detected">
                  {modelsLoading
                    ? t('apiKeyFun.models.loading', '读取中')
                    : family?.fallbackModel ?? '-'}
                </span>
              )}
            </span>
          </div>

          {family && availableClients.length === 0 ? (
            <div className="apikey-fun-config-empty">
              {t('apiKeyFun.config.unsupportedGroup', '该分组暂时没有可一键写入的客户端。')}
            </div>
          ) : (
            <>
              <nav className="apikey-fun-config-clients" aria-label={t('apiKeyFun.toolsTitle', '一键写入客户端')}>
                {availableClients.map((item) => (
                  <button
                    key={item}
                    type="button"
                    className={`apikey-fun-config-client ${client === item ? 'active' : ''}`}
                    onClick={() => setClient(item)}
                  >
                    {APIKEY_FUN_CLIENT_LABELS[item]}
                  </button>
                ))}
              </nav>

              <div className="apikey-fun-config-blocks">
                {preview?.blocks.map((block) => (
                  <section className="apikey-fun-config-block" key={block.label}>
                    <header>
                      <span>{block.label}</span>
                      <button
                        type="button"
                        onClick={() => void handleCopy(block.label, block.text)}
                      >
                        {copiedLabel === block.label ? <Check size={13} /> : <Copy size={13} />}
                        {copiedLabel === block.label
                          ? t('common.copied', '已复制')
                          : t('common.copy', '复制')}
                      </button>
                    </header>
                    <pre className={`apikey-fun-config-code language-${block.language}`}>{block.text}</pre>
                  </section>
                ))}
              </div>
            </>
          )}
          </>
          )}
        </div>

        <footer className="apikey-fun-config-foot">
          <button type="button" className="btn btn-secondary" onClick={onClose}>
            {t('common.close', '关闭')}
          </button>
</footer>
      </div>
    </div>
  );
}
