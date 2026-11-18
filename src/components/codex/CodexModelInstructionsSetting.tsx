import { useEffect, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { useTranslation } from 'react-i18next';
import { getCodexModelInstructions, setCodexModelInstructions } from '../../services/codexService';

export function CodexModelInstructionsSetting() {
  const { t } = useTranslation();
  const [setting, setSetting] = useState<Awaited<ReturnType<typeof getCodexModelInstructions>>>();
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState<string>();

  useEffect(() => {
    let active = true;
    void getCodexModelInstructions()
      .then((value) => { if (active) setSetting(value); })
      .catch((reason) => { if (active) setError(String(reason)); })
      .finally(() => { if (active) setBusy(false); });
    return () => { active = false; };
  }, []);

  async function changeFile(reset: boolean) {
    setBusy(true);
    setError(undefined);
    try {
      if (reset) {
        setSetting(await setCodexModelInstructions(null));
      } else {
        const selected = await open({ multiple: false, directory: false });
        if (selected) setSetting(await setCodexModelInstructions(selected));
      }
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="codex-quick-config-section">
      <div className="codex-quick-config-section__heading">
        <h3>{t('codex.modelInstructions.title')}</h3>
        <p>{t('codex.modelInstructions.description')}</p>
        {setting?.deferred && <p>{t('codex.modelInstructions.deferred')}</p>}
      </div>
      {setting?.path && <div className="codex-quick-config-card__path"><code>{setting.path}</code></div>}
      <div className="codex-model-instructions-actions">
        <button className="btn btn-secondary" type="button" disabled={busy} onClick={() => void changeFile(false)}>
          {t('codex.modelInstructions.selectFile')}
        </button>
        <button className="btn btn-secondary" type="button" disabled={busy || !setting?.path} onClick={() => void changeFile(true)}>
          {t('codex.modelInstructions.useDefault')}
        </button>
      </div>
      {error && <div className="codex-quick-config-field__error" role="alert">{error}</div>}
    </section>
  );
}
