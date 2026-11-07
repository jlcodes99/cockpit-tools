import { useTranslation } from 'react-i18next';
import type { CodexCapabilityFallbackTrace as Trace } from '../../types/codexLocalAccess';
import { resolveCodexAccountReferenceDisplayName } from '../../utils/codexAccountDisplayName';
import { capabilityLabelKey } from '../../utils/codexCapabilityFallback';
import './CodexCapabilityFallback.css';

export function CodexCapabilityFallbackTrace({ trace, accountDisplayNames, maskAccountText }: {
  trace?: Trace | null;
  accountDisplayNames: ReadonlyMap<string, string>;
  maskAccountText: (value?: string | null) => string;
}) {
  const { t } = useTranslation();
  if (!trace) return null;
  const name = (id: string) => maskAccountText(resolveCodexAccountReferenceDisplayName(id, null, accountDisplayNames, t('common.apiKeyAccount')));
  return <div className="codex-capability-fallback-trace">
    <strong>{t('capabilityFallback.traceTitle')}</strong>
    <p>{t(trace.reason === 'preflight' ? 'capabilityFallback.reasonPreflight' : trace.reason === 'upstream_unsupported' ? 'capabilityFallback.reasonUpstream' : 'capabilityFallback.unknown')}</p>
    <dl>
      <dt>{t('capabilityFallback.source')}</dt><dd>{name(trace.fromAccountId)} · <code>{trace.fromModel}</code></dd>
      <dt>{t('capabilityFallback.target')}</dt><dd>{name(trace.toAccountId)} · <code>{trace.toModel}</code></dd>
      <dt>{t('capabilityFallback.required')}</dt><dd>{trace.capabilities.map((id) => t(capabilityLabelKey(id))).join(' · ')}</dd>
    </dl>
  </div>;
}
