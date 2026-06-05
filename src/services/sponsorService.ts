import { invoke } from '@tauri-apps/api/core';
import type { SponsorModuleState } from '../types/sponsor';

export async function getSponsorModuleState(): Promise<SponsorModuleState> {
  return await invoke('announcement_get_sponsor_module');
}

export async function forceRefreshSponsorModuleState(): Promise<SponsorModuleState> {
  return await invoke('announcement_force_refresh_sponsor_module');
}
