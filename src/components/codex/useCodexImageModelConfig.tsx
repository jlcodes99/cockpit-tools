import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  getCodexLocalAccessState,
  updateCodexLocalAccessImageGenerationModel,
  updateCodexLocalAccessImageGenerationMainModel,
} from '../../services/codexLocalAccessService';
import { useCodexImageModelCatalog } from "./useCodexImageModelCatalog";
import { CodexImageModelSelect } from '../CodexImageModelConfig';
import { ModalErrorMessage } from '../ModalErrorMessage';

/** 实例网关继承 API 服务的图片模型设置；读取在后台执行，不阻塞启动预览。 */
export function useCodexImageModelConfig(disabled: boolean, enabled = true) {
  const { t } = useTranslation();
  const [model, setModel] = useState<string | null>(null);
  const [catalogUpdatedAt, setCatalogUpdatedAt] = useState<number | undefined>();
  const [models, setModels] = useState<string[] | undefined>();
  const [deletedModels, setDeletedModels] = useState<string[] | undefined>();
  const [mainModel, setMainModel] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const active = useRef(true);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    if (!enabled) return;
    active.current = true;
    let mounted = true;
    setLoading(true);
    setError('');
    const timer = setTimeout(() => {
      if (!mounted) return;
      mounted = false;
      setLoading(false);
      setError(t('codex.localAccess.imageAccounts.loadFailed'));
    }, 10000);
    void getCodexLocalAccessState().then((state) => {
      if (!mounted) return;
      setModel(state.collection?.imageGenerationModel || 'gpt-image-2.5');
      setMainModel(state.collection?.imageGenerationMainModel || null);
      setModels(state.collection?.imageGenerationModels);
      setDeletedModels(state.collection?.imageGenerationDeletedModels);
      setCatalogUpdatedAt(state.collection?.updatedAt);
    }).catch((cause) => {
      if (mounted) setError(String(cause));
    }).finally(() => { clearTimeout(timer); if (mounted) setLoading(false); });
    return () => { clearTimeout(timer); mounted = false; active.current = false; };
  }, [attempt, enabled, t]);
  const loaded = enabled && !loading && !error;
  const registry = useCodexImageModelCatalog(model || undefined, models, deletedModels, catalogUpdatedAt, loaded);
  const control = (
    <>
      {loading ? <p role="status">{t('common.loading')}</p> : null}
      {loaded && <CodexImageModelSelect model={model || undefined} models={models} deletedModels={deletedModels} catalogUpdatedAt={catalogUpdatedAt} mainModel={mainModel} disabled={disabled || loading || Boolean(error)}
        onSave={async (next) => {
          const state = await updateCodexLocalAccessImageGenerationModel(next);
          if (active.current) {
            setModel(state.collection?.imageGenerationModel || next);
            setModels(state.collection?.imageGenerationModels);
            setDeletedModels(state.collection?.imageGenerationDeletedModels);
            setCatalogUpdatedAt(state.collection?.updatedAt);
          }
        }}
        onSaveMainModel={async (next) => {
          const state = await updateCodexLocalAccessImageGenerationMainModel(next);
          if (active.current) setMainModel(state.collection?.imageGenerationMainModel || null);
        }} />}
      <ModalErrorMessage message={error} />
      {error && <button type="button" className="btn btn-secondary btn-sm" onClick={() => setAttempt((value) => value + 1)} disabled={disabled}>{t('common.retry')}</button>}
    </>
  );
  return { model: loaded ? model : null, control, loaded: loaded && registry.ready, loading: loading || (loaded && !registry.ready), error, deletedModels: registry.deleted };
}
