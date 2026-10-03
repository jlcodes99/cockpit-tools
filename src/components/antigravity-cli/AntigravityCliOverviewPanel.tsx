import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ChevronDown, CircleAlert, ExternalLink, FileUp, Play, RefreshCw } from 'lucide-react';
import { openUrl } from '@tauri-apps/plugin-opener';
import type { AntigravityCliState, AntigravityCliStatusController } from '../../hooks/useAntigravityCliStatus';
import { AntigravityCliLaunchModal } from './AntigravityCliLaunchModal';
import '../../styles/components/antigravity-cli.css';

const FLOW_NOTICE_COLLAPSED_KEY = 'agtools.antigravity_cli.flow_notice_collapsed';
export const ANTIGRAVITY_CLI_INSTALL_URL = 'https://www.antigravity.google/docs/cli/install/';

const STATE_TONE: Record<AntigravityCliState, 'success' | 'warning' | 'danger' | 'muted'> = {
  loading: 'muted',
  error: 'danger',
  keyringError: 'danger',
  apiKey: 'warning',
  managed: 'success',
  notImported: 'warning',
  signedOut: 'muted',
};

function readNoticeCollapsed(): boolean {
  try {
    return localStorage.getItem(FLOW_NOTICE_COLLAPSED_KEY) === 'true';
  } catch {
    return false;
  }
}

interface AntigravityCliOverviewPanelProps {
  cli: AntigravityCliStatusController;
  disabled: boolean;
  onRefresh: () => void;
  onImport: () => void;
}

/** Antigravity CLI 子平台的说明、登录状态与启动入口。 */
export function AntigravityCliOverviewPanel({ cli, disabled, onRefresh, onImport }: AntigravityCliOverviewPanelProps) {
  const { t } = useTranslation();
  const [noticeCollapsed, setNoticeCollapsed] = useState(readNoticeCollapsed);
  const [launchOpen, setLaunchOpen] = useState(false);
  const [installError, setInstallError] = useState<string | null>(null);
  const { status, state, loading, error } = cli;

  const toggleNotice = () => {
    setNoticeCollapsed((prev) => {
      const next = !prev;
      try {
        localStorage.setItem(FLOW_NOTICE_COLLAPSED_KEY, String(next));
      } catch {
        // ignore persistence failures
      }
      return next;
    });
  };

  const stateLabel: Record<AntigravityCliState, string> = {
    loading: t('antigravityCli.state.loading', '检测中'),
    error: t('antigravityCli.state.error', '状态读取失败'),
    keyringError: t('antigravityCli.state.keyringError', '钥匙环异常'),
    apiKey: t('antigravityCli.state.apiKey', 'API Key 模式'),
    managed: t('antigravityCli.state.managed', '已登录'),
    notImported: t('antigravityCli.state.notImported', '已登录 · 未导入'),
    signedOut: t('antigravityCli.state.signedOut', '未登录'),
  };

  const stateDetail: Record<AntigravityCliState, string> = {
    loading: t('antigravityCli.loading', '正在读取 CLI 登录状态…'),
    error: error ?? '',
    keyringError: status?.credential_error ?? '',
    apiKey: t('antigravityCli.apiKeyMode'),
    managed: t('antigravityCli.managed'),
    notImported: t('antigravityCli.notImported'),
    signedOut: t('antigravityCli.notSignedIn'),
  };

  const installed = Boolean(status?.executable);
  const detail = installError ?? (status?.version_error && state !== 'error' ? status.version_error : null);

  return (
    <>
      <div className={`ghcp-flow-notice ${noticeCollapsed ? 'collapsed' : ''}`} role="note">
        <button type="button" className="ghcp-flow-notice-toggle" onClick={toggleNotice}>
          <div className="ghcp-flow-notice-title">
            <CircleAlert size={16} />
            <span>{t('antigravityCli.flowNotice.title', 'Antigravity CLI 账号管理说明（点击展开/收起）')}</span>
          </div>
          <ChevronDown size={16} className={`ghcp-flow-notice-arrow ${noticeCollapsed ? 'collapsed' : ''}`} />
        </button>
        {!noticeCollapsed && (
          <div className="ghcp-flow-notice-body">
            <div className="ghcp-flow-notice-desc">{t('antigravityCli.flowNotice.desc')}</div>
            <ul className="ghcp-flow-notice-list">
              <li>{t('antigravityCli.flowNotice.sharedCredential')}</li>
              <li>{t('antigravityCli.flowNotice.switching')}</li>
              <li>{t('antigravityCli.flowNotice.logout')}</li>
            </ul>
          </div>
        )}
      </div>

      <section
        className={`antigravity-cli-status tone-${STATE_TONE[state]}`}
        aria-label={t('antigravityCli.statusLabel', 'CLI 登录状态')}
      >
        <div className="antigravity-cli-status-main" role="status">
          <span className="antigravity-cli-status-dot" aria-hidden="true" />
          <div className="antigravity-cli-status-text">
            <span className="antigravity-cli-status-title">{stateLabel[state]}</span>
            {stateDetail[state] && <span className="antigravity-cli-status-detail">{stateDetail[state]}</span>}
            {detail && <span className="antigravity-cli-status-detail is-error">{detail}</span>}
          </div>
        </div>
        <div className="antigravity-cli-status-actions">
          {state === 'notImported' && (
            <button className="btn btn-secondary" onClick={onImport} disabled={disabled}>
              <FileUp size={14} />
              {t('antigravityCli.importLocal', '导入 CLI 账号')}
            </button>
          )}
          <button
            className="btn btn-secondary icon-only"
            onClick={onRefresh}
            disabled={loading || disabled}
            title={t('antigravityCli.refresh', '检查状态')}
            aria-label={t('antigravityCli.refresh', '检查状态')}
          >
            <RefreshCw size={14} className={loading ? 'loading-spinner' : ''} />
          </button>
          {installed || loading ? (
            <button
              className="btn btn-primary"
              onClick={() => setLaunchOpen(true)}
              disabled={!installed || disabled}
            >
              <Play size={14} />
              {t('antigravityCli.launch', '打开 CLI')}
            </button>
          ) : (
            <button
              className="btn btn-primary"
              onClick={() => {
                setInstallError(null);
                void openUrl(ANTIGRAVITY_CLI_INSTALL_URL).catch((e) => setInstallError(String(e)));
              }}
            >
              <ExternalLink size={14} />
              {t('antigravityCli.install', '安装指南')}
            </button>
          )}
        </div>
      </section>

      {launchOpen && status?.executable && (
        <AntigravityCliLaunchModal executable={status.executable} onClose={() => setLaunchOpen(false)} />
      )}
    </>
  );
}
