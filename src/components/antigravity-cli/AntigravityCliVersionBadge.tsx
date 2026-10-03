import { useTranslation } from 'react-i18next';
import type { AntigravityCliStatus } from '../../services/antigravityCliService';

interface AntigravityCliVersionBadgeProps {
  status: AntigravityCliStatus | null;
  loading: boolean;
}

/** 与 AntigravityInstalledVersionBadge 相同的展示，数据来源为 agy 的检测结果。 */
export function AntigravityCliVersionBadge({ status, loading }: AntigravityCliVersionBadgeProps) {
  const { t } = useTranslation();

  if (!status && loading) {
    return (
      <div
        className="installed-version-badge is-loading"
        title={t('runtime.installedVersion.loading', '正在检测安装版本')}
      >
        <span className="installed-version-dot" />
        <span className="installed-version-value">
          {t('runtime.installedVersion.detecting', '检测中')}
        </span>
      </div>
    );
  }

  if (!status?.executable || !status.version) {
    const title = status?.executable
      ? `${status.executable}\n${status.version_error ?? ''}`.trim()
      : t('antigravityCli.notInstalled', '未检测到 agy');
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
    <div className="installed-version-badge" title={`Antigravity CLI v${status.version}\n${status.executable}`}>
      <span className="installed-version-dot" />
      <span className="installed-version-name">agy</span>
      <span className="installed-version-value">v{status.version}</span>
    </div>
  );
}
