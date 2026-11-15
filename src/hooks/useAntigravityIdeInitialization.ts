import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

type Phase = 'required' | 'waiting' | 'retry' | 'ready';
const INITIALIZATION_TIMEOUT_MS = 20_000;
const POLL_INTERVAL_MS = 750;

export function useAntigravityIdeInitialization(instanceId: string | undefined) {
  const [status, setStatus] = useState<{ instanceId: string; phase: Phase }>();
  const phase = status?.instanceId === instanceId ? status?.phase : undefined;

  useEffect(() => {
    if (!instanceId || (phase !== undefined && phase !== 'waiting')) return;
    let active = true;
    let poll: ReturnType<typeof setTimeout>;
    const deadline = Date.now() + INITIALIZATION_TIMEOUT_MS;

    const check = async () => {
      try {
        const needsInitialization = await invoke<boolean>('antigravity_ide_needs_initialization', { instanceId });
        if (!active) return;
        if (!needsInitialization) {
          setStatus({ instanceId, phase: 'ready' });
          return;
        }
        if (phase !== 'waiting') {
          setStatus({ instanceId, phase: 'required' });
          return;
        }
      } catch {
        // The normal launch operation still reports its own errors if the preview fails.
        if (!active || phase !== 'waiting') return;
      }

      if (Date.now() >= deadline) {
        setStatus({ instanceId, phase: 'retry' });
      } else {
        poll = setTimeout(() => void check(), POLL_INTERVAL_MS);
      }
    };

    void check();
    return () => { active = false; clearTimeout(poll); };
  }, [instanceId, phase]);

  return {
    phase,
    requireInitialization: () => { if (instanceId) setStatus({ instanceId, phase: 'required' }); },
    waitForInitialization: () => { if (instanceId) setStatus({ instanceId, phase: 'waiting' }); },
  };
}
