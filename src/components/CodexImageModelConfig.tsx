import { useEffect, useRef, useState } from "react";
import { RefreshCw } from "lucide-react";
import { useTranslation } from "react-i18next";
import { SingleSelectDropdown } from "./SingleSelectDropdown";
import { ModalErrorMessage } from "./ModalErrorMessage";
import "./CodexLocalAccessModal.css";
import "./CodexImageModelConfig.css";

const PRESETS = ["gpt-image-2.5", "gpt-image-2"];
const CUSTOM = "__custom__";
const keyPrefix = "codex.localAccess.imageGenerationModel";

interface Props {
  model?: string;
  disabled: boolean;
  onSave: (model: string) => Promise<unknown> | unknown;
  mainModel?: string | null;
  onSaveMainModel: (model: string | null) => Promise<unknown> | unknown;
}

export function CodexImageModelSelect({ model, mainModel, disabled, onSave, onSaveMainModel }: Props) {
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
      <CodexImageToolModelSelect model={model} disabled={disabled || pending} onSave={(next) => save(() => onSave(next))} />
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
      <div className="codex-image-model-inline">
        <label className="codex-image-model-inline-label">
          {t(`${key}.label`)}
          <input ref={input} className="codex-image-model-inline-input" value={draft}
            placeholder="gpt-5.5" maxLength={200} disabled={disabled} aria-invalid={Boolean(error)}
            onChange={(event) => { setDraft(event.target.value); setError(""); }}
            onKeyDown={(event) => { if (event.key === "Enter" && !event.nativeEvent.isComposing) { event.preventDefault(); void save(); } }} />
        </label>
        <button type="button" className="btn btn-secondary btn-sm" disabled={disabled || draft.trim() === (model || "")} onClick={() => void save()}>
          {t("common.save")}
        </button>
      </div>
      <small className="codex-image-main-model-hint">{t(`${key}.hint`)}</small>
      <ModalErrorMessage message={error} position="bottom" />
    </div>
  );
}

/**
 * 紧凑版生图模型选择：与「选择生图账号」同排，选中预设立即保存；
 * 选择「自定义」时才展开输入框与保存按钮，避免占用一整个卡片的高度。
 */
function CodexImageToolModelSelect({ model, disabled, onSave }: Omit<Props, "mainModel" | "onSaveMainModel">) {
  const { t } = useTranslation();
  const initialModel = model?.trim() || PRESETS[0];
  const [selection, setSelection] = useState(
    PRESETS.includes(initialModel) ? initialModel : CUSTOM,
  );
  const [customModel, setCustomModel] = useState(
    PRESETS.includes(initialModel) ? "" : initialModel,
  );
  const [customOpen, setCustomOpen] = useState(!PRESETS.includes(initialModel));
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const savingRef = useRef(false);
  const busy = disabled || pending;

  useEffect(() => {
    const next = model?.trim() || PRESETS[0];
    const isPreset = PRESETS.includes(next);
    setSelection(isPreset ? next : CUSTOM);
    setCustomModel(isPreset ? "" : next);
    setCustomOpen(!isPreset);
  }, [model]);

  const save = async (nextModel: string) => {
    if (busy || savingRef.current) return;
    const normalized = nextModel.trim();
    if (!normalized) {
      setError(t(`${keyPrefix}.required`));
      return;
    }
    savingRef.current = true;
    setPending(true);
    setError("");
    try {
      await onSave(normalized);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      savingRef.current = false;
      setPending(false);
    }
  };

  return (
    <div className="codex-image-model-inline">
      <span className="codex-image-model-inline-label">
        {t(`${keyPrefix}.label`)}
      </span>
      <SingleSelectDropdown
        value={selection}
        options={[
          ...PRESETS.map((value) => ({ value, label: value })),
          { value: CUSTOM, label: t(`${keyPrefix}.custom`) },
        ]}
        className="codex-image-model-inline-select"
        menuWidth={190}
        menuMaxHeight={200}
        disabled={busy}
        ariaLabel={t(`${keyPrefix}.label`)}
        onChange={(value) => {
          setError("");
          setSelection(value);
          if (value === CUSTOM) {
            setCustomOpen(true);
            return;
          }
          setCustomOpen(false);
          void save(value);
        }}
      />
      {customOpen && (
        <>
          <input
            className="codex-image-model-inline-input"
            type="text"
            value={customModel}
            maxLength={200}
            placeholder={t(`${keyPrefix}.placeholder`)}
            aria-label={t(`${keyPrefix}.placeholder`)}
            aria-invalid={Boolean(error)}
            disabled={busy}
            onChange={(event) => {
              setCustomModel(event.target.value);
              setError("");
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter" && !event.nativeEvent.isComposing) {
                event.preventDefault();
                void save(customModel);
              }
            }}
          />
          <button
            type="button"
            className="btn btn-secondary btn-sm"
            onClick={() => void save(customModel)}
            disabled={busy || !customModel.trim()}
          >
            {pending && <RefreshCw size={14} className="loading-spinner" />}
            {t(`${keyPrefix}.save`)}
          </button>
        </>
      )}
      <ModalErrorMessage message={error} position="bottom" />
    </div>
  );
}
