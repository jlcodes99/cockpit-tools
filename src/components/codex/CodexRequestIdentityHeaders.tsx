import { Activity, Copy } from 'lucide-react';
import type { TFunction } from 'i18next';
import type { CodexLocalAccessRequestDetail } from '../../types/codexLocalAccess';
import { resolveCodexAccountReferenceDisplayName } from '../../utils/codexAccountDisplayName';

type HeaderPresentation = { state: 'value'; value: string } | { state: 'notProvided' | 'notCaptured' };

export function requestIdentityHeader(captured?: boolean, value?: string | null): HeaderPresentation {
  if (captured !== true) return { state: 'notCaptured' };
  return typeof value === 'string' && value.length > 0
    ? { state: 'value', value }
    : { state: 'notProvided' };
}

export function CodexRequestIdentityHeaders({ detail, t, accountDisplayNames, maskAccountText, onCopy }: {
  detail: CodexLocalAccessRequestDetail;
  t: TFunction;
  accountDisplayNames: ReadonlyMap<string, string>;
  maskAccountText: (value?: string | null) => string;
  onCopy: (value: string) => Promise<void>;
}) {
  const fields = (entries: [string, HeaderPresentation][]) => (
    <dl className="codex-request-identity-fields">
      {entries.map(([header, presentation]) => {
        return <div className="codex-request-identity-field" key={header}>
          <dt><code>{header}</code></dt>
          <dd>
            {presentation.state === 'value' ? <>
              <code className="codex-request-identity-value" dir="ltr">{presentation.value}</code>
              <button type="button" className="btn btn-secondary btn-sm codex-request-identity-copy"
                aria-label={t('codex.requestDiagnostics.identity.copyHeader', { header })}
                title={t('codex.requestDiagnostics.identity.copyHeader', { header })}
                onClick={() => void onCopy(presentation.value)}><Copy size={13} aria-hidden="true" /></button>
            </> : <span className="codex-request-identity-empty">{t(presentation.state === 'notCaptured'
              ? 'codex.requestDiagnostics.identity.notCaptured' : 'codex.requestDiagnostics.identity.notProvided')}</span>}
          </dd>
        </div>;
      })}
    </dl>
  );
  const headers = (captured?: boolean, sessionId?: string | null, threadId?: string | null,
    metadataCaptured?: boolean, installationId?: string | null, accountId?: string | null) => fields([
    ['Session-Id', requestIdentityHeader(captured, sessionId)],
    ['Thread-Id', requestIdentityHeader(captured, threadId)],
    ['X-Codex-Installation-Id', requestIdentityHeader(metadataCaptured, installationId)],
    ['Chatgpt-Account-Id', requestIdentityHeader(metadataCaptured, accountId)],
  ]);
  return <section className="codex-request-detail-section codex-request-identity" aria-labelledby="codex-request-identity-title">
    <h4 id="codex-request-identity-title"><Activity size={16} aria-hidden="true" />{t('codex.requestDiagnostics.identity.title')}</h4>
    <p className="codex-request-diagnostics-hint">{t('codex.requestDiagnostics.identity.hint')}</p>
    <div className="codex-request-identity-card">
      <h5>{t('codex.requestDiagnostics.identity.client')}</h5>
      {headers(detail.clientIdentityCaptured, detail.clientSessionId, detail.clientThreadId,
        detail.clientIdentityMetadataCaptured, detail.clientInstallationId, detail.clientChatgptAccountId)}
    </div>
    {detail.attempts.length === 0 ? <p className="codex-request-diagnostics-hint">{t('codex.requestDiagnostics.identity.notSent')}</p>
      : detail.attempts.map((attempt) => <div className="codex-request-identity-card" key={attempt.sequence}>
        <div className="codex-request-identity-heading">
          <h5>{t('codex.requestDiagnostics.identity.upstreamAttempt', { sequence: attempt.sequence })}</h5>
          <span>{maskAccountText(resolveCodexAccountReferenceDisplayName(attempt.accountId, attempt.accountEmail, accountDisplayNames, t('common.apiKeyAccount')))}</span>
          <code className="codex-request-transport">{attempt.transport}</code>
        </div>
        {headers(attempt.upstreamIdentityCaptured, attempt.upstreamSessionId, attempt.upstreamThreadId,
          attempt.upstreamIdentityMetadataCaptured, attempt.upstreamInstallationId, attempt.upstreamChatgptAccountId)}
        {attempt.accountIdentity && <div className="codex-request-identity-profile">
          <h5>{t('codex.requestDiagnostics.identity.profileTitle')}</h5>
          <p className="codex-request-diagnostics-hint">{t('codex.requestDiagnostics.identity.profileHint')}</p>
          {fields([
            [t('codex.requestDiagnostics.identity.profileInstallationId'), requestIdentityHeader(true, attempt.accountIdentity.installationId)],
            [t('codex.requestDiagnostics.identity.profileScope'), requestIdentityHeader(true, attempt.accountIdentity.scope)],
            [t('codex.requestDiagnostics.identity.profileVersion'), requestIdentityHeader(true, String(attempt.accountIdentity.version))],
            [t('codex.requestDiagnostics.identity.profileRevision'), requestIdentityHeader(true, String(attempt.accountIdentity.revision))],
          ])}
        </div>}
      </div>)}
  </section>;
}
