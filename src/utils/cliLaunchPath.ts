import { invoke } from '@tauri-apps/api/core';
import i18n from '../i18n';
import { getCodexWakeupCliStatus, updateCodexWakeupRuntimeConfig } from '../services/codexWakeupService';

export const CLI_PATH_LABELS = {
  antigravity_cli: 'Antigravity CLI (agy)',
  claude_cli: 'Claude Code',
  codebuddy_cli: 'CodeBuddy CLI',
  codex_cli: 'Codex CLI',
  grok_cli: 'Grok CLI',
  qoder_cn_cli: 'Qoder CN CLI',
  qoder_cli: 'Qoder CLI',
} as const;

export type CliPathApp = keyof typeof CLI_PATH_LABELS;

export function isCliPathApp(app: string): app is CliPathApp {
  return Object.prototype.hasOwnProperty.call(CLI_PATH_LABELS, app);
}

export async function readCliLaunchPath(app: CliPathApp, detect = false): Promise<string | null> {
  if (app === 'codex_cli') {
    const status = await getCodexWakeupCliStatus();
    return (detect ? status.binary_path : status.configured_codex_cli_path ?? status.binary_path) ?? null;
  }
  if (app === 'grok_cli') {
    const status = await invoke<{ configuredPath?: string; binaryPath?: string }>('grok_get_cli_status');
    return (detect ? status.binaryPath : status.configuredPath ?? status.binaryPath) ?? null;
  }
  return invoke('inspect_cli_launch_path', { app, detect });
}

export async function saveCliLaunchPath(app: CliPathApp, path: string) {
  if (app === 'codex_cli') {
    const status = await getCodexWakeupCliStatus();
    const updated = await updateCodexWakeupRuntimeConfig(path, status.configured_node_path ?? undefined);
    if (!updated.available) throw new Error(updated.message || `CLI_PATH_NOT_FOUND:${app}`);
  } else if (app === 'grok_cli') {
    const updated = await invoke<{ available: boolean }>('grok_update_cli_runtime_config', { grokCliPath: path || null });
    if (!updated.available) throw new Error(`CLI_PATH_NOT_FOUND:${app}`);
  } else {
    await invoke('save_cli_launch_path', { app, path });
  }
}

export async function withCliLaunchPath<T>(
  app: CliPathApp,
  action: () => Promise<T>,
  isMissing = (error: unknown) => String(error).replace(/^Error: /, '') === `CLI_PATH_NOT_FOUND:${app}`,
): Promise<T> {
  try {
    return await action();
  } catch (error) {
    if (!isMissing(error)) throw error;
    return new Promise((resolve, reject) => {
      const event = new CustomEvent('app-path-missing', {
        cancelable: true,
        detail: {
          app,
          retry: { kind: 'cli' },
          resume: async () => { resolve(await action()); },
          cancel: () => reject(new Error(i18n.t('appPath.missing.operationCancelled'))),
        },
      });
      // A secondary webview may not own the path dialog.
      if (window.dispatchEvent(event)) reject(error);
    });
  }
}
