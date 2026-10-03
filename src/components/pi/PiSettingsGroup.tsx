import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { Save } from "lucide-react";
import * as piInstanceService from "../../services/piInstanceService";
import type { PiCliStatus } from "../../services/piInstanceService";

interface PiGeneralConfig {
  pi_sync_official_auth_on_switch?: boolean;
  pi_auto_refresh_minutes?: number;
}

/** Self-contained pi block for the general settings panel. */
export function PiSettingsGroup({ order }: { order?: number }) {
  const { t } = useTranslation();
  const [cliStatus, setCliStatus] = useState<PiCliStatus | null>(null);
  const [cliPath, setCliPath] = useState("");
  const [cliSaving, setCliSaving] = useState(false);
  const [cliError, setCliError] = useState<string | null>(null);
  const [syncOfficial, setSyncOfficial] = useState(true);
  const [autoRefresh, setAutoRefresh] = useState("10");
  const loadedRef = useRef(false);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const [status, config] = await Promise.all([
          piInstanceService.getPiCliStatus(),
          invoke<PiGeneralConfig>("get_general_config"),
        ]);
        if (cancelled) return;
        setCliStatus(status);
        setCliPath(status.configuredPath ?? "");
        setSyncOfficial(config.pi_sync_official_auth_on_switch ?? true);
        setAutoRefresh(String(config.pi_auto_refresh_minutes ?? 10));
        loadedRef.current = true;
      } catch (error) {
        if (!cancelled) setCliError(String(error));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const patch = async (updates: PiGeneralConfig) => {
    try {
      await invoke("patch_general_config", { updates });
      window.dispatchEvent(new Event("config-updated"));
    } catch (error) {
      setCliError(String(error));
    }
  };

  const saveCliPath = async () => {
    setCliSaving(true);
    setCliError(null);
    try {
      const status = await piInstanceService.updatePiCliRuntimeConfig(cliPath);
      setCliStatus(status);
    } catch (error) {
      setCliError(String(error));
    } finally {
      setCliSaving(false);
    }
  };

  const commitAutoRefresh = () => {
    const parsed = Number.parseInt(autoRefresh, 10);
    const value = Number.isNaN(parsed) ? -1 : Math.min(999, Math.max(-1, parsed));
    // 0 is meaningless as an interval; treat it as disabled.
    const normalized = value === 0 ? -1 : value;
    setAutoRefresh(String(normalized));
    if (loadedRef.current) void patch({ pi_auto_refresh_minutes: normalized });
  };

  return (
    <div style={{ order }}>
      <div className="group-title">{t("quickSettings.pi.title", "pi CLI 设置")}</div>
      <div className="settings-group">
        <div className="settings-row">
          <div className="row-label">
            <div className="row-title">{t("quickSettings.pi.cliPath", "CLI 路径")}</div>
            <div className="row-desc">
              {cliStatus?.available
                ? t("quickSettings.pi.cliDetected", "已检测 {{version}} · {{path}}", {
                    version: cliStatus.version || "--",
                    path: cliStatus.binaryPath || "--",
                  })
                : t("quickSettings.pi.cliMissing", "未检测到 pi CLI，可填写自定义路径")}
            </div>
          </div>
          <div className="row-control">
            <input
              className="settings-input settings-input--path"
              value={cliPath}
              placeholder={cliStatus?.binaryPath || "pi"}
              aria-label={t("quickSettings.pi.cliPath", "CLI 路径")}
              onChange={(event) => {
                setCliPath(event.target.value);
                setCliError(null);
              }}
            />
            <button
              type="button"
              className="btn btn-secondary"
              onClick={() => void saveCliPath()}
              disabled={cliSaving}
            >
              <Save size={14} />
              {cliSaving ? t("common.loading", "加载中...") : t("common.save", "保存")}
            </button>
          </div>
        </div>
        {cliError && <div className="form-error">{cliError}</div>}

        <div className="settings-row">
          <div className="row-label">
            <div className="row-title">
              {t("quickSettings.pi.syncOfficialAuthOnSwitch", "切号同步官方登录")}
            </div>
            <div className="row-desc">
              {t(
                "quickSettings.pi.syncOfficialAuthOnSwitchDesc",
                "开启后，默认实例切换账号会写入官方 ~/.pi/agent/auth.json；关闭时使用独立 PI_CODING_AGENT_DIR。",
              )}
            </div>
          </div>
          <div className="row-control">
            <label className="switch">
              <input
                type="checkbox"
                checked={syncOfficial}
                onChange={(event) => {
                  setSyncOfficial(event.target.checked);
                  void patch({ pi_sync_official_auth_on_switch: event.target.checked });
                }}
              />
              <span className="slider"></span>
            </label>
          </div>
        </div>

        <div className="settings-row">
          <div className="row-label">
            <div className="row-title">{t("quickSettings.piRefreshInterval", "用量自动刷新")}</div>
            <div className="row-desc">
              {t("settings.general.windsurfAutoRefreshDesc", "后台自动更新频率")}
            </div>
          </div>
          <div className="row-control">
            <div className="settings-inline-input">
              <input
                type="number"
                min={-1}
                max={999}
                aria-label={t("quickSettings.piRefreshInterval", "用量自动刷新")}
                className="settings-select settings-select--input-mode settings-select--with-unit"
                value={autoRefresh}
                onChange={(event) => {
                  if (/^-?\d*$/.test(event.target.value)) setAutoRefresh(event.target.value);
                }}
                onBlur={commitAutoRefresh}
              />
              <span className="settings-input-unit">{t("settings.general.minutes")}</span>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
