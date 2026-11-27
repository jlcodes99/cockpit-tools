import { invoke } from '@tauri-apps/api/core';
import { isCliPathApp, saveCliLaunchPath } from './cliLaunchPath';

export const PLATFORM_LAUNCH_PATHS = {
  antigravity_legacy: { label: 'Antigravity', guide: 'https://antigravity.google/download' },
  antigravity: { label: 'Antigravity IDE', guide: 'https://antigravity.google/docs/install?tab=ide' },
  antigravity_cli: { label: 'Antigravity CLI (agy)', guide: 'https://antigravity.google/docs/install?tab=cli' },
  codebuddy: { label: 'CodeBuddy', guide: 'https://www.codebuddy.ai/docs/ide/Getting-Started/Installation' },
  codebuddy_cn: { label: 'CodeBuddy CN', guide: 'https://www.codebuddy.cn/downloads/' },
  codebuddy_cli: { label: 'CodeBuddy CLI', guide: 'https://www.codebuddy.cn/docs/cli/installation' },
  claude_cli: { label: 'Claude Code', guide: 'https://code.claude.com/docs/en/setup' },
  qoder_desktop: { label: 'Qoder Desktop', guide: 'https://qoder.com/download?product=qoder' },
  qoder: { label: 'Qoder IDE', guide: 'https://qoder.com/download?product=ide' },
  qoder_cli: { label: 'Qoder CLI', guide: 'https://qoder.com/cli' },
  qoder_cn: { label: 'Qoder CN', guide: 'https://qoder.com.cn/download?product=qoderapp' },
  qoder_cn_ide: { label: 'Qoder CN IDE', guide: 'https://qoder.com.cn/download?product=ide' },
  qoder_cn_cli: { label: 'Qoder CN CLI', guide: 'https://qoder.com.cn/cli' },
} as const;

export type PlatformLaunchPathTarget = keyof typeof PLATFORM_LAUNCH_PATHS;

export interface PlatformLaunchPathStatus {
  configuredPath: string | null;
  resolvedPath: string | null;
  usesConfiguredPath: boolean;
}

const pendingReads = new Map<PlatformLaunchPathTarget, Promise<PlatformLaunchPathStatus>>();

export function readPlatformLaunchPath(app: PlatformLaunchPathTarget) {
  const pending = pendingReads.get(app);
  if (pending) return pending;
  const request = invoke<PlatformLaunchPathStatus>('get_platform_launch_path', { app });
  pendingReads.set(app, request);
  function complete() {
    if (pendingReads.get(app) === request) pendingReads.delete(app);
  }
  void request.then(complete, complete);
  return request;
}

export const scanPlatformLaunchPaths = (app: PlatformLaunchPathTarget) =>
  invoke<string[]>('scan_platform_launch_paths', { app });

export async function savePlatformLaunchPath(app: PlatformLaunchPathTarget, path: string) {
  if (isCliPathApp(app)) await saveCliLaunchPath(app, path.trim());
  else await invoke('set_app_path', { app, path: path.trim(), validate: true });
  pendingReads.delete(app);
  window.dispatchEvent(new CustomEvent('config-updated', { detail: { launchPathTarget: app } }));
}
