import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Settings } from 'lucide-react';
import { useTranslation } from 'react-i18next';

interface QoderCnQuickConfig {
  qoder_cn_auto_refresh_minutes: number;
  qoder_cn_quota_alert_enabled: boolean;
  qoder_cn_quota_alert_threshold: number;
}

/** CN native clients have their own refresh preference and OS-managed profile paths. */
export function QoderCnQuickSettings() {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [minutes, setMinutes] = useState('10');
  const [alertEnabled, setAlertEnabled] = useState(false);
  const [alertThreshold, setAlertThreshold] = useState('20');
  const [error, setError] = useState('');
  const [saving, setSaving] = useState(false);

  const loadConfig = useCallback(async () => {
    try {
      const config = await invoke<QoderCnQuickConfig>('get_general_config');
      setMinutes(String(config.qoder_cn_auto_refresh_minutes));
      setAlertEnabled(Boolean(config.qoder_cn_quota_alert_enabled));
      setAlertThreshold(String(config.qoder_cn_quota_alert_threshold));
      setError('');
      setOpen(true);
    } catch (err) {
      setError(String(err));
    }
  }, []);

  const toggle = async () => {
    if (open) {
      setOpen(false);
      return;
    }
    await loadConfig();
  };

  useEffect(() => {
    const handleExternalOpen = (event: Event) => {
      const detail = (event as CustomEvent<{ type?: string }>).detail;
      if (detail?.type === 'qoder_cn') {
        void loadConfig();
      }
    };
    window.addEventListener('quick-settings:open', handleExternalOpen);
    return () => window.removeEventListener('quick-settings:open', handleExternalOpen);
  }, [loadConfig]);

  const save = async () => {
    const value = Number(minutes);
    const threshold = Number(alertThreshold);
    if (!Number.isInteger(value) || (value !== -1 && (value < 1 || value > 999))) {
      // 之前是静默 return：按钮没反应、也没有任何红字，用户无从得知原因。
      setError(t('quickSettings.validation.refreshIntervalInvalid'));
      return;
    }
    if (!Number.isInteger(threshold) || threshold < 0 || threshold > 100) {
      setError(t('quickSettings.validation.quotaAlertThresholdInvalid'));
      return;
    }
    setError('');
    setSaving(true);
    try {
      await invoke('patch_general_config', {
        updates: {
          qoder_cn_auto_refresh_minutes: value,
          qoder_cn_quota_alert_enabled: alertEnabled,
          qoder_cn_quota_alert_threshold: threshold,
        },
      });
      window.dispatchEvent(new Event('config-updated'));
      setOpen(false);
      setError('');
    } catch (err) {
      setError(String(err));
    } finally {
      setSaving(false);
    }
  };

  return <div style={{ position: 'relative' }}>
    <button className="btn btn-secondary" title="Qoder CN" onClick={() => void toggle()}><Settings size={16} /></button>
    {open && <div className="quick-settings-popover" style={{ position: 'absolute', right: 0, zIndex: 50, minWidth: 270, padding: 16, background: 'var(--bg-primary)', border: '1px solid var(--border-color)', borderRadius: 12 }}>
      <strong>Qoder CN</strong>
      <label style={{ display: 'block', margin: '12px 0' }}>
        {t('quickSettings.refreshInterval', '配额自动刷新')}
        <input aria-label="Qoder CN refresh interval" type="number" value={minutes} disabled={saving} onChange={event => setMinutes(event.target.value)} style={{ width: 70, marginLeft: 8 }} />
        {t('wakeup.refreshInterval.minutes', '分钟')}
      </label>
      <p>{t('quickSettings.disableRefreshHint', '-1 表示禁用')}</p>
      <label style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 8 }}>
        <input aria-label="Qoder CN quota alert" type="checkbox" checked={alertEnabled} disabled={saving} onChange={event => setAlertEnabled(event.target.checked)} />
        {t('quickSettings.quotaAlert.enable', '超额预警')}
      </label>
      {alertEnabled && <label style={{ display: 'block', marginBottom: 12 }}>
        {t('quickSettings.quotaAlert.threshold', '预警阈值')}
        <input aria-label="Qoder CN quota alert threshold" type="number" min={0} max={100} value={alertThreshold} disabled={saving} onChange={event => setAlertThreshold(event.target.value)} style={{ width: 70, marginLeft: 8 }} />
        %
      </label>}
      <button className="btn btn-primary" disabled={saving} onClick={() => void save()}>{t('common.save', '保存')}</button>
    </div>}
    {error && <div role="alert">{error}</div>}
  </div>;
}
