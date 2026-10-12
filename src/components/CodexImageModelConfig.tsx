import { useEffect, useId, useRef, useState } from "react";
import { Plus, Settings2, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { SingleSelectDropdown } from "./SingleSelectDropdown";
import { useCodexImageModelCatalog } from "./codex/useCodexImageModelCatalog";
import { ModalErrorMessage } from "./ModalErrorMessage";
import "./CodexLocalAccessModal.css";
import "./CodexImageModelConfig.css";

const PRESETS = ["gpt-image-2.5", "gpt-image-2"];
const CUSTOM = "__custom__";
const keyPrefix = "codex.localAccess.imageGenerationModel";

interface Props {
  model?: string;
  models?: string[];
  deletedModels?: string[];
  catalogUpdatedAt?: number;
  disabled: boolean;
  onSave: (model: string) => Promise<unknown> | unknown;
  mainModel?: string | null;
  onSaveMainModel: (model: string | null) => Promise<unknown> | unknown;
}

export function CodexImageModelSelect({ model, models, deletedModels, catalogUpdatedAt, mainModel, disabled, onSave, onSaveMainModel }: Props) {
  const [pending, setPending] = useState(false);
  const saving = useRef(false);
  const save = async (action: () => Promise<unknown> | unknown) => {
    if (saving.current) return;
    saving.current = true;
    setPending(true);
    try {
      return await action();
    } finally {
      saving.current = false;
      setPending(false);
    }
  };
  return (
    <div className="codex-image-model-config">
      <CodexImageToolModelSelect model={model} models={models} deletedModels={deletedModels} catalogUpdatedAt={catalogUpdatedAt} disabled={disabled || pending} onSave={(next) => save(() => onSave(next))} />
      <CodexImageMainModelInput model={mainModel} disabled={disabled || pending} onSave={(next) => save(() => onSaveMainModel(next))} />
    </div>
  );
}

function CodexImageMainModelInput({ model, disabled, onSave }: {
  model?: string | null;
  disabled: boolean;
  onSave: (model: string | null) => Promise<unknown> | unknown;
}) {
  const { t } = useTranslation();
  const [draft, setDraft] = useState(model || "");
  const [error, setError] = useState("");
  const input = useRef<HTMLInputElement>(null);
  const inputId = useId();
  const hintId = `${inputId}-hint`;
  const key = "codex.localAccess.imageGenerationMainModel";
  useEffect(() => { setDraft(model || ""); setError(""); }, [model]);
  const save = async () => {
    if (disabled) return;
    setError("");
    const next = draft.trim();
    if (next.length > 200 || /\s|[\x00-\x1f\x7f]/u.test(next)) {
      setError(t(`${key}.invalid`));
      input.current?.focus();
      return;
    }
    try {
      await onSave(next || null);
    } catch (cause) {
      const message = cause instanceof Error ? cause.message : String(cause);
      setError(message.startsWith("codex.localAccess.") ? t(message) : message);
    }
  };
  return (
    <div className="codex-image-main-model">
      <label className="codex-image-model-inline-label" htmlFor={inputId}>
        {t(`${key}.label`)}
      </label>
      <div className="codex-image-model-input-row">
        <input id={inputId} ref={input} className="codex-image-model-inline-input" value={draft}
          placeholder="gpt-5.5" maxLength={200} disabled={disabled} aria-invalid={Boolean(error)} aria-describedby={hintId}
          onChange={(event) => { setDraft(event.target.value); setError(""); }}
          onKeyDown={(event) => { if (event.key === "Enter" && !event.nativeEvent.isComposing) { event.preventDefault(); void save(); } }} />
        <button type="button" className="btn btn-secondary btn-sm" disabled={disabled || draft.trim() === (model || "")} onClick={() => void save()}>
          {t("common.save")}
        </button>
      </div>
      <small id={hintId} className="codex-image-main-model-hint">
        <strong>{t(`${key}.notice`)}</strong>{" "}{t(`${key}.hint`)}
      </small>
      <ModalErrorMessage message={error} position="bottom" />
    </div>
  );
}

/** 顶部预设是模型族分类，不受实际型号墓碑影响；完整后缀由账号行选择，自定义全名走 exact。 */
export function CodexImageToolModelSelect({ model, models, deletedModels, catalogUpdatedAt, disabled, onSave }: Omit<Props, "mainModel" | "onSaveMainModel">) {
  const { t } = useTranslation();
  const currentModel = model?.trim() || PRESETS[0];
  const [selection, setSelection] = useState(PRESETS.includes(currentModel) ? currentModel : CUSTOM);
  const [customModel, setCustomModel] = useState(PRESETS.includes(currentModel) ? "" : currentModel);
  const customInput = useRef<HTMLInputElement>(null);
  const [manageOpen, setManageOpen] = useState(false);
  const [draft, setDraft] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const [draftInvalid, setDraftInvalid] = useState<"custom" | "catalog" | null>(null);
  const input = useRef<HTMLInputElement>(null);
  const savingRef = useRef(false);
  const catalog = useCodexImageModelCatalog(currentModel, models, deletedModels, catalogUpdatedAt);
  const busy = disabled || pending;
  useEffect(() => { setSelection(PRESETS.includes(currentModel) ? currentModel : CUSTOM); setCustomModel(PRESETS.includes(currentModel) ? "" : currentModel); }, [currentModel]);
  const message = (cause: unknown) => {
    const text = String(cause).replace(/^Error:\s*/, "");
    return text.startsWith("codex.localAccess.") || text.startsWith("codex.modelConfig.errors.") ? t(text) : text;
  };
  const save = async (next: string, mode: "select" | "custom" | "catalog" = "select") => {
    if (busy || savingRef.current) return;
    const id = next.trim();
    setError("");
    setDraftInvalid(null);
    if (!id || id.length > 200 || /\s|[\x00-\x1f\x7f]/u.test(id)) {
      setError(t(`${keyPrefix}.${!id ? "required" : id.length > 200 ? "tooLong" : "invalid"}`));
      setDraftInvalid(mode === "select" ? null : mode);
      if (mode === "catalog") input.current?.focus();
      if (mode === "custom") customInput.current?.focus();
      return;
    }
    savingRef.current = true;
    setPending(true);
    try {
      if (mode !== "select") await catalog.update(id);
      if (mode !== "catalog") {
        await onSave(id);
        setSelection(PRESETS.includes(id) ? id : CUSTOM);
        if (!PRESETS.includes(id)) setCustomModel(id);
      }
      if (mode === "catalog") setDraft("");
    } catch (cause) {
      setSelection(PRESETS.includes(currentModel) ? currentModel : CUSTOM);
      setError(message(cause));
    } finally {
      savingRef.current = false;
      setPending(false);
    }
  };
  const remove = async (id: string) => {
    if (busy || savingRef.current) return;
    setError("");
    setDraftInvalid(null);
    if (id === currentModel || id === selection) {
      setError(t(`${keyPrefix}.deleteSelected`));
      return;
    }
    savingRef.current = true;
    setPending(true);
    try { await catalog.update(undefined, id); }
    catch (cause) { setError(message(cause)); }
    finally { savingRef.current = false; setPending(false); }
  };
  return (
    <div className="codex-image-model-inline codex-image-tool-model">
      <span className="codex-image-model-inline-label">{t(`${keyPrefix}.label`)}</span>
      <div className="codex-image-model-select-row">
        <SingleSelectDropdown value={selection} options={[...PRESETS.map((id) => ({ value: id, label: id })), { value: CUSTOM, label: t(`${keyPrefix}.custom`) }]}
          className="codex-image-model-inline-select" menuWidth={280} menuMaxHeight={200} disabled={busy}
          ariaLabel={t(`${keyPrefix}.label`)} onChange={(id) => { setError(""); setDraftInvalid(null); setSelection(id); if (id !== CUSTOM) void save(id); }} />
        <button type="button" className="btn btn-secondary btn-sm codex-image-model-manage" disabled={busy}
          aria-label={t(`${keyPrefix}.manage`)} title={t(`${keyPrefix}.manage`)} aria-expanded={manageOpen}
          onClick={() => { setError(""); setManageOpen((open) => !open); }}><Settings2 size={15} /></button>
      </div>
      {selection === CUSTOM && <div className="codex-image-model-custom-row">
        <input ref={customInput} className="codex-image-model-inline-input" value={customModel} maxLength={200} disabled={busy}
          placeholder={t(`${keyPrefix}.placeholder`)} aria-label={t(`${keyPrefix}.placeholder`)} aria-invalid={draftInvalid === "custom"}
          onChange={(event) => { setCustomModel(event.target.value); setError(""); setDraftInvalid(null); }}
          onKeyDown={(event) => { if (event.key === "Enter" && !event.nativeEvent.isComposing) { event.preventDefault(); void save(customModel, "custom"); } }} />
        <button type="button" className="btn btn-secondary btn-sm" disabled={busy || !customModel.trim()} onClick={() => void save(customModel, "custom")}>{t(`${keyPrefix}.save`)}</button>
      </div>}
      {manageOpen && <div className="codex-image-model-manager">
        <div className="codex-image-model-add-row">
          <input ref={input} className="codex-image-model-inline-input" value={draft} maxLength={200} disabled={busy}
            placeholder={t(`${keyPrefix}.placeholder`)} aria-label={t(`${keyPrefix}.placeholder`)} aria-invalid={draftInvalid === "catalog"}
            onChange={(event) => { setDraft(event.target.value); setError(""); setDraftInvalid(null); }}
            onKeyDown={(event) => { if (event.key === "Enter" && !event.nativeEvent.isComposing) { event.preventDefault(); void save(draft, "catalog"); } }} />
          <button type="button" className="btn btn-secondary btn-sm" disabled={busy || !draft.trim()} onClick={() => void save(draft, "catalog")}>
            <Plus size={14} />{t("common.add")}
          </button>
        </div>
        <div className="codex-image-model-managed-list">
          {catalog.models.map((id) => <div key={id} className="codex-image-model-managed-row">
            <span title={id}>{id}</span>
            <button type="button" className="btn btn-secondary btn-sm" disabled={busy} onClick={() => void remove(id)}
              aria-label={`${t("common.delete")}: ${id}`} title={t("common.delete")}><Trash2 size={13} /></button>
          </div>)}
        </div>
      </div>}
      <ModalErrorMessage message={error} position="bottom" />
      {catalog.syncError && <div className="codex-image-model-sync-error">
        <small role="alert">{message(catalog.syncError)}</small>
        <button type="button" className="btn btn-secondary btn-sm" disabled={busy} onClick={catalog.retry}>{t("common.retry")}</button>
      </div>}
    </div>
  );
}
