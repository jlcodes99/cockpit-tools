import { useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { RefreshCw } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import './ClientInitializationStatus.css';

type Progress = {
  platform: string;
  targetId: string;
  phase: 'initializing' | 'restarting' | 'finished';
};

export function ClientInitializationStatus({ platform, targetId }: { platform?: string; targetId?: string }) {
  const { t } = useTranslation();
  const [active, setActive] = useState<Record<string, Progress>>({});
  useEffect(() => {
    let mounted = true;
    const unlisten = listen<Progress>('client-initialization-progress', ({ payload }) => {
      if (!mounted || (platform && payload.platform !== platform) || (targetId && payload.targetId !== targetId)) return;
      const key = `${payload.platform}:${payload.targetId}`;
      setActive(current => {
        const next = { ...current };
        if (payload.phase === 'finished') delete next[key];
        else next[key] = payload;
        return next;
      });
    }).catch(() => undefined);
    return () => {
      mounted = false;
      void unlisten.then(stop => stop?.());
    };
  }, [platform, targetId]);

  const current = Object.values(active).find(item =>
    (!platform || item.platform === platform) && (!targetId || item.targetId === targetId));
  if (!current) return null;
  return (
    <div className={`client-initialization-status${platform ? '' : ' client-initialization-status-global'}`} role="status">
      <RefreshCw className="loading-spinner" size={16} aria-hidden="true" />
      <span>{t(current.phase === 'initializing' ? 'instances.status.firstLaunchInitializing' : 'instances.status.restarting')}</span>
    </div>
  );
}
