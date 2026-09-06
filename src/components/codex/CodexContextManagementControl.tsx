import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Brain } from 'lucide-react';
import * as codexService from '../../services/codexService';
import * as codexInstanceService from '../../services/codexInstanceService';
import './CodexContextManagementControl.css';

type ControlVariant = 'quick' | 'settings';

interface CodexContextManagementControlProps {
  variant?: ControlVariant;
  /** 传入实例 ID 时只操作该实例的 CODEX_HOME；不传则操作默认官方实例。 */
  instanceId?: string;
  active?: boolean;
}

const CONTEXT_MANAGEMENT_UPDATED_EVENT = 'codex-context-management-updated';

export function CodexContextManagementControl({
  variant = 'settings',
  instanceId,
  active = true,
}: CodexContextManagementControlProps) {
  const { t } = useTranslation();
  const [enabled, setEnabled] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const load = useCallback(async () => {
    if (!active) return;
    setLoading(true);
    setError(null);
    try {
      const config = instanceId
        ? await codexInstanceService.getCodexInstanceQuickConfig(instanceId)
        : await codexService.getCodexQuickConfig();
      setEnabled(Boolean(config.context_management_experimental_mode));
      setLoaded(true);
    } catch (loadError) {
      setLoaded(false);
      setError(
        t('settings.general.codexContextManagementLoadFailed', {
          defaultValue: '读取官方上下文管理开关失败：{{error}}',
          error: String(loadError),
        }),
      );
    } finally {
      setLoading(false);
    }
  }, [active, instanceId, t]);

  useEffect(() => {
    if (active) {
      void load();
    } else {
      setLoading(false);
    }
  }, [active, load]);

  useEffect(() => {
    if (!active) return;
    const handleUpdated = (event: Event) => {
      const detail = (event as CustomEvent<{ instanceId?: string }>).detail;
      if ((detail?.instanceId ?? null) !== (instanceId ?? null)) return;
      void load();
    };
    window.addEventListener(CONTEXT_MANAGEMENT_UPDATED_EVENT, handleUpdated);
    return () => window.removeEventListener(CONTEXT_MANAGEMENT_UPDATED_EVENT, handleUpdated);
  }, [active, instanceId, load]);

  const handleChange = async (nextEnabled: boolean) => {
    if (!loaded || loading || saving) return;
    const previous = enabled;
    setEnabled(nextEnabled);
    setSaving(true);
    setError(null);
    setNotice(null);
    try {
      const saved = instanceId
        ? await codexInstanceService.saveCodexInstanceContextManagement(
            instanceId,
            nextEnabled,
          )
        : await codexService.saveCodexContextManagement(nextEnabled);
      setEnabled(Boolean(saved.context_management_experimental_mode));
      setNotice(t('settings.general.codexContextManagementSaveSuccess', '官方上下文管理开关已保存'));
      window.dispatchEvent(
        new CustomEvent(CONTEXT_MANAGEMENT_UPDATED_EVENT, {
          detail: { instanceId: instanceId ?? null },
        }),
      );
    } catch (saveError) {
      setEnabled(previous);
      setError(
        t('settings.general.codexContextManagementSaveFailed', {
          defaultValue: '保存官方上下文管理开关失败：{{error}}',
          error: String(saveError),
        }),
      );
    } finally {
      setSaving(false);
    }
  };

  const description = t(
    'settings.general.codexContextManagementDesc',
    '默认关闭。仅写入当前官方 Codex 配置的实验性上下文管理开关，完全重启 Codex 后生效；不修改上下文窗口、压缩阈值或 API 服务。',
  );
  const status = error || (saving ? t('common.saving', '保存中...') : notice);

  if (variant === 'quick') {
    return (
      <div className="codex-context-management-control codex-context-management-control--quick">
        <div className="qs-row qs-row--top">
          <div className="qs-row-label">
            <Brain size={15} />
            <span>
              {t(
                'settings.general.codexContextManagement',
                '开启experimental_mode = true（实验性上下文管理）',
              )}
            </span>
          </div>
          <div className="qs-row-control">
            <label className="qs-switch">
              <input
                type="checkbox"
                checked={enabled}
                disabled={!loaded || loading || saving}
                onChange={(event) => void handleChange(event.target.checked)}
                aria-label={t(
                  'settings.general.codexContextManagement',
                  '开启experimental_mode = true（实验性上下文管理）',
                )}
              />
              <span className="qs-switch-slider" />
            </label>
          </div>
        </div>
        <div className="qs-hint">{description}</div>
        {status && (
          <div className={`qs-codex-quick-status ${error ? 'error' : saving ? '' : 'success'}`}>
            {status}
          </div>
        )}
      </div>
    );
  }

  return (
    <div className="codex-context-management-control codex-context-management-control--settings">
      <div className="settings-row">
        <div className="row-label">
          <div className="row-title">
            {t(
              'settings.general.codexContextManagement',
              '开启experimental_mode = true（实验性上下文管理）',
            )}
          </div>
          <div className="row-desc">{description}</div>
          {status && (
            <div className={`codex-context-management-status ${error ? 'error' : 'success'}`}>
              {status}
            </div>
          )}
        </div>
        <div className="row-control">
          <label className="switch">
            <input
              type="checkbox"
              checked={enabled}
              disabled={!loaded || loading || saving}
              onChange={(event) => void handleChange(event.target.checked)}
              aria-label={t(
                'settings.general.codexContextManagement',
                '开启experimental_mode = true（实验性上下文管理）',
              )}
            />
            <span className="slider" />
          </label>
        </div>
      </div>
    </div>
  );
}
