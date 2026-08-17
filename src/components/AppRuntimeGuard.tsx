import React, { ErrorInfo, ReactNode, useEffect, useMemo, useState } from 'react';
import i18n from '../i18n';
import { dismissBootSplash } from '../utils/bootSplash';

type GuardFailureCode = 'render-crash' | 'chunk-load';

type GuardFailure = {
  code: GuardFailureCode;
  message: string;
  detail?: string;
};

type AppRuntimeGuardProps = {
  children: ReactNode;
};

type RenderCrashBoundaryProps = {
  children: ReactNode;
};

type RenderCrashBoundaryState = {
  failure: GuardFailure | null;
};

function normalizeErrorMessage(value: unknown): string {
  if (value instanceof Error) {
    return value.message || value.name || 'error';
  }
  if (typeof value === 'string') {
    return value.trim() || 'error';
  }
  if (value === null || value === undefined) {
    return 'error';
  }
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

function isLikelyChunkLoadFailure(value: string): boolean {
  const normalized = value.toLowerCase();
  return (
    normalized.includes('chunkloaderror') ||
    normalized.includes('loading chunk') ||
    normalized.includes('failed to fetch dynamically imported module') ||
    normalized.includes('importing a module script failed') ||
    normalized.includes('dynamic import')
  );
}

function createFallbackMessage(rawMessage: string): string {
  const action = i18n.t('common.appName', 'Cockpit Tools');
  return i18n.t('messages.actionFailed', {
    action,
    error: rawMessage || 'error',
    defaultValue: '{{action}} failed: {{error}}',
  });
}

function GuardFallback({ failure }: { failure: GuardFailure }) {
  useEffect(() => {
    dismissBootSplash();
  }, []);
  const title = i18n.t('common.failed', 'Failed');
  const refreshLabel = i18n.t('common.refresh', 'Refresh');
  const detailLabel = i18n.t('common.detail', 'Details');
  const message = useMemo(() => createFallbackMessage(failure.message), [failure.message]);
  const detailText = failure.detail?.trim();

  return (
    <div
      style={{
        position: 'fixed',
        inset: 0,
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        padding: 24,
        background: 'var(--bg-primary, #f8fafc)',
        color: 'var(--text-primary, #0f172a)',
      }}
    >
      <div
        style={{
          width: 'min(680px, calc(100vw - 32px))',
          borderRadius: 12,
          border: '1px solid var(--border, rgba(148, 163, 184, 0.28))',
          background: 'var(--bg-card, #ffffff)',
          boxShadow: '0 12px 32px rgba(2, 6, 23, 0.08)',
          padding: 20,
          display: 'flex',
          flexDirection: 'column',
          gap: 12,
        }}
      >
        <div style={{ fontSize: 16, fontWeight: 700 }}>{title}</div>
        <div style={{ fontSize: 13, lineHeight: 1.6, color: 'var(--text-secondary, #475569)' }}>
          {message}
        </div>
        {detailText ? (
          <div
            style={{
              borderRadius: 10,
              border: '1px solid var(--border-light, rgba(148, 163, 184, 0.2))',
              background: 'var(--bg-tertiary, rgba(248, 250, 252, 0.8))',
              padding: 10,
              fontSize: 12,
              lineHeight: 1.5,
              color: 'var(--text-secondary, #475569)',
              whiteSpace: 'pre-wrap',
              wordBreak: 'break-word',
            }}
          >
            <strong>{detailLabel}: </strong>
            {detailText}
          </div>
        ) : null}
        <div>
          <button
            type="button"
            className="btn btn-primary"
            onClick={() => window.location.reload()}
          >
            {refreshLabel}
          </button>
        </div>
      </div>
    </div>
  );
}

class RenderCrashBoundary extends React.Component<RenderCrashBoundaryProps, RenderCrashBoundaryState> {
  state: RenderCrashBoundaryState = {
    failure: null,
  };

  static getDerivedStateFromError(error: Error): RenderCrashBoundaryState {
    return {
      failure: {
        code: 'render-crash',
        message: normalizeErrorMessage(error),
        detail: error?.stack || '',
      },
    };
  }

  componentDidCatch(error: Error, errorInfo: ErrorInfo): void {
    const nextFailure: GuardFailure = {
      code: 'render-crash',
      message: normalizeErrorMessage(error),
      detail: [error?.stack, errorInfo.componentStack].filter(Boolean).join('\n'),
    };
    this.setState({ failure: nextFailure });
    console.error('[AppRuntimeGuard] Render crash captured:', error, errorInfo);
  }

  render() {
    if (this.state.failure) {
      return <GuardFallback failure={this.state.failure} />;
    }
    return this.props.children;
  }
}

export function AppRuntimeGuard({ children }: AppRuntimeGuardProps) {
  const [chunkFailure, setChunkFailure] = useState<GuardFailure | null>(null);

  useEffect(() => {
    const handleWindowError = (event: ErrorEvent) => {
      const text = `${event.message || ''} ${event.error?.message || ''}`.trim();
      if (!isLikelyChunkLoadFailure(text)) {
        return;
      }
      setChunkFailure({
        code: 'chunk-load',
        message: normalizeErrorMessage(event.error || event.message),
        detail: [event.filename, event.error?.stack].filter(Boolean).join('\n'),
      });
    };

    const handleUnhandledRejection = (event: PromiseRejectionEvent) => {
      const text = normalizeErrorMessage(event.reason);
      if (!isLikelyChunkLoadFailure(text)) {
        return;
      }
      setChunkFailure({
        code: 'chunk-load',
        message: text,
        detail: text,
      });
    };

    window.addEventListener('error', handleWindowError);
    window.addEventListener('unhandledrejection', handleUnhandledRejection);
    return () => {
      window.removeEventListener('error', handleWindowError);
      window.removeEventListener('unhandledrejection', handleUnhandledRejection);
    };
  }, []);

  if (chunkFailure) {
    return <GuardFallback failure={chunkFailure} />;
  }

  return <RenderCrashBoundary>{children}</RenderCrashBoundary>;
}

