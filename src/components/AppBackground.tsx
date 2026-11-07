import { useEffect } from 'react';
import { useAppBackgroundStore } from '../stores/useAppBackgroundStore';
import { appBackgroundStyle, type AppBackgroundSettings } from '../utils/appBackground';
import './AppBackground.css';

export function BackgroundSurface({ settings, imageUrl }: { settings: AppBackgroundSettings; imageUrl: string | null }) {
  return <div className="app-background-surface" aria-hidden="true">
    <div className="app-background-image" style={appBackgroundStyle(settings, imageUrl)} />
    <div className="app-background-shade" />
  </div>;
}

/** Loading local appearance never delays app startup or changes account/card surfaces. */
export function AppBackground() {
  const { record, imageUrl, load, flush } = useAppBackgroundStore();
  useEffect(() => {
    void load();
    const savePending = () => { void flush(); };
    window.addEventListener('pagehide', savePending);
    return () => { window.removeEventListener('pagehide', savePending); };
  }, [load, flush]);
  if (record.settings.preset === 'default') return null;
  return <div className="app-background-layer" aria-hidden="true">
    <BackgroundSurface settings={record.settings} imageUrl={imageUrl} />
  </div>;
}
