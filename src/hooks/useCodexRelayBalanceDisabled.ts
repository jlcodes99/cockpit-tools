import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';
import { subscribeTauriEvent } from '../utils/tauriEventListener';

export function useCodexRelayBalanceDisabled(): boolean {
  const [disabled, setDisabled] = useState(true);

  useEffect(() => {
    let initialReadCurrent = true;
    const unsubscribe = subscribeTauriEvent<boolean>('codex:relay-balance-changed', ({ payload }) => {
      initialReadCurrent = false;
      setDisabled(payload);
    });
    void invoke<{ codex_hide_relay_quota?: boolean }>('get_general_config')
      .then((config) => {
        if (initialReadCurrent) setDisabled(config.codex_hide_relay_quota ?? false);
      })
      .catch((error) => console.error('Failed to load Codex balance setting:', error));
    return () => {
      initialReadCurrent = false;
      unsubscribe();
    };
  }, []);

  return disabled;
}
