import { useEffect, useMemo, useState } from 'react';
import { TauriEvent } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { Maximize2, Minimize2, Minus, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { subscribeTauriEvent } from '../../utils/tauriEventListener';

function logWindowError(error: unknown) {
  console.warn('[Window] Linux title bar:', error);
}

export function LinuxWindowTitlebar() {
  const { t } = useTranslation();
  const currentWindow = useMemo(getCurrentWindow, []);
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    let active = true;
    const refresh = () => {
      void currentWindow.isMaximized().then((value) => {
        if (active) setMaximized(value);
      }).catch(logWindowError);
    };
    const unsubscribe = subscribeTauriEvent(TauriEvent.WINDOW_RESIZED, refresh, {
      target: { kind: 'Window', label: currentWindow.label },
    });
    refresh();
    return () => {
      active = false;
      unsubscribe();
    };
  }, [currentWindow]);

  const minimizeLabel = t('common.windowMinimize');
  const maximizeLabel = t(maximized ? 'common.windowRestore' : 'common.windowMaximize');
  const closeLabel = t('common.close');

  return (
    <div className="linux-window-titlebar">
      <div className="linux-window-drag" data-tauri-drag-region />
      <button type="button" className="linux-window-control" title={minimizeLabel} aria-label={minimizeLabel}
        onClick={() => void currentWindow.minimize().catch(logWindowError)}>
        <Minus size={16} aria-hidden="true" />
      </button>
      <button type="button" className="linux-window-control" title={maximizeLabel} aria-label={maximizeLabel}
        onClick={() => void currentWindow.toggleMaximize().catch(logWindowError)}>
        {maximized ? <Minimize2 size={15} aria-hidden="true" /> : <Maximize2 size={15} aria-hidden="true" />}
      </button>
      <button type="button" className="linux-window-control linux-window-control-close" title={closeLabel} aria-label={closeLabel}
        onClick={() => void currentWindow.close().catch(logWindowError)}>
        <X size={17} aria-hidden="true" />
      </button>
    </div>
  );
}
