import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  AntigravityInstalledVersionInfo,
  getAntigravityInstalledVersionInfo,
} from '../services/antigravityRuntimeService';
import { useAntigravityRuntimeTarget } from '../hooks/useAntigravityRuntimeTarget';
import type { AntigravityRuntimeTarget } from '../utils/antigravityRuntimeTarget';

const INSTALLED_VERSION_DEFER_MS = 250;

export function AntigravityInstalledVersionBadge() {
  const { t } = useTranslation();
  const runtimeTarget = useAntigravityRuntimeTarget();
  const [scan, setScan] = useState<{
    target: AntigravityRuntimeTarget;
    info: AntigravityInstalledVersionInfo | null;
  } | null>(null);
  const loaded = scan?.target === runtimeTarget;
  const info = loaded ? scan.info : null;
  const desktopLabel = t('instances.form.launchModeApp', 'Desktop');
  const isDesktop = runtimeTarget === 'antigravity';
  const productName = info?.product_name || (isDesktop ? 'Antigravity' : 'Antigravity IDE');
  const label = isDesktop ? desktopLabel : productName;
  const titleName = isDesktop ? `${productName} · ${desktopLabel}` : productName;

  useEffect(() => {
    let cancelled = false;
    setScan(null);

    const loadVersion = async () => {
      let quickInfo: AntigravityInstalledVersionInfo | null = null;
      try {
        quickInfo = await getAntigravityInstalledVersionInfo(runtimeTarget, 'quick');
        if (!cancelled && quickInfo) {
          setScan({ target: runtimeTarget, info: quickInfo });
        }
      } catch (error) {
        console.warn('[AntigravityInstalledVersionBadge] failed to load installed version:', error);
      }

      if (cancelled) return;
      let fullInfo: AntigravityInstalledVersionInfo | null = null;
      try {
        fullInfo = await getAntigravityInstalledVersionInfo(runtimeTarget, 'full');
      } catch (error) {
        console.warn('[AntigravityInstalledVersionBadge] failed to complete installed version scan:', error);
      }
      if (!cancelled) {
        setScan({ target: runtimeTarget, info: fullInfo ?? quickInfo });
      }
    };

    const timer = window.setTimeout(() => {
      void loadVersion();
    }, INSTALLED_VERSION_DEFER_MS);

    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [runtimeTarget]);

  const version = info?.version;
  const title = !loaded
    ? t('runtime.installedVersion.loading', '正在检测安装版本')
    : version
      ? `v${version}\n${info.app_path}`
      : t('runtime.installedVersion.missing', '未检测到已安装版本');
  const statusClass = !loaded ? ' is-loading' : version ? '' : ' is-missing';

  return (
    <div className={`installed-version-badge${statusClass}`} title={`${titleName} ${title}`}>
      <span className="installed-version-dot" />
      <span className="installed-version-name">{label}</span>
      <span className="installed-version-value">
        {!loaded
          ? t('runtime.installedVersion.detecting', '检测中')
          : version
            ? `v${version}`
            : t('runtime.installedVersion.notFound', '未检测到版本')}
      </span>
    </div>
  );
}
