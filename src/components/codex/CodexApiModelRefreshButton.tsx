import { useState } from "react";
import { RefreshCw } from "lucide-react";
import { useTranslation } from "react-i18next";
import { refreshCodexModelData } from "../../services/codexLocalAccessService";

interface Props {
  onNotice: (message: string) => void;
  onError: (message: string) => void;
}

export function CodexApiModelRefreshButton({ onNotice, onError }: Props) {
  const { t } = useTranslation();
  const [refreshing, setRefreshing] = useState(false);

  async function refresh() {
    setRefreshing(true);
    onError("");
    onNotice("");
    try {
      const result = await refreshCodexModelData();
      if (result.modelError && result.priceError) {
        onError(t("codex.apiService.models.refreshFailed"));
      } else if (result.modelError) {
        onError(t("codex.apiService.models.refreshModelsFailed"));
      } else if (result.priceError) {
        onError(t("codex.apiService.models.refreshPricesFailed"));
      } else {
        onNotice(t("codex.apiService.models.refreshComplete"));
      }
    } catch (error) {
      console.warn("[Codex catalog] Refresh failed", error);
      onError(t("codex.apiService.models.refreshFailed"));
    } finally {
      setRefreshing(false);
    }
  }

  return (
    <button type="button" className="btn btn-secondary btn-sm"
      disabled={refreshing} aria-busy={refreshing} onClick={() => void refresh()}>
      <RefreshCw size={14} className={refreshing ? "loading-spinner" : undefined} />
      {t("codex.apiService.models.refreshCatalog")}
    </button>
  );
}
