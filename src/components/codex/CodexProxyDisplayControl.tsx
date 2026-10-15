import { useId, useState } from 'react';
import { ChevronRight, Shield } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useCodexProxyDisplay } from '../../hooks/useCodexProxyDisplay';
import { CODEX_PROXY_DISPLAYS, persistCodexProxyDisplay } from '../../utils/codexProxyDisplay';
import { ModalErrorMessage } from '../ModalErrorMessage';
import '../../styles/pages/codex-proxy-display.css';

/** All entry points save the same immediate, local presentation preference. */
export function CodexProxyDisplayControl({ showSample = true }: { showSample?: boolean }) {
  const { t } = useTranslation();
  const value = useCodexProxyDisplay();
  const [failed, setFailed] = useState(false);
  const id = useId();
  return <section className="codex-proxy-display-control" aria-labelledby={id}>
    <div className="codex-proxy-display-control-heading">
      <strong id={id}>{t('codex.proxy.display.title')}</strong>
      <div className="codex-proxy-display-options" role="group" aria-labelledby={id}>
        {CODEX_PROXY_DISPLAYS.map((mode) => <button type="button" key={mode}
          className={`btn btn-secondary${mode === value ? ' is-selected' : ''}`} aria-pressed={mode === value}
          onClick={() => { setFailed(false); setFailed(!persistCodexProxyDisplay(mode)); }}>
          {t(`codex.proxy.display.${mode}`)}
        </button>)}
      </div>
    </div>
    <p>{t('codex.proxy.display.hint')}</p>
    {showSample && <div className={`codex-proxy-display-sample codex-account-proxy-card is-${value}`} aria-label={t('codex.proxy.display.preview')}>
      {value === 'compact' ? <><Shield size={14} /><span>{t('codex.proxy.action')}</span></> :
        <div className="codex-account-proxy-card-open codex-proxy-display-sample-card">
          <span className="codex-account-proxy-card-emblem"><Shield size={16} aria-hidden="true" /></span>
          <span className="codex-account-proxy-card-heading"><span className="codex-account-proxy-card-mode">{t('codex.proxy.action')}</span></span>
          <ChevronRight className="codex-account-proxy-card-chevron" size={15} aria-hidden="true" />
          <span className="codex-account-proxy-card-nodes"><span className="codex-account-proxy-card-node">
            <span className="codex-account-proxy-card-node-label"><strong>{t('codex.proxy.currentNode')}</strong></span>
            <span className="codex-account-proxy-card-latency">{t('codex.proxy.latencyPending')}</span>
          </span></span>
          {value === 'detailed' && <>
            <span className="codex-account-proxy-card-details">
              <span className="codex-account-proxy-card-channel">
                <span>{t('codex.proxy.sharedRuntime')}</span><strong>—</strong>
              </span>
            </span>
            <span className="codex-account-proxy-card-request"><span className="codex-account-proxy-card-request-heading">
              <span>{t('codex.proxy.display.lastRequest')}</span><small>—</small>
            </span></span>
          </>}
        </div>}
    </div>}
    <ModalErrorMessage message={failed ? t('codex.proxy.display.saveFailed') : null} />
  </section>;
}
