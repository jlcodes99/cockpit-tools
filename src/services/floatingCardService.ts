import { invoke } from '@tauri-apps/api/core';
import type { Page } from '../types/navigation';
import type { PlatformId } from '../types/platform';

export interface FloatingCardInstanceContext {
  platformId: PlatformId;
  instanceId: string;
  instanceName: string;
  boundAccountId: string;
}

export interface FloatingCardAppearance {
  minimal: boolean;
  backgroundOpacity: number;
}

export const FLOATING_CARD_APPEARANCE_CHANGED_EVENT = 'floating-card:appearance-changed';

export async function updateFloatingCardAppearance(patch: Partial<FloatingCardAppearance>): Promise<FloatingCardAppearance> {
  return await invoke('update_floating_card_appearance', {
    minimal: patch.minimal ?? null,
    backgroundOpacity: patch.backgroundOpacity ?? null,
  });
}

export async function resizeCurrentFloatingCardWindow(height: number): Promise<void> {
  return await invoke('resize_current_floating_card_window', { height });
}

export async function showFloatingCardWindow(): Promise<void> {
  return await invoke('show_floating_card_window');
}

export async function showInstanceFloatingCardWindow(
  context: FloatingCardInstanceContext,
): Promise<void> {
  return await invoke('show_instance_floating_card_window', { context });
}

export async function getFloatingCardContext(
  windowLabel: string,
): Promise<FloatingCardInstanceContext | null> {
  return await invoke('get_floating_card_context', { windowLabel });
}

export async function hideFloatingCardWindow(): Promise<void> {
  return await invoke('hide_floating_card_window');
}

export async function hideCurrentFloatingCardWindow(): Promise<void> {
  return await invoke('hide_current_floating_card_window');
}

export async function setFloatingCardAlwaysOnTop(alwaysOnTop: boolean): Promise<void> {
  return await invoke('set_floating_card_always_on_top', { alwaysOnTop });
}

export async function setCurrentFloatingCardWindowAlwaysOnTop(alwaysOnTop: boolean): Promise<void> {
  return await invoke('set_current_floating_card_window_always_on_top', { alwaysOnTop });
}

export async function setFloatingCardConfirmOnClose(confirmOnClose: boolean): Promise<void> {
  return await invoke('set_floating_card_confirm_on_close', { confirmOnClose });
}

export async function saveFloatingCardPosition(x: number, y: number): Promise<void> {
  return await invoke('save_floating_card_position', { x, y });
}

export async function showMainWindowAndNavigate(page: Page): Promise<void> {
  return await invoke('show_main_window_and_navigate', { page });
}
