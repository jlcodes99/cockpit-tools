import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react';
import { CodexProxyEngineContext } from './useCodexProxyEngine';
import { useCodexProxyEngineController } from './useCodexProxyEngineController';
import { CodexProxyEngineRequiredDialog } from './CodexProxyEngineRequiredDialog';
import { CODEX_PROXY_ENGINE_REQUIRED_EVENT, proxyEnginePrerequisiteCode } from '../../utils/codexProxyEnginePrerequisite';

/** One controller for settings, first-use guidance and action prerequisites. */
export function CodexProxyEngineProvider({ children }: { children: ReactNode }) {
  const engine = useCodexProxyEngineController();
  const current = useRef(engine);
  current.current = engine;
  const [required, setRequired] = useState<string | null>(null);
  const requiredRef = useRef<string | null>(null);
  const close = useCallback(() => { requiredRef.current = null; setRequired(null); }, []);
  useEffect(() => {
    const requested = (event: Event) => {
      const code = proxyEnginePrerequisiteCode((event as CustomEvent<unknown>).detail);
      if (!code || requiredRef.current === code) return;
      requiredRef.current = code;
      current.current.reportPreflightFailure(code);
      current.current.refresh();
      setRequired(code);
    };
    window.addEventListener(CODEX_PROXY_ENGINE_REQUIRED_EVENT, requested);
    return () => window.removeEventListener(CODEX_PROXY_ENGINE_REQUIRED_EVENT, requested);
  }, []);
  return <CodexProxyEngineContext.Provider value={engine}>
    {children}
    {required && <CodexProxyEngineRequiredDialog reason={required} onClose={close} />}
  </CodexProxyEngineContext.Provider>;
}
