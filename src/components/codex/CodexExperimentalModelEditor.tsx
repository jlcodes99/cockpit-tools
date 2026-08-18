import { Plus, Star, Trash2, X } from 'lucide-react';
import { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type {
  CodexExperimentalModelDefinition,
  CodexReasoningEffort,
} from '../../types/codex';
import './CodexExperimentalModelEditor.css';

interface CodexExperimentalModelEditorProps {
  models: CodexExperimentalModelDefinition[];
  defaultModelId?: string | null;
  disabled?: boolean;
  mode?: 'inline' | 'summary';
  onChange: (models: CodexExperimentalModelDefinition[]) => void;
  onDefaultModelChange?: (modelId: string | null) => void;
  onValidationChange?: (error: string | null) => void;
}

const MODEL_ID_PATTERN = /^[A-Za-z0-9._:/-]+$/;
const REASONING_EFFORT_OPTIONS: CodexReasoningEffort[] = [
  'low',
  'medium',
  'high',
  'xhigh',
];

export function validateCodexExperimentalModels(
  models: CodexExperimentalModelDefinition[],
  translate: (key: string, fallback: string) => string,
): string | null {
  if (models.length === 0) {
    return translate(
      'codex.experimentalModelCatalog.models.validation.required',
      '至少保留一个模型。',
    );
  }
  const seen = new Set<string>();
  for (const model of models) {
    const modelId = model.model_id.trim();
    if (!modelId || modelId.length > 128 || !MODEL_ID_PATTERN.test(modelId)) {
      return translate(
        'codex.experimentalModelCatalog.models.validation.modelId',
        '模型 ID 只能包含字母、数字、点、横线、下划线、斜杠和冒号。',
      );
    }
    if (!model.display_name.trim() || model.display_name.trim().length > 100) {
      return translate(
        'codex.experimentalModelCatalog.models.validation.displayName',
        '请输入不超过 100 个字符的展示名。',
      );
    }
    const key = modelId.toLowerCase();
    if (seen.has(key)) {
      return translate(
        'codex.experimentalModelCatalog.models.validation.duplicate',
        '模型 ID 不能重复。',
      );
    }
    seen.add(key);
  }
  return null;
}

function nextModelDefinition(
  models: CodexExperimentalModelDefinition[],
): CodexExperimentalModelDefinition {
  const existing = new Set(models.map((model) => model.model_id.trim().toLowerCase()));
  let suffix = 1;
  while (existing.has(`custom-model${suffix}`)) suffix += 1;
  return {
    model_id: `custom-model${suffix}`,
    display_name: `Custom Model ${suffix}`,
  };
}

export function CodexExperimentalModelEditor({
  models,
  defaultModelId = null,
  disabled = false,
  mode = 'inline',
  onChange,
  onDefaultModelChange,
  onValidationChange,
}: CodexExperimentalModelEditorProps) {
  const { t } = useTranslation();
  const [openReasoningIndex, setOpenReasoningIndex] = useState<number | null>(null);
  const [managerOpen, setManagerOpen] = useState(false);
  const reasoningPickerRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (openReasoningIndex === null) return;
    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (target instanceof Node && !reasoningPickerRef.current?.contains(target)) {
        setOpenReasoningIndex(null);
      }
    };
    document.addEventListener('pointerdown', handlePointerDown);
    return () => document.removeEventListener('pointerdown', handlePointerDown);
  }, [openReasoningIndex]);
  const rowErrors = useMemo(() => {
    const counts = new Map<string, number>();
    models.forEach((model) => {
      const key = model.model_id.trim().toLowerCase();
      if (key) counts.set(key, (counts.get(key) ?? 0) + 1);
    });
    return models.map((model) => {
      const modelId = model.model_id.trim();
      return {
        modelId:
          !modelId || modelId.length > 128 || !MODEL_ID_PATTERN.test(modelId)
            ? t(
                'codex.experimentalModelCatalog.models.validation.modelId',
                '模型 ID 只能包含字母、数字、点、横线、下划线、斜杠和冒号。',
              )
            : counts.get(modelId.toLowerCase())! > 1
              ? t(
                  'codex.experimentalModelCatalog.models.validation.duplicate',
                  '模型 ID 不能重复。',
                )
              : null,
        displayName:
          !model.display_name.trim() || model.display_name.trim().length > 100
            ? t(
                'codex.experimentalModelCatalog.models.validation.displayName',
                '请输入不超过 100 个字符的展示名。',
              )
            : null,
      };
    });
  }, [models, t]);

  const validationError = validateCodexExperimentalModels(
    models,
    (key, fallback) => t(key, fallback),
  );
  useEffect(() => {
    onValidationChange?.(validationError);
  }, [onValidationChange, validationError]);

  useEffect(() => {
    if (!managerOpen) return;
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setManagerOpen(false);
    };
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [managerOpen]);

  const updateModel = (
    index: number,
    field: keyof CodexExperimentalModelDefinition,
    value: string,
  ) => {
    const previous = models[index];
    onChange(
      models.map((model, modelIndex) =>
        modelIndex === index ? { ...model, [field]: value } : model,
      ),
    );
    if (field === 'model_id' && defaultModelId === previous?.model_id) {
      onDefaultModelChange?.(value.trim() || null);
    }
  };

  const reasoningLabel = (effort: CodexReasoningEffort) =>
    t(`codex.wakeup.reasoningEfforts.${effort}`, effort.toUpperCase());

  const updateReasoningEfforts = (
    index: number,
    effort: CodexReasoningEffort | 'official',
  ) => {
    onChange(
      models.map((model, modelIndex) => {
        if (modelIndex !== index) return model;
        if (effort === 'official') {
          return { ...model, reasoning_efforts: undefined };
        }
        const current = model.reasoning_efforts ?? [];
        if (current.includes(effort)) {
          if (current.length === 1) return model;
          return {
            ...model,
            reasoning_efforts: current.filter((item) => item !== effort),
          };
        }
        return { ...model, reasoning_efforts: [...current, effort] };
      }),
    );
    if (effort === 'official') setOpenReasoningIndex(null);
  };

  const editorContent = (
    <div className="codex-experimental-model-editor">
      <div className="codex-experimental-model-editor__header">
        <span>
          {t('codex.experimentalModelCatalog.models.title', '模型列表')}
        </span>
        <button
          type="button"
          className="codex-experimental-model-editor__icon-btn"
          onClick={() => onChange([...models, nextModelDefinition(models)])}
          disabled={disabled}
          title={t('codex.experimentalModelCatalog.models.add', '添加模型')}
          aria-label={t('codex.experimentalModelCatalog.models.add', '添加模型')}
        >
          <Plus size={15} />
        </button>
      </div>

      <div className="codex-experimental-model-editor__list">
        {models.map((model, index) => (
          <div
            className="codex-experimental-model-editor__row"
            key={`${index}:${model.model_id}`}
          >
            <div className="codex-experimental-model-editor__fields">
              <label>
                <span>{t('codex.experimentalModelCatalog.models.modelId', '模型 ID')}</span>
                <input
                  type="text"
                  value={model.model_id}
                  onChange={(event) => updateModel(index, 'model_id', event.target.value)}
                  disabled={disabled}
                  className={rowErrors[index]?.modelId ? 'has-error' : ''}
                  placeholder="custom-model"
                />
                {rowErrors[index]?.modelId && (
                  <small className="codex-experimental-model-editor__error">
                    {rowErrors[index].modelId}
                  </small>
                )}
              </label>
              <label>
                <span>
                  {t('codex.experimentalModelCatalog.models.displayName', '展示名')}
                </span>
                <input
                  type="text"
                  value={model.display_name}
                  onChange={(event) => updateModel(index, 'display_name', event.target.value)}
                  disabled={disabled}
                  className={rowErrors[index]?.displayName ? 'has-error' : ''}
                  placeholder="Custom Model"
                />
                {rowErrors[index]?.displayName && (
                  <small className="codex-experimental-model-editor__error">
                    {rowErrors[index].displayName}
                  </small>
                )}
              </label>
              <div
                className="codex-experimental-model-editor__reasoning"
                ref={openReasoningIndex === index ? reasoningPickerRef : undefined}
              >
                <span className="codex-experimental-model-editor__field-label">
                  {t('codex.experimentalModelCatalog.models.reasoning', '推理强度')}
                </span>
                <div className="codex-experimental-model-editor__reasoning-picker">
                  <button
                    type="button"
                    className="codex-experimental-model-editor__reasoning-trigger"
                    onClick={() =>
                      setOpenReasoningIndex((current) => (current === index ? null : index))
                    }
                    disabled={disabled}
                    aria-expanded={openReasoningIndex === index}
                    title={
                      model.reasoning_efforts?.length
                        ? model.reasoning_efforts.map(reasoningLabel).join('、')
                        : t(
                            'codex.experimentalModelCatalog.models.followOfficial',
                            '跟随官方',
                          )
                    }
                  >
                    {model.reasoning_efforts?.length
                      ? model.reasoning_efforts.map(reasoningLabel).join('、')
                      : t(
                          'codex.experimentalModelCatalog.models.followOfficial',
                          '跟随官方',
                        )}
                  </button>
                  {openReasoningIndex === index && (
                    <div className="codex-experimental-model-editor__reasoning-menu">
                      <button
                        type="button"
                        className={`codex-experimental-model-editor__reasoning-option${
                          !model.reasoning_efforts?.length ? ' is-selected' : ' is-muted'
                        }`}
                        onClick={() => updateReasoningEfforts(index, 'official')}
                      >
                        <span
                          className="codex-experimental-model-editor__check"
                          aria-hidden="true"
                        >
                          {!model.reasoning_efforts?.length ? '✓' : ''}
                        </span>
                        {t(
                          'codex.experimentalModelCatalog.models.followOfficial',
                          '跟随官方',
                        )}
                      </button>
                      {REASONING_EFFORT_OPTIONS.map((effort) => {
                        const selected = model.reasoning_efforts?.includes(effort) ?? false;
                        return (
                          <button
                            key={effort}
                            type="button"
                            className={`codex-experimental-model-editor__reasoning-option${
                              selected
                                ? ' is-selected'
                                : !model.reasoning_efforts?.length
                                  ? ' is-muted'
                                  : ''
                            }`}
                            onClick={() => updateReasoningEfforts(index, effort)}
                          >
                            <span
                              className="codex-experimental-model-editor__check"
                              aria-hidden="true"
                            >
                              {selected ? '✓' : ''}
                            </span>
                            {reasoningLabel(effort)}
                          </button>
                        );
                      })}
                    </div>
                  )}
                </div>
              </div>
              <div className="codex-experimental-model-editor__operation">
                <span className="codex-experimental-model-editor__field-label">
                  {t('codex.experimentalModelCatalog.models.operation', '操作')}
                </span>
                <div className="codex-experimental-model-editor__operation-actions">
                  <button
                    type="button"
                    className="codex-experimental-model-editor__icon-btn"
                    data-default={defaultModelId === model.model_id ? 'true' : undefined}
                    onClick={() => onDefaultModelChange?.(model.model_id)}
                    disabled={disabled || !onDefaultModelChange}
                    title={t('codex.experimentalModelCatalog.models.setDefault', '设为默认模型')}
                    aria-label={t('codex.experimentalModelCatalog.models.setDefault', '设为默认模型')}
                    aria-pressed={defaultModelId === model.model_id}
                  >
                    <Star
                      size={14}
                      fill={defaultModelId === model.model_id ? 'currentColor' : 'none'}
                    />
                  </button>
                  <button
                    type="button"
                    className="codex-experimental-model-editor__icon-btn is-danger"
                    onClick={() => {
                      onChange(models.filter((_, itemIndex) => itemIndex !== index));
                      if (defaultModelId === model.model_id) onDefaultModelChange?.(null);
                    }}
                    disabled={disabled || models.length === 1}
                    title={t('codex.experimentalModelCatalog.models.remove', '删除模型')}
                    aria-label={t('codex.experimentalModelCatalog.models.remove', '删除模型')}
                  >
                    <Trash2 size={14} />
                  </button>
                </div>
              </div>
            </div>
          </div>
        ))}
      </div>
      <p className="codex-experimental-model-editor__hint">
        {t(
          'codex.experimentalModelCatalog.models.inheritHint',
          '官方模型保留原有能力字段；自定义模型使用通用能力模板。可见模型可直接新增或删除。',
        )}
      </p>
    </div>
  );

  if (mode === 'summary') {
    return (
      <>
        <div className="codex-experimental-model-summary">
          <div className="codex-experimental-model-summary__header">
            <span>{t('codex.experimentalModelCatalog.models.title', '模型列表')}</span>
            <button
              type="button"
              className="codex-experimental-model-summary__manage"
              onClick={() => setManagerOpen(true)}
              disabled={disabled}
            >
              {t('codex.experimentalModelCatalog.models.manage', '管理')}
            </button>
          </div>
          <div className="codex-experimental-model-summary__list">
            {models.map((model) => (
              <div
                className="codex-experimental-model-summary__row"
                key={`${model.model_id}:${model.display_name}`}
              >
                <span className="codex-experimental-model-summary__name">
                  {model.display_name || model.model_id}
                </span>
                <code>{model.model_id}</code>
                <span className="codex-experimental-model-summary__reasoning">
                  {model.reasoning_efforts?.length
                    ? model.reasoning_efforts.map(reasoningLabel).join('、')
                    : t(
                        'codex.experimentalModelCatalog.models.followOfficial',
                        '跟随官方',
                      )}
                </span>
                {defaultModelId === model.model_id && (
                  <span className="codex-experimental-model-summary__default">
                    {t('codex.experimentalModelCatalog.models.default', '默认')}
                  </span>
                )}
              </div>
            ))}
          </div>
        </div>
        {managerOpen && (
          <div className="codex-experimental-model-manager-overlay">
            <div
              className="codex-experimental-model-manager-modal"
              role="dialog"
              aria-modal="true"
              aria-labelledby="codex-experimental-model-manager-title"
            >
              <div className="codex-experimental-model-manager-modal__header">
                <h3 id="codex-experimental-model-manager-title">
                  {t('codex.experimentalModelCatalog.models.title', '模型列表')}
                </h3>
                <button
                  type="button"
                  className="codex-experimental-model-manager-modal__close"
                  onClick={() => setManagerOpen(false)}
                  aria-label={t('common.close', '关闭')}
                >
                  <X size={16} />
                </button>
              </div>
              {editorContent}
            </div>
          </div>
        )}
      </>
    );
  }

  return editorContent;
}
