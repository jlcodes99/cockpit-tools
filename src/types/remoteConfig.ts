import type { PlatformId } from './platform';

export type RemoteUpdatePromptMode = 'normal' | 'popup';

export interface RemoteConfigAppliedRule {
  platformIds: PlatformId[];
  reason?: string | null;
}

export interface RemoteConfigState {
  version: string;
  codexOAuthAppVersion: string;
  updatedAt: number;
  currentOs: string;
  hiddenPlatformIds: PlatformId[];
  appliedRules: RemoteConfigAppliedRule[];
  refreshIntervalMs: number;
  updatePromptMode: RemoteUpdatePromptMode;
}
