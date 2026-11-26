import { useEffect, useState } from 'react';
import { Globe, SlidersHorizontal } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import {
  getT3codeNativeLanguage,
  subscribeT3codeLanguage,
  updateT3codeNativeLanguage,
} from '../../services/t3codeService';
import { T3codeLanguageSelect } from './T3codeLanguageSelect';
import { T3codePlatformOrderButton } from './T3codePlatformOrder';
import { T3codePrivacySettingsButton } from './T3codePrivacySettings';

export function T3codeCommonSettings() {
  const { t } = useTranslation();
  const [retries, setRetries] = useState(0);
  const [language, setLanguage] = useState('');
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState('');
  const [savedForLaunch, setSavedForLaunch] = useState(false);
  const label = t('t3code.commonSettings.language');

  useEffect(() => {
    let current = true;
    let languageChanged = false;
    const unsubscribe = subscribeT3codeLanguage((language) => {
      languageChanged = true;
      setLanguage(language);
      setError('');
    });
    void getT3codeNativeLanguage()
      .then(({ language }) => {
        if (current && !languageChanged) setLanguage(language);
      })
      .catch((cause) => {
        if (current) setError(String(cause));
      })
      .finally(() => {
        if (current) setBusy(false);
      });
    return () => {
      current = false;
      unsubscribe();
    };
  }, [retries]);

  async function changeLanguage(value: string) {
    setBusy(true);
    setError('');
    setSavedForLaunch(false);
    try {
      const saved = await updateT3codeNativeLanguage(value);
      setSavedForLaunch(!saved.clientConnected);
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="t3code-common-settings" aria-label={t('t3code.commonSettings.title')}>
      <div className="t3code-common-settings-lead">
        <span className="t3code-common-settings-icon" aria-hidden="true">
          <SlidersHorizontal size={15} />
        </span>
        <h3 className="t3code-common-settings-title">{t('t3code.commonSettings.title')}</h3>
      </div>

      <div className="t3code-common-settings-controls">
        <div className="t3code-common-settings-language">
          <Globe size={14} className="t3code-common-settings-lang-icon" aria-hidden="true" />
          <span title={label}>{label}</span>
          <T3codeLanguageSelect
            className="settings-dropdown t3code-common-settings-select"
            label={label}
            value={language}
            disabled={busy || !language}
            placeholder={t(busy ? 'common.loading' : 'common.unknown')}
            onChange={(value) => void changeLanguage(value)}
          />
        </div>
        <span className="t3code-common-settings-divider" aria-hidden="true" />
        <T3codePrivacySettingsButton />
        <T3codePlatformOrderButton />
      </div>

      {error ? (
        <div className="t3code-common-settings-feedback is-error">
          <span role="alert">{error}</span>
          {!language && (
            <button
              className="btn btn-secondary btn-sm"
              type="button"
              onClick={() => {
                setBusy(true);
                setError('');
                setRetries((count) => count + 1);
              }}
            >
              {t('common.retry')}
            </button>
          )}
        </div>
      ) : (
        savedForLaunch && (
          <p className="t3code-common-settings-feedback" role="status">
            {t('t3code.nativeSettings.savedForLaunch')}
          </p>
        )
      )}
    </section>
  );
}
