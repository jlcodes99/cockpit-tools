import { useEffect, useMemo, useRef, useState } from 'react';
import { Crosshair, Loader2, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import type { WorkbuddyAccount } from '../../types/workbuddy';
import { getWorkbuddyAccountDisplayEmail } from '../../types/workbuddy';
import { SingleSelectFilterDropdown } from '../SingleSelectFilterDropdown';
import { useEscCloseTopmost } from '../../hooks/useEscClose';
import {
  fetchWorkbuddyCloudModels, probeWorkbuddyModel,
  type WorkbuddyCloudModel, type WorkbuddyModelProbeResult,
} from '../../services/workbuddyService';
import { clearWorkbuddyProbeRecord, parseProbeResetAt, setWorkbuddyProbeRecord } from '../../utils/workbuddyProbeStatus';

type ProbeState = { kind: 'idle' } | { kind: 'running' }
  | { kind: 'done'; result: WorkbuddyModelProbeResult } | { kind: 'failed'; error: string };

interface Props {
  accounts: WorkbuddyAccount[];
  currentAccountId?: string | null;
  onClose: () => void;
}

export function WorkbuddyModelProbeModal({ accounts, currentAccountId, onClose }: Props) {
  const { t } = useTranslation();
  const dialog = useRef<HTMLDivElement>(null);
  const resultArea = useRef<HTMLDivElement>(null);
  const mounted = useRef(true);
  const inFlight = useRef(false);
  const [accountId, setAccountId] = useState(() =>
    accounts.find((account) => account.id === currentAccountId)?.id ?? accounts[0]?.id ?? '');
  const [model, setModel] = useState('');
  const [keepConversation, setKeepConversation] = useState(false);
  const [state, setState] = useState<ProbeState>({ kind: 'idle' });
  const [cloudModels, setCloudModels] = useState<WorkbuddyCloudModel[]>([]);
  const [catalog, setCatalog] = useState<'loading' | 'loaded' | 'error'>('loading');
  useEscCloseTopmost(true, onClose, dialog);

  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);

  useEffect(() => {
    let cancelled = false;
    setCloudModels([]);
    setCatalog('loading');
    if (!accountId) { setCatalog('error'); return; }
    void fetchWorkbuddyCloudModels(accountId).then((models) => {
      if (cancelled) return;
      setCloudModels(models);
      setCatalog('loaded');
    }).catch(() => { if (!cancelled) setCatalog('error'); });
    return () => { cancelled = true; };
  }, [accountId]);

  useEffect(() => {
    if (state.kind === 'done' || state.kind === 'failed') {
      resultArea.current?.scrollIntoView({ block: 'nearest' });
    }
  }, [state]);

  const options = useMemo(() => cloudModels.map((item) => ({
    value: item.id, label: [item.name ?? item.id, item.credits].filter(Boolean).join(' · '),
  })), [cloudModels]);

  const running = state.kind === 'running';
  const clear = () => setState({ kind: 'idle' });
  const diagnostic = (message: string) => {
    if (message.startsWith('WORKBUDDY_PROBE_')) {
      return t('workbuddy.modelProbe.protocolFailure') + ' (' + message + ')';
    }
    return message;
  };

  const run = async () => {
    if (!accountId || !model.trim() || inFlight.current) return;
    inFlight.current = true;
    setState({ kind: 'running' });
    const selectedModel = model.trim();
    try {
      const result = await probeWorkbuddyModel(accountId, selectedModel, keepConversation);
      if (result.status === 'rate_limited' || result.status === 'error') {
        setWorkbuddyProbeRecord({ accountId, model: selectedModel, status: result.status,
          message: result.message, resetAt: result.status === 'rate_limited' ? parseProbeResetAt(result.message) : null,
          checkedAt: Date.now() });
      } else if (result.status === 'available') {
        clearWorkbuddyProbeRecord(accountId, selectedModel);
      }
      if (mounted.current) setState({ kind: 'done', result });
    } catch (error) {
      if (mounted.current) setState({ kind: 'failed', error: String(error) });
    } finally {
      inFlight.current = false;
    }
  };

  const result = state.kind === 'done' ? state.result : null;
  return (
    <div className="modal-overlay">
      <div ref={dialog} className="modal-content workbuddy-model-probe-modal" role="dialog"
        aria-modal="true" aria-labelledby="workbuddy-probe-title">
        <div className="modal-header">
          <h3 id="workbuddy-probe-title"><Crosshair size={18} /> {t('workbuddy.modelProbe.title')}</h3>
          <button type="button" className="modal-close" onClick={onClose} aria-label={t('common.close')}><X size={18} /></button>
        </div>
        <div className="modal-body workbuddy-model-probe-body">
          <p className="wb-probe-hint">{t('workbuddy.modelProbe.hint')}</p>
          <div className="wb-probe-field">
            <span>{t('workbuddy.modelProbe.account')}</span>
            <SingleSelectFilterDropdown value={accountId}
              options={accounts.map((account) => ({ value: account.id, label: getWorkbuddyAccountDisplayEmail(account) }))}
              ariaLabel={t('workbuddy.modelProbe.account')} disabled={running}
              onChange={(id) => { clear(); setAccountId(id); }} />
          </div>
          <div className="wb-probe-field">
            <label htmlFor="workbuddy-probe-model">{t('workbuddy.modelProbe.model')}</label>
            <SingleSelectFilterDropdown value={model} options={options}
              ariaLabel={t('workbuddy.modelProbe.catalog')} placeholder={t('workbuddy.modelProbe.catalog')}
              disabled={running} onChange={(value) => { clear(); setModel(value); }} />
            <input id="workbuddy-probe-model" className="wb-probe-input" value={model} disabled={running}
              placeholder={t('workbuddy.modelProbe.modelPlaceholder')}
              onChange={(event) => { clear(); setModel(event.target.value); }} />
            <span className="wb-probe-cloud-note">{t(`workbuddy.modelProbe.catalog_${catalog}`)}</span>
          </div>
          <label className="wb-probe-checkbox">
            <input type="checkbox" checked={keepConversation} disabled={running}
              onChange={(event) => { clear(); setKeepConversation(event.target.checked); }} />
            <span>{t('workbuddy.modelProbe.keep')}</span>
          </label>
          {(state.kind === 'failed' || result) && (
            <div ref={resultArea} className="wb-probe-result" role={state.kind === 'failed' || result?.status !== 'available' ? 'alert' : 'status'}>
              {state.kind === 'failed' && <span>{t('workbuddy.modelProbe.failed')} {diagnostic(state.error)}</span>}
              {result && <>
                <strong>{t(`workbuddy.modelProbe.status_${result.status}`)}</strong>
                {result.message && <span>{diagnostic(result.message)}</span>}
                {result.reply && <pre>{result.reply}</pre>}
                {result.conversationId && <span>
                  {t(result.conversationDeleted ? 'workbuddy.modelProbe.deleted' : 'workbuddy.modelProbe.retained')}
                </span>}
                {result.cleanupError && <span>{t('workbuddy.modelProbe.cleanupFailed')} {diagnostic(result.cleanupError)}</span>}
              </>}
            </div>
          )}
        </div>
        <div className="modal-footer">
          <button type="button" className="btn btn-secondary" onClick={onClose}>{t('common.close')}</button>
          <button type="button" className="btn btn-primary" disabled={!accountId || !model.trim() || running} onClick={() => { void run(); }}>
            {running && <Loader2 size={14} className="animate-spin" />}
            {t(running ? 'workbuddy.modelProbe.running' : 'workbuddy.modelProbe.run')}
          </button>
        </div>
      </div>
    </div>
  );
}
