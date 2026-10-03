import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { RefreshCcw } from 'lucide-react';
import { useTranslation } from 'react-i18next';

type PiSyncConfig = { pi_sync_official_auth_on_switch?: boolean };

/** Toolbar toggle for "sync official login on switch" (pi has no quota settings). */
export function PiSyncToggle() {
  const { t } = useTranslation();
  const [enabled, setEnabled] = useState<boolean | null>(null);
  const [saving, setSaving] = useState(false);

  const load = useCallback(async () => {
    try {
      const config = await invoke<PiSyncConfig>('get_general_config');
      setEnabled(config.pi_sync_official_auth_on_switch ?? true);
    } catch (error) {
      console.error('Failed to load pi sync config:', error);
    }
  }, []);

  useEffect(() => {
    void load();
    const onUpdated = () => void load();
    window.addEventListener('config-updated', onUpdated);
    return () => window.removeEventListener('config-updated', onUpdated);
  }, [load]);

  const toggle = async () => {
    if (enabled == null || saving) return;
    const next = !enabled;
    setSaving(true);
    setEnabled(next);
    try {
      await invoke('patch_general_config', {
        updates: { pi_sync_official_auth_on_switch: next },
      });
      window.dispatchEvent(new Event('config-updated'));
    } catch (error) {
      console.error('Failed to save pi sync config:', error);
      setEnabled(!next);
    } finally {
      setSaving(false);
    }
  };

  const label = t('quickSettings.pi.syncOfficialAuthOnSwitch', '切号同步官方登录');
  const hint = t(
    'quickSettings.pi.syncOfficialAuthOnSwitchDesc',
    '开启后，切换账号会把凭据合并写入 ~/.pi/agent/auth.json 并更新默认模型；关闭时使用独立 PI_CODING_AGENT_DIR。',
  );

  return (
    <button
      type="button"
      className={`btn btn-secondary pi-sync-toggle${enabled ? ' is-on' : ''}`}
      role="switch"
      aria-checked={!!enabled}
      aria-label={label}
      title={`${label}: ${enabled ? 'ON' : 'OFF'}\n${hint}`}
      onClick={() => void toggle()}
      disabled={enabled == null || saving}
    >
      <RefreshCcw size={14} />
      <span>{label}</span>
    </button>
  );
}
