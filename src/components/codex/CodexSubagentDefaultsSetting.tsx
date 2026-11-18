import { useEffect, useId, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  getCodexSubagentDefaults,
  setCodexSubagentDefaults,
  type CodexSubagentDefaults,
} from '../../services/codexService';

const REASONING_EFFORT_EXAMPLES = ['minimal', 'low', 'medium', 'high', 'xhigh', 'ultra', 'max'];

export function CodexSubagentDefaultsSetting({ modelIds }: { modelIds: string[] }) {
  const { t } = useTranslation();
  const modelOptionsId = useId();
  const effortOptionsId = useId();
  const [saved, setSaved] = useState<CodexSubagentDefaults>();
  const [model, setModel] = useState('');
  const [reasoningEffort, setReasoningEffort] = useState('');
  const [saving, setSaving] = useState(false);
  const [success, setSuccess] = useState(false);
  const [error, setError] = useState<string>();

  useEffect(() => {
    let active = true;
    void getCodexSubagentDefaults().then((value) => {
      if (!active) return;
      setSaved(value);
      setModel(value.model ?? '');
      setReasoningEffort(value.reasoningEffort ?? '');
    }).catch((reason) => { if (active) setError(String(reason)); });
    return () => { active = false; };
  }, []);

  const pending = { model: model.trim() || null, reasoningEffort: reasoningEffort.trim() || null };
  const canSave = saved && !saving &&
    (pending.model !== saved.model || pending.reasoningEffort !== saved.reasoningEffort);

  async function save() {
    if (!canSave) return;
    setSaving(true);
    setError(undefined);
    setSuccess(false);
    try {
      const updated = await setCodexSubagentDefaults(pending);
      setSaved(updated);
      setModel(updated.model ?? '');
      setReasoningEffort(updated.reasoningEffort ?? '');
      setSuccess(true);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setSaving(false);
    }
  }

  return (
    <section className="codex-quick-config-section">
      <div className="codex-quick-config-section__heading">
        <h3>{t('codex.subagentDefaults.title')}</h3>
        <p>{t('codex.subagentDefaults.description')}</p>
      </div>
      <div className="codex-context-override-fields">
        <label>
          {t('codex.wakeup.presetModelLabel')}
          <input className="form-input" list={modelOptionsId} value={model}
            disabled={!saved || saving}
            onChange={(event) => { setModel(event.target.value); setSuccess(false); }} />
          <datalist id={modelOptionsId}>
            {modelIds.map((id) => <option key={id} value={id} />)}
          </datalist>
        </label>
        <label>
          {t('codex.wakeup.reasoningEffortLabel')}
          <input className="form-input" list={effortOptionsId} value={reasoningEffort}
            disabled={!saved || saving}
            onChange={(event) => { setReasoningEffort(event.target.value); setSuccess(false); }} />
          <datalist id={effortOptionsId}>
            {REASONING_EFFORT_EXAMPLES.map((effort) => <option key={effort} value={effort} />)}
          </datalist>
        </label>
      </div>
      <div>
        <button className="btn btn-secondary" type="button" disabled={!canSave} onClick={() => void save()}>
          {t('common.save')}
        </button>
      </div>
      {success && <div role="status">{t('common.saved')}</div>}
      {error && <div className="codex-quick-config-field__error" role="alert">{error}</div>}
    </section>
  );
}
