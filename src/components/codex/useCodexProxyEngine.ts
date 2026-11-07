import { createContext, useContext } from 'react';
import type { useCodexProxyEngineController } from './useCodexProxyEngineController';

export type CodexProxyEngineController = ReturnType<typeof useCodexProxyEngineController>;
export const CodexProxyEngineContext = createContext<CodexProxyEngineController | null>(null);

/** All entry points share one installation, status request loop and action lock. */
export function useCodexProxyEngine(): CodexProxyEngineController {
  const engine = useContext(CodexProxyEngineContext);
  if (!engine) throw new Error('useCodexProxyEngine requires CodexProxyEngineProvider');
  return engine;
}
