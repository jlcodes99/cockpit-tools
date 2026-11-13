import { previewDurableUiSetting, readDurableUiSetting } from './durableUiSettings';

const ANTIGRAVITY_SEAMLESS_SWITCH_UNLOCK_STORAGE_KEY =
  'agtools.unlock.antigravity_seamless_switch.v1';

export const FEATURE_UNLOCK_CHANGED_EVENT = 'agtools:feature-unlock-changed';

export type FeatureUnlockChangedDetail = {
  feature: 'antigravity.seamless_switch';
  unlocked: boolean;
};

export function isAntigravitySeamlessSwitchFeatureUnlocked(): boolean {
  try {
    return readDurableUiSetting(ANTIGRAVITY_SEAMLESS_SWITCH_UNLOCK_STORAGE_KEY) === '1';
  } catch {
    return false;
  }
}

export function persistAntigravitySeamlessSwitchFeatureUnlocked(unlocked: boolean): void {
  previewDurableUiSetting(ANTIGRAVITY_SEAMLESS_SWITCH_UNLOCK_STORAGE_KEY, unlocked ? '1' : '0');
}
