import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { RefreshCw, Undo2 } from 'lucide-react';
import { BackgroundSurface } from '../../components/AppBackground';
import { SingleSelectDropdown } from '../../components/SingleSelectDropdown';
import { prepareBackgroundImage } from '../../services/appBackgroundService';
import { useAppBackgroundStore } from '../../stores/useAppBackgroundStore';
import { BACKGROUND_PRESETS, DEFAULT_APP_BACKGROUND, type AppBackgroundSettings, type BackgroundPosition } from '../../utils/appBackground';
import { BackgroundSkinLibrary } from './BackgroundSkinLibrary';
import './AppBackgroundSettings.css';

export function AppBackgroundSettings() {
  const { t } = useTranslation();
  const { record, imageUrl, loaded, loading, saving, error: loadError, saveError, load, update, flush, retrySave,
    addSkin, selectSkin, replaceSkin, renameSkin, deleteSkin } = useAppBackgroundStore();
  const { settings } = record;
  const [preparing, setPreparing] = useState(false);
  const [error, setError] = useState('');
  const fileInput = useRef<HTMLInputElement>(null);
  const errorRegion = useRef<HTMLDivElement>(null);
  const mounted = useRef(true);
  const pickerTarget = useRef<string | null>(null);
  const preparingRef = useRef(false);
  useEffect(() => { mounted.current = true; void load(); return () => { mounted.current = false; void flush(); }; }, [load, flush]);
  useEffect(() => {
    if (error || saveError) {
      errorRegion.current?.scrollIntoView({ block: 'nearest' });
      errorRegion.current?.focus({ preventScroll: true });
    }
  }, [error, saveError]);
  const change = (patch: Partial<AppBackgroundSettings>, debounce = false) => {
    setError(''); update(patch, { debounce });
  };
  const openPicker = (id: string | null = null) => {
    setError(''); pickerTarget.current = id;
    if (fileInput.current) { fileInput.current.value = ''; fileInput.current.click(); }
  };
  const selectImage = async (file?: File) => {
    if (!file || preparingRef.current) return;
    const target = pickerTarget.current;
    preparingRef.current = true;
    setPreparing(true); setError('');
    try {
      const prepared = await prepareBackgroundImage(file);
      if (mounted.current) {
        if (target) replaceSkin(target, prepared);
        else addSkin(file.name.replace(/\.[^.]+$/, '').slice(0, 80), prepared);
      }
    } catch (cause) {
      if (mounted.current) {
        const key = cause instanceof Error ? cause.message : '';
        setError(['imageType', 'imageSize', 'imageRead'].includes(key) ? key : 'imageRead');
      }
    } finally {
      preparingRef.current = false;
      if (mounted.current) setPreparing(false);
    }
  };
  const inactive = !loaded || preparing;
  return <section className="settings-group background-settings" aria-labelledby="background-setting-title">
    <div className="background-settings-heading">
      <div>
        <h3 id="background-setting-title">{t('settings.background.title')}</h3>
        <p>{t('settings.background.description')}</p>
      </div>
    </div>
    {loading && <p role="status">{t('common.loading')}</p>}
    {loadError && <div className="background-settings-error" role="alert">
      <span>{t('settings.background.storage')}</span>
      <button type="button" className="btn btn-secondary" onClick={() => void load()}><RefreshCw size={14} />{t('common.retry')}</button>
    </div>}
    <fieldset disabled={inactive} className="background-settings-fields" aria-busy={preparing || loading}>
      <div className="background-settings-grid">
        <div className="background-settings-options">
          <h4>{t('settings.background.builtInSkins')}</h4>
          <div className="background-presets" role="group" aria-label={t('settings.background.presets')}>
            {BACKGROUND_PRESETS.filter(preset => preset !== 'custom').map(preset => <button type="button" key={preset}
              className={`background-preset ${settings.preset === preset ? 'selected' : ''}`}
              disabled={inactive}
              aria-pressed={settings.preset === preset} onClick={() => change({ preset })}>
              <span className="background-preset-swatch">
                <BackgroundSurface settings={{ ...DEFAULT_APP_BACKGROUND, preset, strength: 85 }} imageUrl={imageUrl} />
              </span>
              <span>{t(`settings.background.${preset}`)}</span>
            </button>)}
          </div>
          <BackgroundSkinLibrary skins={record.skins ?? []} activeSkinId={record.activeSkinId ?? null} disabled={inactive}
            onAdd={() => openPicker()} onReplace={openPicker} onSelect={id => { setError(''); selectSkin(id); }}
            onRename={(id, name) => { setError(''); renameSkin(id, name); }} onDelete={id => { setError(''); deleteSkin(id); }} />
          <p className="background-settings-hint">{t('settings.background.imageHint')}</p>
          <input hidden ref={fileInput} type="file" accept="image/jpeg,image/png,image/webp" onChange={event => {
            const file = event.target.files?.[0]; event.target.value = ''; void selectImage(file);
          }} />
        </div>
        <div className="background-preview" role="img" aria-label={t('settings.background.preview')}>
          <BackgroundSurface settings={settings} imageUrl={imageUrl} />
          <div className="background-preview-chrome" aria-hidden="true"><i /><div><i /><i /><i /></div></div>
          <span>{t('settings.background.preview')}</span>
        </div>
      </div>
      <div className="background-adjustments">
        <label className="background-range">
          <span>{t('settings.background.strength')}<output>{settings.strength}%</output></span>
          <input type="range" min={0} max={100} step={1} value={settings.strength} disabled={inactive || settings.preset === 'default'} onChange={event => change({ strength: Number(event.target.value) }, true)} onBlur={() => void flush()} />
        </label>
        <label className="background-range">
          <span>{t('settings.background.blur')}<output>{t('settings.background.pixels', { count: settings.blur })}</output></span>
          <input type="range" min={0} max={20} step={1} value={settings.blur} disabled={inactive || settings.preset === 'default'} onChange={event => change({ blur: Number(event.target.value) }, true)} onBlur={() => void flush()} />
        </label>
        <div className="background-position">
          <span>{t('settings.background.position')}</span>
          <SingleSelectDropdown value={settings.position} disabled={inactive || settings.preset !== 'custom'}
            ariaLabel={t('settings.background.position')}
            options={(['center', 'top', 'bottom'] as const).map(value => ({ value, label: t(`settings.background.${value}`) }))}
            onChange={position => change({ position: position as BackgroundPosition })} />
        </div>
      </div>
      <p className="background-settings-hint">{t('settings.background.localHint')}</p>
      <div className="background-settings-actions">
        {saving && <span className="background-settings-saving" role="status">{t('common.saving')}</span>}
        <button type="button" className="btn btn-secondary" disabled={inactive} onClick={() => { setError(''); update(DEFAULT_APP_BACKGROUND, { image: null }); }}><Undo2 size={14} />{t('settings.background.restore')}</button>
      </div>
    </fieldset>
    {preparing && <p className="background-settings-hint" role="status">{t('common.processing')}</p>}
    {(error || saveError) && <div className="background-settings-error" role="alert" tabIndex={-1} ref={errorRegion}>
      {error ? t(`settings.background.${error}`) : t('settings.background.saveFailed')}
      {!error && saveError && <button type="button" className="btn btn-secondary" onClick={retrySave}>{t('common.retry')}</button>}
    </div>}
  </section>;
}
