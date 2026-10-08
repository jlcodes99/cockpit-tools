import { useTranslation } from 'react-i18next';
import { useState } from 'react';
import { SingleSelectDropdown } from '../../components/SingleSelectDropdown';
import { ModalErrorMessage } from '../../components/ModalErrorMessage';
import { useFloatingCardAppearance } from '../../hooks/useFloatingCardAppearance';
import { FLOATING_CARD_OPACITY_OPTIONS } from '../../utils/floatingCardAppearance';
import { showFloatingCardWindow } from '../../services/floatingCardService';

export function FloatingCardSettings({ startup, onTop, onStartupChange, onTopChange }: {
  startup: boolean; onTop: boolean;
  onStartupChange: (value: boolean) => void; onTopChange: (value: boolean) => void;
}) {
  const { t } = useTranslation();
  const appearance = useFloatingCardAppearance();
  const booleanOptions = [{ value: 'false', label: t('common.disable') }, { value: 'true', label: t('common.enable') }];
  const rows = [
    { key: 'floatingCardStartup', desc: 'floatingCardStartupDesc', value: String(startup), options: booleanOptions, disabled: false, change: (value: string) => onStartupChange(value === 'true') },
    { key: 'floatingCardAlwaysOnTop', desc: 'floatingCardAlwaysOnTopDesc', value: String(onTop), options: booleanOptions, disabled: false, change: (value: string) => onTopChange(value === 'true') },
    { key: 'floatingCardStyle', desc: 'floatingCardStyleDesc', value: String(appearance.minimal), options: [{ value: 'false', label: t('settings.general.floatingCardFull') }, { value: 'true', label: t('settings.general.floatingCardMinimal') }], disabled: appearance.busy, change: (value: string) => { void appearance.update({ minimal: value === 'true' }); } },
    { key: 'floatingCardOpacity', desc: 'floatingCardOpacityDesc', value: String(appearance.backgroundOpacity), options: [...new Set([...FLOATING_CARD_OPACITY_OPTIONS, appearance.backgroundOpacity])].sort((a, b) => b - a).map((value) => ({ value: String(value), label: `${Math.round(value * 100)}%` })), disabled: appearance.busy, change: (value: string) => { void appearance.update({ backgroundOpacity: Number(value) }); } },
  ];
  return <>
    {rows.map((row) => <div className="settings-row" key={row.key}>
      <div className="row-label"><div className="row-title">{t(`settings.general.${row.key}`)}</div><div className="row-desc">{t(`settings.general.${row.desc}`)}</div></div>
      <div className="row-control"><SingleSelectDropdown value={row.value} options={row.options} onChange={row.change} disabled={row.disabled} ariaLabel={t(`settings.general.${row.key}`)} menuWidth={200} menuMaxHeight={240} /></div>
    </div>)}
    <ModalErrorMessage message={appearance.error ? t(appearance.error, { defaultValue: appearance.error }) : ''} position="bottom" />
  </>;
}

export function FloatingCardShowSetting() {
  const { t } = useTranslation();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState('');
  const show = async () => {
    if (pending) return;
    setPending(true); setError('');
    try { await showFloatingCardWindow(); }
    catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)); }
    finally { setPending(false); }
  };
  return <>
    <div className="settings-row">
      <div className="row-label"><div className="row-title">{t('settings.general.floatingCardShowNow')}</div><div className="row-desc">{t('settings.general.floatingCardShowNowDesc')}</div></div>
      <div className="row-control"><button type="button" className="btn btn-secondary" disabled={pending} onClick={() => void show()}>{t(pending ? 'common.loading' : 'settings.general.floatingCardShowNowAction')}</button></div>
    </div>
    <ModalErrorMessage message={error} position="bottom" />
  </>;
}
