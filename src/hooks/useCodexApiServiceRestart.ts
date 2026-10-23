import { useRef, useState } from 'react';
import { confirm } from '@tauri-apps/plugin-dialog';
import type { TFunction } from 'i18next';
import { restartCodexLocalAccessSidecar } from '../services/codexLocalAccessService';
import type { CodexLocalAccessState } from '../types/codexLocalAccess';

/** Lock before confirmation as well as IPC: rapid clicks must not queue restarts. */
export function useCodexApiServiceRestart({ t, onState, onError, onNotice, presentError }: {
  t: TFunction;
  onState: (state: CodexLocalAccessState) => void;
  onError: (error: string) => void;
  onNotice: (notice: string) => void;
  presentError: (error: unknown, retry: () => Promise<void>) => boolean;
}) {
  const pending = useRef(false);
  const [restarting, setRestarting] = useState(false);
  const restart = async () => {
    if (pending.current) return;
    pending.current = true;
    setRestarting(true);
    onError(''); onNotice('');
    try {
      const confirmed = await confirm(t('codex.localAccess.restartConfirmMessage'), {
        title: t('codex.localAccess.restartTitle'), kind: 'warning',
        okLabel: t('codex.localAccess.restartAction'), cancelLabel: t('common.cancel'),
      });
      if (!confirmed) return;
      const next = await restartCodexLocalAccessSidecar();
      onState(next);
      onNotice(next.recovering ? '' : t('codex.localAccess.restartSuccess'));
    } catch (error) {
      if (!presentError(error, async () => { await restart(); })) {
        onError(String(error).replace(/^Error:\s*/, ''));
      }
    } finally {
      pending.current = false;
      setRestarting(false);
    }
  };
  return { restarting, restart };
}
