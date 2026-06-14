import type { TFunction } from 'i18next';
import type { Page } from '../types/navigation';
import { PLATFORM_PAGE_MAP, type PlatformId } from '../types/platform';
import {
  resolveEntryDefaultPlatformId,
  type PlatformLayoutEntryId,
  type PlatformLayoutGroup,
} from '../stores/usePlatformLayoutStore';
import { getPlatformLabel } from './platformMeta';

export const LAST_CLOSED_STARTUP_PAGE = 'last_closed' as const;
export const DEFAULT_STARTUP_PAGE: Page = 'dashboard';

export type StartupPageValue = Page | typeof LAST_CLOSED_STARTUP_PAGE;

export interface StartupPageOption {
  value: StartupPageValue;
  label: string;
}

export interface StartupPageCandidateInput {
  sidebarEntryIds: PlatformLayoutEntryId[];
  platformGroups: PlatformLayoutGroup[];
  apiRelayVisible: boolean;
}

export interface StartupPageResolution {
  page: Page;
  correctedStartupPage?: StartupPageValue;
  correctedLastClosedPage?: Page;
}

const PAGE_VALUES: readonly Page[] = [
  'dashboard',
  'manual',
  'api-relay',
  'overview',
  'codex',
  'codex-api-service',
  'zed',
  'github-copilot',
  'windsurf',
  'kiro',
  'cursor',
  'gemini',
  'codebuddy',
  'codebuddy-cn',
  'qoder',
  'trae',
  'workbuddy',
  'codex-instances',
  'instances',
  'accounts',
  'wakeup',
  'verification',
  '2fa',
  'settings',
];

const PLATFORM_BY_PAGE = Object.entries(PLATFORM_PAGE_MAP).reduce(
  (result, [platformId, page]) => {
    if (!result[page]) {
      result[page] = platformId as PlatformId;
    }
    return result;
  },
  {} as Partial<Record<Page, PlatformId>>,
);

function isPage(value: string): value is Page {
  return PAGE_VALUES.includes(value as Page);
}

export function normalizeStartupPageValue(value?: string | null): StartupPageValue {
  const normalized = value?.trim();
  if (normalized === LAST_CLOSED_STARTUP_PAGE) {
    return LAST_CLOSED_STARTUP_PAGE;
  }
  if (normalized && isPage(normalized)) {
    return normalized;
  }
  return DEFAULT_STARTUP_PAGE;
}

export function normalizePageValue(value?: string | null): Page {
  const normalized = value?.trim();
  return normalized && isPage(normalized) ? normalized : DEFAULT_STARTUP_PAGE;
}

export function buildStartupPageCandidates(input: StartupPageCandidateInput): Page[] {
  const pages: Page[] = [DEFAULT_STARTUP_PAGE, 'manual', 'settings'];
  if (input.apiRelayVisible) {
    pages.push('api-relay');
  }

  for (const entryId of input.sidebarEntryIds) {
    const platformId = resolveEntryDefaultPlatformId(entryId, input.platformGroups);
    if (!platformId) {
      continue;
    }
    pages.push(PLATFORM_PAGE_MAP[platformId]);
  }

  return Array.from(new Set(pages));
}

export function resolveStartupPage(
  startupPage: string | undefined,
  lastClosedPage: string | undefined,
  candidates: Page[],
): StartupPageResolution {
  const candidateSet = new Set(candidates);
  const rawStartupPage = startupPage?.trim();
  const normalizedStartupPage = normalizeStartupPageValue(startupPage);

  if (normalizedStartupPage === LAST_CLOSED_STARTUP_PAGE) {
    const normalizedLastClosedPage = normalizePageValue(lastClosedPage);
    if (candidateSet.has(normalizedLastClosedPage)) {
      return { page: normalizedLastClosedPage };
    }
    return {
      page: DEFAULT_STARTUP_PAGE,
      correctedLastClosedPage: DEFAULT_STARTUP_PAGE,
    };
  }

  if (rawStartupPage && rawStartupPage !== normalizedStartupPage) {
    return {
      page: DEFAULT_STARTUP_PAGE,
      correctedStartupPage: DEFAULT_STARTUP_PAGE,
    };
  }

  if (candidateSet.has(normalizedStartupPage)) {
    return { page: normalizedStartupPage };
  }

  return {
    page: DEFAULT_STARTUP_PAGE,
    correctedStartupPage: DEFAULT_STARTUP_PAGE,
  };
}

export function getStartupPageLabel(value: StartupPageValue, t: TFunction): string {
  if (value === LAST_CLOSED_STARTUP_PAGE) {
    return t('settings.general.startupPageLastClosed', '上次关闭页面');
  }

  switch (value) {
    case 'dashboard':
      return t('nav.dashboard', '仪表盘');
    case 'manual':
      return t('nav.manual', '使用手册');
    case 'settings':
      return t('nav.settings', '设置');
    case 'api-relay':
      return t('nav.apiRelay', '中转站');
    default: {
      const platformId = PLATFORM_BY_PAGE[value];
      return platformId ? getPlatformLabel(platformId, t) : value;
    }
  }
}

export function buildStartupPageOptions(input: StartupPageCandidateInput, t: TFunction): StartupPageOption[] {
  return [
    ...buildStartupPageCandidates(input).map((page) => ({
      value: page,
      label: getStartupPageLabel(page, t),
    })),
    {
      value: LAST_CLOSED_STARTUP_PAGE,
      label: getStartupPageLabel(LAST_CLOSED_STARTUP_PAGE, t),
    },
  ];
}
