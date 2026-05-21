import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  AntigravityInstalledVersionInfo,
  getAntigravityInstalledVersionInfo,
} from '../services/antigravityRuntimeService';
import { useAntigravityRuntimeTarget } from '../hooks/useAntigravityRuntimeTarget';

const INSTALLED_VERSION_DEFER_MS = 250;

export function AntigravityInstalledVersionBadge() {
  const { t } = useTranslation();
  const runtimeTarget = useAntigravityRuntimeTarget();
  const [info, setInfo] = useState<AntigravityInstalledVersionInfo | null>(null);
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    let cancelled = false;
    let timer = 0;
    setLoaded(false);

    const loadVersion = async () => {
      try {
        const quickInfo = await getAntigravityInstalledVersionInfo(runtimeTarget, 'quick');
        if (!cancelled) {
          setInfo(quickInfo);
          setLoaded(!!quickInfo);
        }
      } catch (error) {
        console.warn('[AntigravityInstalledVersionBadge] failed to load installed version:', error);
        if (!cancelled) {
          setInfo(null);
        }
      }

      try {
        const fullInfo = await getAntigravityInstalledVersionInfo(runtimeTarget, 'full');
        if (!cancelled) {
          if (fullInfo) {
            setInfo(fullInfo);
          }
          setLoaded(true);
        }
      } catch (error) {
        console.warn('[AntigravityInstalledVersionBadge] failed to complete installed version scan:', error);
        if (!cancelled) {
          setLoaded(true);
        }
      }
    };

    timer = window.setTimeout(() => {
      void loadVersion();
    }, INSTALLED_VERSION_DEFER_MS);

    return () => {
      cancelled = true;
      if (timer) {
        window.clearTimeout(timer);
      }
    };
  }, [runtimeTarget]);

  const title = useMemo(() => {
    if (!loaded) {
      return t('runtime.installedVersion.loading', '正在检测安装版本');
    }
    if (!info?.version) {
      return t('runtime.installedVersion.missing', '未检测到已安装版本');
    }
    return `${info.product_name || 'Antigravity'} v${info.version}\n${info.app_path || ''}`;
  }, [info, loaded, t]);

  if (!loaded) {
    return (
      <div className="installed-version-badge is-loading" title={title}>
        <span className="installed-version-dot" />
        <span className="installed-version-value">
          {t('runtime.installedVersion.detecting', '检测中')}
        </span>
      </div>
    );
  }

  if (!info?.version) {
    return (
      <div className="installed-version-badge is-missing" title={title}>
        <span className="installed-version-dot" />
        <span className="installed-version-value">
          {t('runtime.installedVersion.notFound', '未检测到版本')}
        </span>
      </div>
    );
  }

  return (
    <div className="installed-version-badge" title={title}>
      <span className="installed-version-dot" />
      <span className="installed-version-name">{info.product_name || 'Antigravity'}</span>
      <span className="installed-version-value">v{info.version}</span>
    </div>
  );
}
