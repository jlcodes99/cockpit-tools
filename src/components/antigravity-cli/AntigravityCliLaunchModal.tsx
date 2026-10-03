import { useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Check, ExternalLink, FolderOpen, Play, RefreshCw, X } from 'lucide-react';
import { open } from '@tauri-apps/plugin-dialog';
import { openUrl } from '@tauri-apps/plugin-opener';
import { launchAntigravityCli } from '../../services/antigravityCliService';
import { useEscCloseTopmost } from '../../hooks/useEscClose';
import { useModalFocusTrap } from '../../hooks/useModalFocusTrap';
import '../../styles/components/antigravity-cli.css';

const WORKING_DIR_STORAGE_KEY = 'agtools.antigravity_cli.working_dir.v1';
export const ANTIGRAVITY_CLI_INSTALL_URL = 'https://www.antigravity.google/docs/cli/install/';

function readLastWorkingDir(): string {
  try {
    return localStorage.getItem(WORKING_DIR_STORAGE_KEY) ?? '';
  } catch {
    return '';
  }
}

function persistLastWorkingDir(value: string): void {
  try {
    if (value) localStorage.setItem(WORKING_DIR_STORAGE_KEY, value);
    else localStorage.removeItem(WORKING_DIR_STORAGE_KEY);
  } catch {
    // ignore persistence failures
  }
}

interface AntigravityCliLaunchModalProps {
  /** null 表示未检测到 agy，弹窗改为引导安装。 */
  executable: string | null;
  /** 状态仍在检测中，暂不判定是否已安装。 */
  detecting: boolean;
  onClose: () => void;
}

export function AntigravityCliLaunchModal({ executable, detecting, onClose }: AntigravityCliLaunchModalProps) {
  const { t } = useTranslation();
  const dialogRef = useRef<HTMLDivElement>(null);
  const [workingDir, setWorkingDir] = useState(readLastWorkingDir);
  const [launching, setLaunching] = useState(false);
  const [launched, setLaunched] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEscCloseTopmost(true, onClose);
  useModalFocusTrap(dialogRef, true);

  const chooseDirectory = async () => {
    try {
      const selected = await open({ directory: true, multiple: false });
      if (typeof selected === 'string') setWorkingDir(selected);
    } catch (e) {
      setError(String(e));
    }
  };

  const openInstallGuide = () => {
    setError(null);
    void openUrl(ANTIGRAVITY_CLI_INSTALL_URL).catch((e) => setError(String(e)));
  };

  const launch = async () => {
    if (!executable) return;
    const directory = workingDir.trim();
    setLaunching(true);
    setLaunched(false);
    setError(null);
    try {
      await launchAntigravityCli(directory || undefined);
      persistLastWorkingDir(directory);
      setLaunched(true);
    } catch (e) {
      setError(String(e));
    } finally {
      setLaunching(false);
    }
  };

  return (
    <div className="modal-overlay">
      <div
        ref={dialogRef}
        className="modal antigravity-cli-launch-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="antigravity-cli-launch-title"
        tabIndex={-1}
        onClick={(event) => event.stopPropagation()}
      >
        <div className="modal-header">
          <h2 id="antigravity-cli-launch-title">{t('antigravityCli.launchTitle', '打开 Antigravity CLI')}</h2>
          <button className="modal-close" onClick={onClose} aria-label={t('common.close', '关闭')}>
            <X />
          </button>
        </div>
        <div className="modal-body">
          <div className="form-group">
            <label>{t('antigravityCli.executable', '可执行文件')}</label>
            <input
              className="form-input antigravity-cli-mono"
              value={executable ?? ''}
              placeholder={detecting ? t('common.loading', '加载中...') : t('antigravityCli.notInstalled', '未检测到 agy')}
              readOnly
            />
          </div>
          {!executable && !detecting && (
            <p className="antigravity-cli-hint">{t('antigravityCli.installHint')}</p>
          )}
          {executable && (
            <div className="form-group">
              <label htmlFor="antigravity-cli-working-dir">{t('antigravityCli.directory', '工作目录')}</label>
              <div className="antigravity-cli-directory-row">
                <input
                  id="antigravity-cli-working-dir"
                  className="form-input"
                  value={workingDir}
                  placeholder={t('antigravityCli.homeDirectory', '默认使用用户主目录')}
                  onChange={(event) => setWorkingDir(event.target.value)}
                  disabled={launching}
                />
                <button
                  type="button"
                  className="btn btn-secondary icon-only"
                  onClick={() => void chooseDirectory()}
                  disabled={launching}
                  title={t('antigravityCli.chooseDirectory', '选择工作目录')}
                  aria-label={t('antigravityCli.chooseDirectory', '选择工作目录')}
                >
                  <FolderOpen size={16} />
                </button>
              </div>
              <p className="antigravity-cli-hint">
                {t('antigravityCli.terminalHint', '使用「设置 → 默认终端」中配置的终端打开。')}
              </p>
            </div>
          )}
          {launched && (
            <div className="antigravity-cli-feedback success" role="status">
              <Check size={16} />
              <span>{t('antigravityCli.launched', '已请求打开终端，如需登录请在 agy 中完成。')}</span>
            </div>
          )}
          {error && (
            <div className="antigravity-cli-feedback error" role="alert">
              <span>{error}</span>
            </div>
          )}
        </div>
        <div className="modal-footer">
          <button className="btn btn-secondary" onClick={onClose}>
            {t('common.close', '关闭')}
          </button>
          {executable || detecting ? (
            <button className="btn btn-primary" onClick={() => void launch()} disabled={launching || !executable}>
              {launching ? <RefreshCw size={16} className="loading-spinner" /> : <Play size={16} />}
              {t('antigravityCli.launch', '打开 CLI')}
            </button>
          ) : (
            <button className="btn btn-primary" onClick={openInstallGuide}>
              <ExternalLink size={16} />
              {t('antigravityCli.install', '安装指南')}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
