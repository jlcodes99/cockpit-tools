import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { cleanupExpiredPelican, clearAllPelican, deletePelican, getPelican, historyPelican, retentionSettingsPelican, updateRetentionDaysPelican } from '../../../services/codexPelicanService';
import { useCodexPelicanStore } from '../../../stores/useCodexPelicanStore';
import { isPelicanRunning, type CodexPelicanBatch } from '../../../types/codexPelican';
import { ModalErrorMessage, useModalErrorState } from '../../ModalErrorMessage';
import { PelicanConfirm } from './PelicanConfirm';
import { pelicanError } from './pelicanUtils';

export function PelicanHistory() {
  const { t } = useTranslation();
  const [items, setItems] = useState<CodexPelicanBatch[]>([]);
  const [hasMore, setHasMore] = useState(false);
  const [loading, setLoading] = useState(false);
  const [deleting, setDeleting] = useState<CodexPelicanBatch | null>(null);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const [clearConfirm, setClearConfirm] = useState(false);
  const [retentionDays, setRetentionDays] = useState(7);
  const [retentionInput, setRetentionInput] = useState('7');
  const [retentionBusy, setRetentionBusy] = useState(false);
  const [clearBusy, setClearBusy] = useState(false);
  const [clearError, setClearError] = useState<string | null>(null);
  const error = useModalErrorState();
  const load = useCallback(async (offset: number) => {
    setLoading(true); error.clear();
    try {
      const page = await historyPelican(offset);
      setItems((previous) => offset ? [...previous, ...page.items.filter((entry) => !previous.some((existing) => existing.id === entry.id))] : page.items);
      setHasMore(page.hasMore);
    } catch (cause) { error.set(pelicanError(cause, t)); }
    finally { setLoading(false); }
  }, [error.clear, error.set, t]);
  useEffect(() => { void load(0); }, [load]);
  useEffect(() => { void retentionSettingsPelican().then((settings) => { setRetentionDays(settings.days); setRetentionInput(String(settings.days)); }).catch((cause) => error.set(pelicanError(cause, t))); }, [error.set, t]);

  const saveRetention = async () => {
    const days = Number(retentionInput);
    if (!Number.isInteger(days) || days < 1 || days > 3650) { error.set(t('pelican.error.invalidRetention')); return; }
    setRetentionBusy(true); error.clear();
    try { const settings = await updateRetentionDaysPelican(days); setRetentionDays(settings.days); setRetentionInput(String(settings.days)); await cleanupExpiredPelican(); await load(0); }
    catch (cause) { error.set(pelicanError(cause, t)); }
    finally { setRetentionBusy(false); }
  };

  return <>
    <div className="pelican-section-heading"><h3>{t('pelican.history')}</h3><div className="pelican-actions"><button className="btn btn-secondary" disabled={loading || retentionBusy} onClick={() => void load(0)}>{t('common.refresh')}</button><button className="btn btn-danger" disabled={loading || retentionBusy} onClick={() => { setClearError(null); setClearConfirm(true); }}>{t('pelican.clearAll')}</button></div></div>
    <ModalErrorMessage message={error.message} scrollKey={error.scrollKey} />
    <section className="pelican-retention-card"><div><strong>{t('pelican.retention')}</strong><p className="pelican-muted">{t('pelican.retentionHelp')}</p></div><div className="pelican-retention-controls"><input type="number" min={1} max={3650} value={retentionInput} disabled={retentionBusy} aria-label={t('pelican.retention')} onChange={(event) => setRetentionInput(event.target.value)} /><span className="pelican-muted">{t('pelican.days')}</span><button className="btn btn-secondary" disabled={retentionBusy || retentionInput === String(retentionDays)} onClick={() => void saveRetention()}>{t('common.save')}</button></div></section>
    {loading && items.length > 0 && <p className="pelican-muted" role="status">{t('common.loading')}</p>}
    {!items.length && <p className="pelican-muted">{t(loading ? 'common.loading' : 'pelican.historyEmpty')}</p>}
    {items.map((batch) => <article key={batch.id} className="pelican-history-row">
      <div><strong>{new Date(batch.createdAt).toLocaleString()}</strong><p className="pelican-muted">{batch.model} · {batch.effort} · {batch.items.length} · {t(`pelican.${batch.status}`)}</p></div>
      <div className="pelican-actions"><button className="btn btn-secondary" disabled={loading} onClick={async () => {
        error.clear(); setLoading(true);
        try { useCodexPelicanStore.getState().showBatch(await getPelican(batch.id)); }
        catch (cause) { error.set(pelicanError(cause, t)); }
        finally { setLoading(false); }
      }}>{t('common.open')}</button>
        <button className="btn btn-danger" disabled={loading || isPelicanRunning(batch)} onClick={() => { error.clear(); setDeleteError(null); setDeleting(batch); }}>{t('common.delete')}</button></div>
    </article>)}
    {hasMore && <button className="btn btn-secondary" disabled={loading} onClick={() => void load(items.length)}>{t(loading ? 'common.loading' : 'pelican.loadMore')}</button>}
    {deleting && <PelicanConfirm title={t('pelican.confirmDelete')} description={t('pelican.deleteHelp')} busy={loading} error={deleteError} confirmLabel={t('common.delete')}
      onCancel={() => { setDeleting(null); setDeleteError(null); }} onConfirm={() => void (async () => {
        setLoading(true); setDeleteError(null);
        try {
          await deletePelican(deleting.id);
          const store = useCodexPelicanStore.getState();
          useCodexPelicanStore.setState({ dismissedIds: new Set([...store.dismissedIds, deleting.id]),
            ...(store.active?.id === deleting.id ? { active: null, batch: null } : {}) });
          setDeleting(null); await load(0);
        } catch (cause) { setDeleteError(pelicanError(cause, t)); }
        finally { setLoading(false); }
      })()} />}
    {clearConfirm && <PelicanConfirm title={t('pelican.confirmClearAll')} description={t('pelican.clearAllHelp')} busy={clearBusy} error={clearError} confirmLabel={t('pelican.clearAll')}
      onCancel={() => { if (!clearBusy) { setClearConfirm(false); setClearError(null); } }} onConfirm={() => void (async () => {
        setClearBusy(true); setClearError(null);
        try { await clearAllPelican(); setItems([]); setHasMore(false); useCodexPelicanStore.setState({ active: null, batch: null }); setClearConfirm(false); }
        catch (cause) { setClearError(pelicanError(cause, t)); }
        finally { setClearBusy(false); }
      })()} />}
  </>;
}
