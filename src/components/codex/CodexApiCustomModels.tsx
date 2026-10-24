import { useEffect, useState } from "react";
import { Check, Plus, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { SingleSelectDropdown } from "../SingleSelectDropdown";
import type { CodexLocalAccessCustomModel } from "../../types/codexLocalAccess";

interface Props {
  models: CodexLocalAccessCustomModel[];
  accounts: { id: string; name: string }[];
  disabled: boolean;
  onSave: (models: CodexLocalAccessCustomModel[]) => Promise<void>;
}

export function CodexApiCustomModels({ models, accounts, disabled, onSave }: Props) {
  const { t } = useTranslation();
  const [drafts, setDrafts] = useState(models);
  const persisted = JSON.stringify(models);
  useEffect(() => { setDrafts(JSON.parse(persisted)); }, [persisted]);
  const update = (index: number, field: keyof CodexLocalAccessCustomModel, value: string) => {
    setDrafts((rows) => rows.map((row, i) => i === index ? { ...row, [field]: value } : row));
  };
  return (
    <section className="codex-api-service-panel codex-api-custom-models">
      <div className="codex-api-service-panel-head">
        <h2>{t("codex.apiService.models.customTitle", "自定义模型")}</h2>
        <div className="codex-api-service-head-actions">
          <button type="button" className="btn btn-secondary btn-sm" disabled={disabled || accounts.length === 0}
            onClick={() => setDrafts((rows) => [...rows, { clientModel: "", accountId: accounts[0]?.id ?? "", upstreamModel: "" }])}>
            <Plus size={14} />{t("common.add", "添加")}
          </button>
          <button type="button" className="btn btn-secondary btn-sm" disabled={disabled}
            onClick={() => void onSave(drafts.map((row) => ({ ...row, upstreamModel: row.upstreamModel.trim() || row.clientModel.trim() })))}>
            <Check size={14} />{t("common.save", "保存")}
          </button>
        </div>
      </div>
      {drafts.map((row, index) => (
        <div className="codex-api-custom-model-row" key={index}>
          <label><span>{t("codex.apiService.accountModelMappings.clientModel", "请求模型")}</span>
            <input value={row.clientModel} disabled={disabled} onChange={(event) => update(index, "clientModel", event.target.value)} />
          </label>
          <label><span>{t("codex.apiService.tabs.accounts", "账号")}</span>
            <SingleSelectDropdown value={row.accountId} disabled={disabled} options={accounts.map((account) => ({ value: account.id, label: account.name }))}
              onChange={(value) => update(index, "accountId", value)} ariaLabel={t("codex.apiService.tabs.accounts", "账号")} />
          </label>
          <label><span>{t("codex.apiService.accountModelMappings.upstreamModel", "上游模型")}</span>
            <input value={row.upstreamModel} placeholder={row.clientModel} disabled={disabled} onChange={(event) => update(index, "upstreamModel", event.target.value)} />
          </label>
          <button type="button" className="folder-icon-btn" disabled={disabled} title={t("common.delete", "删除")}
            aria-label={t("common.delete", "删除")} onClick={() => setDrafts((rows) => rows.filter((_, i) => i !== index))}><Trash2 size={14} /></button>
        </div>
      ))}
    </section>
  );
}
