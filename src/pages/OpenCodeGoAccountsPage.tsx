import { useCallback, useEffect, useRef, useState } from 'react';
import { Clock3, KeyRound, RefreshCw } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { OpenCodeIcon } from '../components/icons/OpenCodeIcon';
import { ModelProviderUsagePanel } from '../components/model-provider/ModelProviderUsagePanel';
import {
  listCodexModelProviders,
  queryCodexModelProviderUsage,
  type CodexModelProvider,
  type CodexModelProviderUsageSummary,
} from '../services/codexModelProviderService';
import {
  classifyOpenCodeGoUsageError,
  findOpenCodeGoProvider,
  maskOpenCodeGoApiKey,
  type OpenCodeGoUsageErrorKind,
} from '../utils/openCodeGoPlatform';

type ConnectionUsageState = {
  loading: boolean;
  summary?: CodexModelProviderUsageSummary;
  error?: OpenCodeGoUsageErrorKind;
  updatedAt?: number;
};

export function OpenCodeGoAccountsPage() {
  const { t } = useTranslation();
  const requestIdRef = useRef(0);
  const [provider, setProvider] = useState<CodexModelProvider | null>(null);
  const [usageByKeyId, setUsageByKeyId] = useState<
    Record<string, ConnectionUsageState>
  >({});
  const [loading, setLoading] = useState(true);
  const [loadFailed, setLoadFailed] = useState(false);

  const errorText = useCallback(
    (kind?: OpenCodeGoUsageErrorKind) => {
      switch (kind) {
        case 'authentication':
          return t(
            'openCodeGo.errors.authentication',
            'Authentication failed. Check this connection key.',
          );
        case 'rate_limit':
          return t(
            'openCodeGo.errors.rateLimit',
            'OpenCode Go is rate limiting usage checks. Try again shortly.',
          );
        case 'network':
          return t(
            'openCodeGo.errors.network',
            'Unable to reach OpenCode Go.',
          );
        default:
          return t(
            'openCodeGo.errors.unavailable',
            'Usage data is unavailable for this connection.',
          );
      }
    },
    [t],
  );

  const refresh = useCallback(async () => {
    const requestId = ++requestIdRef.current;
    setLoading(true);
    setLoadFailed(false);

    try {
      const providers = await listCodexModelProviders();
      const nextProvider = findOpenCodeGoProvider(providers);
      if (requestId !== requestIdRef.current) return;

      setProvider(nextProvider);
      if (!nextProvider) {
        setUsageByKeyId({});
        return;
      }

      setUsageByKeyId(
        Object.fromEntries(
          nextProvider.apiKeys.map((apiKey) => [apiKey.id, { loading: true }]),
        ),
      );

      const usageEntries = await Promise.all(
        nextProvider.apiKeys.map(async (apiKey) => {
          try {
            const summary = await queryCodexModelProviderUsage({
              baseUrl: nextProvider.baseUrl,
              apiKey: apiKey.apiKey,
              integrationType: nextProvider.integrationType,
            });
            return [
              apiKey.id,
              { loading: false, summary, updatedAt: Date.now() },
            ] as const;
          } catch (error) {
            return [
              apiKey.id,
              {
                loading: false,
                error: classifyOpenCodeGoUsageError(error),
                updatedAt: Date.now(),
              },
            ] as const;
          }
        }),
      );

      if (requestId === requestIdRef.current) {
        setUsageByKeyId(Object.fromEntries(usageEntries));
      }
    } catch {
      if (requestId === requestIdRef.current) {
        setProvider(null);
        setUsageByKeyId({});
        setLoadFailed(true);
      }
    } finally {
      if (requestId === requestIdRef.current) {
        setLoading(false);
      }
    }
  }, []);

  useEffect(() => {
    void refresh();
    return () => {
      requestIdRef.current += 1;
    };
  }, [refresh]);

  return (
    <main className="main-content opencode-go-accounts-page fade-in">
      <div className="page-tabs-row opencode-go-header">
        <div className="platform-header-title">
          <span className="opencode-go-title-icon">
            <OpenCodeIcon size={28} />
          </span>
          <div>
            <h1 className="opencode-go-title">OpenCode Go</h1>
            <p>
              {t(
                'openCodeGo.subtitle',
                'Configured connections and 5-hour, weekly, and monthly quota windows.',
              )}
            </p>
          </div>
        </div>
        <button
          type="button"
          className="header-action-btn"
          onClick={() => void refresh()}
          disabled={loading}
          aria-label={t('common.refresh', 'Refresh')}
        >
          <RefreshCw size={15} className={loading ? 'loading-spinner' : ''} />
          <span>{t('common.refresh', 'Refresh')}</span>
        </button>
      </div>

      <section className="opencode-go-summary" aria-live="polite">
        <div>
          <KeyRound size={18} />
          <span>{t('openCodeGo.connections', 'Configured connections')}</span>
          <strong>{provider?.apiKeys.length ?? 0}</strong>
        </div>
        {provider && (
          <code title={provider.baseUrl}>{provider.baseUrl}</code>
        )}
      </section>

      {loadFailed ? (
        <div className="opencode-go-empty" role="alert">
          {t(
            'openCodeGo.errors.configuration',
            'Cockpit could not read the configured OpenCode Go connections.',
          )}
        </div>
      ) : !provider ? (
        <div className="opencode-go-empty">
          {loading
            ? t('common.loading', 'Loading...')
            : t(
                'openCodeGo.emptyProvider',
                'No OpenCode Go provider is configured in Cockpit.',
              )}
        </div>
      ) : provider.apiKeys.length === 0 ? (
        <div className="opencode-go-empty">
          {t(
            'openCodeGo.emptyConnections',
            'The OpenCode Go provider has no configured connection keys.',
          )}
        </div>
      ) : (
        <div className="opencode-go-connection-grid">
          {provider.apiKeys.map((apiKey, index) => {
            const usage = usageByKeyId[apiKey.id];
            const name =
              apiKey.name.trim() ||
              t('openCodeGo.connectionFallback', 'Connection {{index}}', {
                index: index + 1,
              });
            return (
              <article className="opencode-go-connection-card" key={apiKey.id}>
                <header>
                  <div>
                    <span className="opencode-go-connection-icon">
                      <KeyRound size={16} />
                    </span>
                    <div>
                      <h2>{name}</h2>
                      <code>{maskOpenCodeGoApiKey(apiKey.apiKey)}</code>
                    </div>
                  </div>
                  {usage?.updatedAt && (
                    <span className="opencode-go-updated" title={new Date(usage.updatedAt).toLocaleString()}>
                      <Clock3 size={13} />
                      {new Date(usage.updatedAt).toLocaleTimeString()}
                    </span>
                  )}
                </header>
                <ModelProviderUsagePanel
                  summary={usage?.summary}
                  loading={usage?.loading ?? loading}
                  error={usage?.error ? errorText(usage.error) : undefined}
                  className="opencode-go-quota-panel"
                />
              </article>
            );
          })}
        </div>
      )}
    </main>
  );
}
