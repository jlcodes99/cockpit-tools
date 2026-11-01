import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync, readdirSync } from 'node:fs';
import type { AnnouncementState } from '../types/announcement';
import {
  APIKEY_FUN_CAMPAIGN,
  filterApiKeyFunCampaignAnnouncement,
  getApiKeyFunCampaignRefreshDelay,
  isApiKeyFunCampaignActive,
} from './apiKeyFunCampaign.ts';
import { APIKEY_FUN_REGISTER_URL, normalizeApiKeyFunOfficialUrl } from './apikeyFunLinks.ts';

test('campaign includes all of November 1–11 in Beijing time and excludes both outer boundaries', () => {
  for (const [time, expected] of [
    ['2026-10-31T23:59:59.999+08:00', false],
    ['2026-11-01T00:00:00+08:00', true],
    ['2026-11-11T23:59:59.999+08:00', true],
    ['2026-11-12T00:00:00+08:00', false],
    ['2027-11-01T00:00:00+08:00', false],
  ] as const) {
    assert.equal(isApiKeyFunCampaignActive(Date.parse(time)), expected, time);
  }
});

test('campaign uses absolute instants, independent of the computer timezone', () => {
  assert.equal(isApiKeyFunCampaignActive(Date.parse('2026-10-31T16:00:00Z')), true);
  assert.equal(isApiKeyFunCampaignActive(Date.parse('2026-10-31T09:00:00-07:00')), true);
  assert.equal(isApiKeyFunCampaignActive(Date.parse('2026-11-11T16:00:00Z')), false);
});

test('refresh wakes at a boundary and caps clock-change detection at one minute', () => {
  const start = Date.parse(APIKEY_FUN_CAMPAIGN.startsAt);
  const end = Date.parse(APIKEY_FUN_CAMPAIGN.expiresAt);
  assert.equal(getApiKeyFunCampaignRefreshDelay(start - 1), 1);
  assert.equal(getApiKeyFunCampaignRefreshDelay(end - 1234), 1234);
  for (const now of [start - 86_400_000, start, end, end + 86_400_000]) {
    assert.equal(getApiKeyFunCampaignRefreshDelay(now), 60_000);
  }
});

test('cached campaign announcement and unread badge disappear outside the period without mutating other notices', () => {
  const notice = (id: string) => ({ id, type: 'info', title: id, summary: '', content: '', priority: 1, targetVersions: '*', popup: false, createdAt: APIKEY_FUN_CAMPAIGN.startsAt });
  const campaign = notice(APIKEY_FUN_CAMPAIGN.id);
  const other = notice('other');
  const state: AnnouncementState = { announcements: [campaign, other], unreadIds: [campaign.id, other.id], popupAnnouncement: campaign };
  assert.equal(filterApiKeyFunCampaignAnnouncement(state, true), state);
  assert.deepEqual(filterApiKeyFunCampaignAnnouncement(state, false), { announcements: [other], unreadIds: [other.id], popupAnnouncement: null });
  assert.equal(state.announcements.length, 2);
  assert.equal(filterApiKeyFunCampaignAnnouncement({ ...state, popupAnnouncement: other }, false).popupAnnouncement, other);
});

test('remote announcement is available to all versions and languages, with matching dates and affiliate URL', () => {
  const feed = JSON.parse(readFileSync(new URL('../../announcements.json', import.meta.url), 'utf8'));
  const notices = feed.announcements.filter((item: { id: string }) => item.id === APIKEY_FUN_CAMPAIGN.id);
  assert.equal(notices.length, 1);
  const notice = notices[0];
  assert.equal(notice.targetVersions, '*');
  assert.deepEqual(notice.targetLanguages, ['*']);
  assert.equal(notice.createdAt, APIKEY_FUN_CAMPAIGN.startsAt);
  assert.equal(notice.expiresAt, APIKEY_FUN_CAMPAIGN.expiresAt);
  assert.equal(notice.action.type, 'url');
  assert.equal(notice.action.target, APIKEY_FUN_CAMPAIGN.url);
  assert.equal(notice.popup, false);
  for (const file of readdirSync(new URL('../locales/', import.meta.url)).filter((name) => name.endsWith('.json'))) {
    const locale = JSON.parse(readFileSync(new URL(`../locales/${file}`, import.meta.url), 'utf8'));
    const translated = notice.locales[file.slice(0, -5)];
    assert.ok(translated, `Missing announcement locale: ${file}`);
    assert.equal(translated.actionLabel, locale.apiKeyFun.campaign.action);
    assert.ok(translated.content.includes(locale.apiKeyFun.campaign.period));
  }
});

test('campaign link retains uppercase affiliate code while ordinary registration stays unchanged', () => {
  assert.equal(new URL(APIKEY_FUN_CAMPAIGN.url).searchParams.get('aff'), 'COCKPIT');
  assert.equal(normalizeApiKeyFunOfficialUrl(APIKEY_FUN_CAMPAIGN.url), APIKEY_FUN_CAMPAIGN.url);
  assert.equal(normalizeApiKeyFunOfficialUrl('https://apikey.fan/register'), APIKEY_FUN_REGISTER_URL);
});
