import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { RefreshCw } from "lucide-react";
import { listModelProviderModels } from "../../services/modelProviderUsageService";
import { ModalErrorMessage } from "../ModalErrorMessage";

export function CodexProviderModelFetch({ baseUrl, apiKey, wireApi, authScheme, disabled, onModels }: {
  baseUrl: string; apiKey: string; wireApi: string; authScheme: "x-api-key" | "bearer";
  disabled?: boolean; onModels: (models: string[]) => void;
}) {
  const { t } = useTranslation();
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const signature = JSON.stringify([baseUrl, apiKey, wireApi, authScheme, disabled]);
  const current = useRef({ signature, request: 0, mounted: true });
  current.current.signature = signature;
  useEffect(() => { setLoading(false); setError(null); }, [signature]);
  useEffect(() => { current.current.mounted = true; return () => { current.current.mounted = false; }; }, []);
  const fetchModels = async () => {
    const request = ++current.current.request;
    const isCurrent = () => current.current.mounted && current.current.signature === signature && current.current.request === request;
    setLoading(true); setError(null);
    try {
      const result = await listModelProviderModels({ baseUrl: baseUrl.trim(), apiKey: apiKey.trim(), wireApi, authScheme });
      if (!isCurrent()) return;
      const models = [...new Set(result.models.map(model => model.id.trim()).filter(Boolean))];
      if (models.length) onModels(models);
      else setError(t("codex.api.modelCatalog.fetchEmpty"));
    } catch (cause) {
      if (isCurrent()) setError(t("codex.api.modelCatalog.fetchFailed", { error: String(cause).replace(/^Error:\s*/, "") }));
    } finally { if (isCurrent()) setLoading(false); }
  };
  return <>
    <button type="button" className="btn btn-secondary api-model-catalog-fetch" disabled={disabled || loading || !baseUrl.trim() || !apiKey.trim()} onClick={() => void fetchModels()}>
      <RefreshCw size={14} className={loading ? "loading-spinner" : undefined} />
      {t(loading ? "codex.api.modelCatalog.fetching" : "codex.api.modelCatalog.fetch")}
    </button>
    <ModalErrorMessage message={error} />
  </>;
}
