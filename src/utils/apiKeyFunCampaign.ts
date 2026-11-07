import type { AnnouncementState } from '../types/announcement';

// Beijing time; the end is exclusive so November 11 is included in full.
export const APIKEY_FUN_CAMPAIGN = {
  id: 'apikey-fan-double11-2026',
  startsAt: '2026-11-01T00:00:00+08:00',
  expiresAt: '2026-11-12T00:00:00+08:00',
  url: 'https://apikey.fan/activities/double11-2026?aff=COCKPIT',
} as const;

const startsAt = Date.parse(APIKEY_FUN_CAMPAIGN.startsAt);
const expiresAt = Date.parse(APIKEY_FUN_CAMPAIGN.expiresAt);

export function isApiKeyFunCampaignActive(now = Date.now()): boolean {
  return now >= startsAt && now < expiresAt;
}

export function getApiKeyFunCampaignRefreshDelay(now = Date.now()): number {
  const nextBoundary = now < startsAt ? startsAt : now < expiresAt ? expiresAt : Infinity;
  // Recheck clock changes without network requests; also wake at the exact boundary.
  return Math.min(60_000, nextBoundary - now);
}

export function filterApiKeyFunCampaignAnnouncement(state: AnnouncementState, active: boolean): AnnouncementState {
  if (active) return state;
  return {
    announcements: state.announcements.filter((item) => item.id !== APIKEY_FUN_CAMPAIGN.id),
    unreadIds: state.unreadIds.filter((id) => id !== APIKEY_FUN_CAMPAIGN.id),
    popupAnnouncement: state.popupAnnouncement?.id === APIKEY_FUN_CAMPAIGN.id ? null : state.popupAnnouncement,
  };
}
