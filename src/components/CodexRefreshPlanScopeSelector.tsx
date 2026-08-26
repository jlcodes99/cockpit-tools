import { useEffect, useMemo, useRef, useState } from 'react';
import { Check, ChevronDown, Minus } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import type {
  CodexAutoRefreshPlanKey,
  CodexAutoRefreshPlanOption,
} from '../utils/codexAutoRefreshPlanScope';
import './CodexRefreshPlanScopeSelector.css';

interface CodexRefreshPlanScopeSelectorProps {
  options: CodexAutoRefreshPlanOption[];
  selectedKeys: CodexAutoRefreshPlanKey[];
  onChange: (keys: CodexAutoRefreshPlanKey[]) => void;
  disabled?: boolean;
  variant?: 'default' | 'quick';
}

export function CodexRefreshPlanScopeSelector({
  options,
  selectedKeys,
  onChange,
  disabled = false,
  variant = 'default',
}: CodexRefreshPlanScopeSelectorProps) {
  const { t, i18n } = useTranslation();
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);

  const selectedSet = useMemo(() => new Set(selectedKeys), [selectedKeys]);
  const allSelected = options.length > 0 && options.every((option) => selectedSet.has(option.key));
  const partiallySelected = selectedKeys.length > 0 && !allSelected;
  const totalCount = useMemo(
    () => options.reduce((sum, option) => sum + option.count, 0),
    [options],
  );
  const language = (i18n.resolvedLanguage || i18n.language || '').toLowerCase();
  const chinesePlanLabels: Partial<Record<CodexAutoRefreshPlanKey, string>> = language.startsWith(
    'zh-tw',
  )
    ? {
        free: '免費版',
        go: 'Go 版',
        plus: 'Plus 版',
        pro: 'Pro 版',
        team: '團隊版',
        business: '商業版',
        enterprise: '企業版',
        edu_k12: '教育 / K12',
        unknown: '其他 / 未識別',
      }
    : language.startsWith('zh')
      ? {
          free: '免费版',
          go: 'Go 版',
          plus: 'Plus 版',
          pro: 'Pro 版',
          team: '团队版',
          business: '商业版',
          enterprise: '企业版',
          edu_k12: '教育 / K12',
          unknown: '其他 / 未识别',
        }
      : {};
  const summary = allSelected
    ? t('settings.general.codexAutoRefreshPlansAll', 'All plans')
    : selectedKeys.length === 0
      ? t('settings.general.codexAutoRefreshPlansNone', 'No automatic quota refresh')
      : t('settings.general.codexAutoRefreshPlansSelected', {
          count: selectedKeys.length,
          defaultValue: '{{count}} selected',
        });

  useEffect(() => {
    if (!open) return;

    const handlePointerDown = (event: MouseEvent) => {
      const target = event.target as Node | null;
      if (target && !rootRef.current?.contains(target)) {
        setOpen(false);
      }
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setOpen(false);
      }
    };

    document.addEventListener('mousedown', handlePointerDown);
    document.addEventListener('keydown', handleKeyDown);
    return () => {
      document.removeEventListener('mousedown', handlePointerDown);
      document.removeEventListener('keydown', handleKeyDown);
    };
  }, [open]);

  useEffect(() => {
    if (disabled) setOpen(false);
  }, [disabled]);

  const toggleAll = () => {
    onChange(allSelected ? [] : options.map((option) => option.key));
  };

  const toggleOption = (key: CodexAutoRefreshPlanKey) => {
    if (selectedSet.has(key)) {
      onChange(selectedKeys.filter((selectedKey) => selectedKey !== key));
      return;
    }
    onChange([...selectedKeys, key]);
  };

  return (
    <div
      className={`codex-plan-scope${variant === 'quick' ? ' codex-plan-scope--quick' : ''}`}
      ref={rootRef}
    >
      <button
        type="button"
        className={`codex-plan-scope-trigger${open ? ' is-open' : ''}`}
        onClick={() => setOpen((previous) => !previous)}
        aria-haspopup="listbox"
        aria-expanded={open}
        disabled={disabled}
      >
        <span className="codex-plan-scope-trigger-label" title={summary}>
          {summary}
        </span>
        <ChevronDown size={15} aria-hidden="true" />
      </button>

      {open && (
        <div
          className="codex-plan-scope-menu"
          role="listbox"
          aria-multiselectable="true"
          aria-label={t('settings.general.codexAutoRefreshPlans', 'Auto-refresh plans')}
        >
          <button
            type="button"
            className={`codex-plan-scope-option codex-plan-scope-option-all${
              selectedKeys.length > 0 ? ' is-selected' : ''
            }`}
            onClick={toggleAll}
            role="option"
            aria-selected={allSelected}
          >
            <span
              className={`codex-plan-scope-checkbox${
                allSelected || partiallySelected ? ' is-checked' : ''
              }`}
              aria-hidden="true"
            >
              {partiallySelected ? <Minus size={12} /> : allSelected ? <Check size={12} /> : null}
            </span>
            <span className="codex-plan-scope-option-content">
              <span>{t('settings.general.codexAutoRefreshPlansAll', 'All plans')}</span>
              <span className="codex-plan-scope-count">{totalCount}</span>
            </span>
          </button>

          <div className="codex-plan-scope-separator" />

          {options.map((option) => {
            const selected = selectedSet.has(option.key);
            return (
              <button
                key={option.key}
                type="button"
                className={`codex-plan-scope-option${selected ? ' is-selected' : ''}`}
                onClick={() => toggleOption(option.key)}
                role="option"
                aria-selected={selected}
              >
                <span
                  className={`codex-plan-scope-checkbox${selected ? ' is-checked' : ''}`}
                  aria-hidden="true"
                >
                  {selected ? <Check size={12} /> : null}
                </span>
                <span className="codex-plan-scope-option-content">
                  <span>
                    {chinesePlanLabels[option.key] ||
                      (option.key === 'unknown'
                        ? t('settings.general.codexAutoRefreshPlansUnknown', 'Other / Unknown')
                        : option.label)}
                  </span>
                  <span className="codex-plan-scope-count">{option.count}</span>
                </span>
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
