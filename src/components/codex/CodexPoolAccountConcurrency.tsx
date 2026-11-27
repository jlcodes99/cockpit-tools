import { useEffect, useId, useRef, useState, type FormEvent } from 'react';
import { useTranslation } from 'react-i18next';

interface Props {
  value: number | null;
  disabled?: boolean;
  onSave: (value: number | null) => Promise<void>;
}

export function CodexPoolAccountConcurrency({ value, disabled, onSave }: Props) {
  const { t } = useTranslation();
  const name = t('codex.apiService.routing.accountConcurrencyLimit');
  const hintId = useId();
  const details = useRef<HTMLDetailsElement>(null);
  const [draft, setDraft] = useState(value === null ? '' : String(value));
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');

  useEffect(() => {
    setDraft(value === null ? '' : String(value));
  }, [value]);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const input = draft.trim();
    const next = input === '' ? null : Number(input);
    if (next !== null && (!/^\d+$/.test(input) || next < 0 || next > 64)) {
      setError(t('codex.apiService.routing.accountConcurrencyInvalid'));
      return;
    }
    if (disabled || saving) return;
    setSaving(true);
    setError('');
    try {
      await onSave(next);
      if (details.current) details.current.open = false;
    } catch {
      setError(t('codex.apiService.routing.accountConcurrencySaveFailed'));
    } finally {
      setSaving(false);
    }
  }

  const summarySuffix =
    value === null
      ? ''
      : value === 0
        ? ` · ${t('codex.apiService.routing.accountConcurrencyUnlimited')}`
        : ` · ${value}`;

  return (
    <details ref={details} className="codex-pool-quota-floor">
      <summary>
        {name}
        {summarySuffix}
      </summary>
      <form onSubmit={(event) => void submit(event)}>
        <p id={hintId}>{t('codex.apiService.routing.accountConcurrencyHint')}</p>
        <div>
          <input
            type="text"
            inputMode="numeric"
            value={draft}
            aria-label={name}
            aria-describedby={hintId}
            onChange={(event) => {
              setDraft(event.target.value);
              setError('');
            }}
            disabled={disabled || saving}
          />
          <button type="submit" className="btn btn-secondary btn-sm" disabled={disabled || saving}>
            {t(saving ? 'common.saving' : 'common.save')}
          </button>
        </div>
        {error && <p role="alert">{error}</p>}
      </form>
    </details>
  );
}
