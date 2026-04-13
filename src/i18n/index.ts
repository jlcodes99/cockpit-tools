import i18n from 'i18next';
import { initReactI18next } from 'react-i18next';
import enResources from '../locales/en.json';
import zhCnResources from '../locales/zh-CN.json';

type LocaleModule = { default: Record<string, unknown> };

const languageAliases: Record<string, string> = {
  'zh-CN': 'zh-cn',
  'zh-TW': 'zh-tw',
  'en-US': 'en',
  'pt-BR': 'pt-br',
  'vi-VN': 'vi',
  'vi-vn': 'vi',
  'id-ID': 'id',
  'id-id': 'id',
};

export const supportedLanguages = [
  'en',
  'zh-cn',
  'zh-tw',
  'ja',
  'es',
  'de',
  'fr',
  'pt-br',
  'ru',
  'ko',
  'it',
  'tr',
  'pl',
  'cs',
  'vi',
  'ar',
  'id',
];

const localeLoaders: Record<string, () => Promise<LocaleModule>> = {
  'zh-tw': () => import('../locales/zh-tw.json'),
  ja: () => import('../locales/ja.json'),
  es: () => import('../locales/es.json'),
  de: () => import('../locales/de.json'),
  fr: () => import('../locales/fr.json'),
  'pt-br': () => import('../locales/pt-br.json'),
  ru: () => import('../locales/ru.json'),
  ko: () => import('../locales/ko.json'),
  it: () => import('../locales/it.json'),
  tr: () => import('../locales/tr.json'),
  pl: () => import('../locales/pl.json'),
  cs: () => import('../locales/cs.json'),
  vi: () => import('../locales/vi.json'),
  ar: () => import('../locales/ar.json'),
  id: () => import('../locales/id.json'),
};

const loadedLanguages = new Set<string>();
let initPromise: Promise<void> | null = null;
let i18nBootstrapped = false;

export function normalizeLanguage(lang: string): string {
  const trimmed = lang.trim();
  if (!trimmed) {
    return 'zh-cn';
  }

  if (languageAliases[trimmed]) {
    return languageAliases[trimmed];
  }

  const lower = trimmed.toLowerCase();
  if (languageAliases[lower]) {
    return languageAliases[lower];
  }

  return lower;
}

function resolveSupportedLanguage(lang: string): string {
  const normalized = normalizeLanguage(lang);
  return supportedLanguages.includes(normalized) ? normalized : 'en';
}

async function ensureLanguageResources(lang: string): Promise<string> {
  const resolved = resolveSupportedLanguage(lang);
  if (loadedLanguages.has(resolved)) {
    return resolved;
  }

  const loader = localeLoaders[resolved];
  if (!loader) {
    loadedLanguages.add(resolved);
    return resolved;
  }
  const module = await loader();
  i18n.addResourceBundle(resolved, 'translation', module.default, true, true);
  loadedLanguages.add(resolved);
  return resolved;
}

function getSavedLanguage(): string {
  try {
    return resolveSupportedLanguage(localStorage.getItem('app-language') || 'en');
  } catch {
    return 'en';
  }
}

function getBootstrapLanguage(savedLanguage: string): string {
  if (savedLanguage === 'zh-cn') {
    return 'zh-cn';
  }
  return 'en';
}

function bootstrapI18n(savedLanguage: string): string {
  if (i18nBootstrapped) {
    return getBootstrapLanguage(savedLanguage);
  }

  const bootstrapLanguage = getBootstrapLanguage(savedLanguage);
  i18n
    .use(initReactI18next)
    .init({
      resources: {
        en: { translation: enResources },
        'zh-cn': { translation: zhCnResources },
      },
      lng: bootstrapLanguage,
      fallbackLng: 'en',
      supportedLngs: supportedLanguages,
      lowerCaseLng: true,
      load: 'currentOnly',
      initImmediate: false,
      interpolation: {
        escapeValue: false, // React 已经处理了 XSS
      },
    });

  loadedLanguages.add('en');
  loadedLanguages.add('zh-cn');
  i18nBootstrapped = true;
  return bootstrapLanguage;
}

export async function initI18n(): Promise<void> {
  if (initPromise) {
    return initPromise;
  }

  const savedLanguage = getSavedLanguage();
  const bootstrapLanguage = bootstrapI18n(savedLanguage);

  initPromise = (async () => {
    if (savedLanguage !== bootstrapLanguage) {
      await ensureLanguageResources(savedLanguage);
    }
    if (i18n.language !== savedLanguage) {
      await i18n.changeLanguage(savedLanguage);
    }
  })();

  return initPromise;
}
/**
 * 切换语言
 */
export async function changeLanguage(lang: string): Promise<void> {
  const resolved = await ensureLanguageResources(lang);
  await i18n.changeLanguage(resolved);
  localStorage.setItem('app-language', resolved);
}

/**
 * 获取当前语言
 */
export function getCurrentLanguage(): string {
  return normalizeLanguage(i18n.language || 'zh-CN');
}

export default i18n;
