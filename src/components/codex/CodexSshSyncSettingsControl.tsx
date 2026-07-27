import { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Server, Settings2 } from 'lucide-react';
import { useSshServerStore } from '../../stores/useSshServerStore';
import { CodexSshSyncModal } from './CodexSshSyncModal';

type ControlVariant = 'quick' | 'settings';

interface CodexSshSyncSettingsControlProps {
  variant?: ControlVariant;
}

/**
 * Codex 设置中的「切号同步 SSH」开关 + 配置入口。
 * 打开开关会弹出配置弹框；关闭开关会取消选中服务器（停止切号自动同步）。
 */
export function CodexSshSyncSettingsControl({
  variant = 'settings',
}: CodexSshSyncSettingsControlProps) {
  const { t } = useTranslation();
  const servers = useSshServerStore((s) => s.servers);
  const selectedServerId = useSshServerStore((s) => s.selectedServerId);
  const fetchServers = useSshServerStore((s) => s.fetchServers);
  const selectServer = useSshServerStore((s) => s.selectServer);
  const [modalOpen, setModalOpen] = useState(false);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    void fetchServers();
  }, [fetchServers]);

  const selectedServer = useMemo(
    () => servers.find((server) => server.id === selectedServerId) ?? null,
    [servers, selectedServerId],
  );

  const enabled = Boolean(
    selectedServer && selectedServer.sync_on_codex_switch,
  );

  const openModal = useCallback(() => setModalOpen(true), []);
  const closeModal = useCallback(() => setModalOpen(false), []);

  const handleToggle = async (next: boolean) => {
    if (busy) return;
    if (next) {
      setModalOpen(true);
      return;
    }
    setBusy(true);
    try {
      // 关闭：取消选中，后端切号同步不会再推到任何 SSH 主机
      await selectServer(null);
    } catch (error) {
      console.warn('[Codex SSH] disable sync failed:', error);
    } finally {
      setBusy(false);
    }
  };

  const hintText = enabled && selectedServer
    ? t('codex.ssh.syncSwitchActive', '已启用 · {{name}}（{{user}}@{{host}}）', {
        name: selectedServer.name,
        user: selectedServer.username,
        host: selectedServer.host,
      })
    : t(
        'codex.ssh.syncSwitchDesc',
        '开启后配置 SSH 主机；Codex 切号时把当前账号同步到远端。',
      );

  if (variant === 'quick') {
    return (
      <>
        <div className="qs-row" style={{ marginTop: 8 }}>
          <div className="qs-row-label">
            <Server size={15} />
            <span>{t('codex.ssh.syncSwitch', '切号同步 SSH')}</span>
          </div>
          <div className="qs-row-control" style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
            <button
              type="button"
              className="btn btn-secondary icon-only"
              title={t('codex.ssh.configure', '配置')}
              aria-label={t('codex.ssh.configure', '配置')}
              onClick={openModal}
            >
              <Settings2 size={14} />
            </button>
            <label className="qs-switch">
              <input
                type="checkbox"
                checked={enabled}
                disabled={busy}
                onChange={(e) => void handleToggle(e.target.checked)}
              />
              <span className="qs-switch-slider" />
            </label>
          </div>
        </div>
        <div className="qs-hint">{hintText}</div>
        <CodexSshSyncModal open={modalOpen} onClose={closeModal} />
      </>
    );
  }

  return (
    <>
      <div className="settings-row">
        <div className="row-label">
          <div className="row-title">{t('codex.ssh.syncSwitch', '切号同步 SSH')}</div>
          <div className="row-desc">{hintText}</div>
        </div>
        <div className="row-control" style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
          <button
            type="button"
            className="btn btn-secondary"
            onClick={openModal}
          >
            <Settings2 size={14} />
            <span>{t('codex.ssh.configure', '配置')}</span>
          </button>
          <label className="switch">
            <input
              type="checkbox"
              checked={enabled}
              disabled={busy}
              onChange={(e) => void handleToggle(e.target.checked)}
            />
            <span className="slider" />
          </label>
        </div>
      </div>
      <CodexSshSyncModal open={modalOpen} onClose={closeModal} />
    </>
  );
}
