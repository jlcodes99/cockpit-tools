import { useMemo } from 'react';

export type PlatformRuntimeSupport = 'desktop' | 'macos-only' | 'windows-only' | 'linux-only';

export function usePlatformRuntimeSupport(mode: PlatformRuntimeSupport): boolean {
  return useMemo(() => {
    if (typeof navigator === 'undefined') return false;
    const platform = (navigator as any).platform || '';
    const ua = navigator.userAgent || '';
    const isMac = /mac/i.test(platform) || /mac/i.test(ua);
    const isWindows = /win/i.test(platform) || /windows/i.test(ua);
    const isLinux = /linux/i.test(platform) || /linux/i.test(ua);

    if (mode === 'macos-only') {
      return isMac;
    }
    if (mode === 'windows-only') {
      return isWindows;
    }
    if (mode === 'linux-only') {
      return isLinux;
    }
    return isMac || isWindows || isLinux;
  }, [mode]);
}
