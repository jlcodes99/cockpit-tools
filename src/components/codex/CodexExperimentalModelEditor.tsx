import { Plus, Star, Trash2 } from 'lucide-react';
import { useEffect, useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import type { CodexExperimentalModelDefinition } from '../../types/codex';
import './CodexExperimentalModelEditor.css';

interface CodexExperimentalModelEditorProps {
  models: CodexExperimentalModelDefinition[];
  disabled?: boolean;
  onChange: (models: CodexExperimentalModelDefinition[]) => void;
  onValidationChange?: (error: string | null) => void;
}

const MODEL_ID_PATTERN = /^[A-Za-z0-9._:/-]+$/;

export function validateCodexExperimentalModels(
  models: CodexExperimentalModelDefinition[],
  translate: (key: string, fallback: string) => string,
): string | null {
  if (models.length === 0) {
    return translate(
      'codex.experimentalModelCatalog.models.validation.required',
      '至少保留一个实验模型。',
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
  let suffix = 2;
  while (existing.has(`gpt-5.6-sol-wm${suffix}`)) suffix += 1;
  return {
    model_id: `gpt-5.6-sol-wm${suffix}`,
    display_name: `GPT-5.6 Sol WM${suffix}`,
  };
}

export function CodexExperimentalModelEditor({
  models,
  disabled = false,
  onChange,
  onValidationChange,
}: CodexExperimentalModelEditorProps) {
  const { t } = useTranslation();
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

  const updateModel = (
    index: number,
    field: keyof CodexExperimentalModelDefinition,
    value: string,
  ) => {
    onChange(
      models.map((model, modelIndex) =>
        modelIndex === index ? { ...model, [field]: value } : model,
      ),
    );
  };

  return (
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
            <div className="codex-experimental-model-editor__row-head">
              <span className={index === 0 ? 'is-default' : ''}>
                {index === 0
                  ? t('codex.experimentalModelCatalog.models.default', '默认模型')
                  : t('codex.experimentalModelCatalog.models.model', '实验模型')}
              </span>
              <div className="codex-experimental-model-editor__actions">
                {index > 0 && (
                  <button
                    type="button"
                    className="codex-experimental-model-editor__icon-btn"
                    onClick={() =>
                      onChange([model, ...models.filter((_, itemIndex) => itemIndex !== index)])
                    }
                    disabled={disabled}
                    title={t(
                      'codex.experimentalModelCatalog.models.setDefault',
                      '设为默认模型',
                    )}
                    aria-label={t(
                      'codex.experimentalModelCatalog.models.setDefault',
                      '设为默认模型',
                    )}
                  >
                    <Star size={14} />
                  </button>
                )}
                <button
                  type="button"
                  className="codex-experimental-model-editor__icon-btn is-danger"
                  onClick={() =>
                    onChange(models.filter((_, itemIndex) => itemIndex !== index))
                  }
                  disabled={disabled || models.length === 1}
                  title={t('codex.experimentalModelCatalog.models.remove', '删除模型')}
                  aria-label={t('codex.experimentalModelCatalog.models.remove', '删除模型')}
                >
                  <Trash2 size={14} />
                </button>
              </div>
            </div>
            <div className="codex-experimental-model-editor__fields">
              <label>
                <span>{t('codex.experimentalModelCatalog.models.modelId', '模型 ID')}</span>
                <input
                  type="text"
                  value={model.model_id}
                  onChange={(event) => updateModel(index, 'model_id', event.target.value)}
                  disabled={disabled}
                  className={rowErrors[index]?.modelId ? 'has-error' : ''}
                  placeholder="gpt-5.6-sol-wm2"
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
                  placeholder="GPT-5.6 Sol WM2"
                />
                {rowErrors[index]?.displayName && (
                  <small className="codex-experimental-model-editor__error">
                    {rowErrors[index].displayName}
                  </small>
                )}
              </label>
            </div>
          </div>
        ))}
      </div>
      <p className="codex-experimental-model-editor__hint">
        {t(
          'codex.experimentalModelCatalog.models.inheritHint',
          '模型继承 gpt-5.6-sol 的完整能力字段；列表第一项同时作为默认模型。',
        )}
      </p>
    </div>
  );
}
