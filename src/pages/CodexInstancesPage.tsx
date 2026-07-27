import { useEffect, useMemo, useState } from "react";
import { RefreshCw, Settings, X } from "lucide-react";
import { confirm as confirmDialog } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import { PlatformInstancesContent } from "../components/platform/PlatformInstancesContent";
import { CodexCliLaunchDialog } from "../components/codex/CodexCliLaunchDialog";
import { useLaunchTerminalOptions } from "../hooks/useLaunchTerminalOptions";
import { useCodexInstanceStore } from "../stores/useCodexInstanceStore";
import { useCodexAccountStore } from "../stores/useCodexAccountStore";
import { isCodexApiKeyAccount, type CodexAccount } from "../types/codex";
import {
  CODEX_API_SERVICE_BIND_ID,
  CODEX_PROVIDER_GATEWAY_BIND_PREFIX,
  type InstanceProfile,
} from "../types/instance";
import { usePlatformRuntimeSupport } from "../hooks/usePlatformRuntimeSupport";
import {
  buildCodexAccountPresentation,
  buildQuotaPreviewLines,
} from "../presentation/platformAccountPresentation";
import * as codexInstanceService from "../services/codexInstanceService";
import {
  CODEX_ADDITIONAL_QUOTA_VISIBILITY_CHANGED_EVENT,
  CODEX_CODE_REVIEW_QUOTA_VISIBILITY_CHANGED_EVENT,
  CODEX_PLAN_BADGE_STYLE_CHANGED_EVENT,
  getCodexPlanBadgeStyle,
  isCodexAdditionalQuotaVisibleByDefault,
  isCodexCodeReviewQuotaVisibleByDefault,
  type CodexPlanBadgeStyle,
} from "../utils/codexPreferences";
import {
  findCodexApiProviderPresetById,
  resolveCodexApiProviderPresetId,
} from "../utils/codexProviderPresets";
import { useEscClose } from "../hooks/useEscClose";

/**
 * Codex 应用多开内容组件（不包含 header）
 * 用于嵌入到 CodexAccountsPage 中
 */
interface CodexInstancesContentProps {
  accountsForSelect?: CodexAccount[];
}

interface CodexLaunchModalState {
  instanceId: string;
  instanceName: string;
  switchMessage: string;
  baseLaunchCommand: string;
  launchCommand: string;
  preparing: boolean;
  copied: boolean;
  executing: boolean;
  executeMessage: string | null;
  executeError: string | null;
}

const OPENAI_OFFICIAL_PRESET_ID = "openai_official";

function normalizeCodexApiBaseUrl(rawValue?: string | null): string {
  return (rawValue || "").trim().replace(/\/+$/, "");
}

export function CodexInstancesContent({
  accountsForSelect,
}: CodexInstancesContentProps = {}) {
  const { t } = useTranslation();
  const instanceStore = useCodexInstanceStore();
  const { accounts: storeAccounts, fetchAccounts } = useCodexAccountStore();
  const accounts = accountsForSelect ?? storeAccounts;
  const isMacOS = usePlatformRuntimeSupport("macos-only");
  const isWindows = usePlatformRuntimeSupport("windows-only");
  const isSupportedPlatform = isMacOS || isWindows;
  const [showCodeReviewQuota, setShowCodeReviewQuota] = useState<boolean>(
    isCodexCodeReviewQuotaVisibleByDefault,
  );
  const [showAdditionalQuota, setShowAdditionalQuota] = useState<boolean>(
    isCodexAdditionalQuotaVisibleByDefault,
  );
  const [planBadgeStyle, setPlanBadgeStyle] = useState<CodexPlanBadgeStyle>(
    getCodexPlanBadgeStyle,
  );
  const [launchModal, setLaunchModal] = useState<CodexLaunchModalState | null>(
    null,
  );
  const [syncingAllRecords, setSyncingAllRecords] = useState(false);
  const [autoSyncUpdating, setAutoSyncUpdating] = useState(false);
  const [showSyncSettingsModal, setShowSyncSettingsModal] = useState(false);
  const [syncRecordsMessage, setSyncRecordsMessage] = useState<{
    text: string;
    tone?: "error";
  } | null>(null);

  useEscClose(!!launchModal, () => setLaunchModal(null));
  useEscClose(showSyncSettingsModal, () => setShowSyncSettingsModal(false));
  const { terminalOptions, selectedTerminal, setSelectedTerminal } =
    useLaunchTerminalOptions(isSupportedPlatform);

  useEffect(() => {
    const syncCodeReviewVisibility = () => {
      setShowCodeReviewQuota(isCodexCodeReviewQuotaVisibleByDefault());
    };
    const syncAdditionalQuotaVisibility = () => {
      setShowAdditionalQuota(isCodexAdditionalQuotaVisibleByDefault());
    };

    window.addEventListener(
      CODEX_CODE_REVIEW_QUOTA_VISIBILITY_CHANGED_EVENT,
      syncCodeReviewVisibility as EventListener,
    );
    window.addEventListener(
      CODEX_ADDITIONAL_QUOTA_VISIBILITY_CHANGED_EVENT,
      syncAdditionalQuotaVisibility as EventListener,
    );
    const syncPlanBadgeStyle = () => {
      setPlanBadgeStyle(getCodexPlanBadgeStyle());
    };
    window.addEventListener(
      CODEX_PLAN_BADGE_STYLE_CHANGED_EVENT,
      syncPlanBadgeStyle as EventListener,
    );
    return () => {
      window.removeEventListener(
        CODEX_CODE_REVIEW_QUOTA_VISIBILITY_CHANGED_EVENT,
        syncCodeReviewVisibility as EventListener,
      );
      window.removeEventListener(
        CODEX_ADDITIONAL_QUOTA_VISIBILITY_CHANGED_EVENT,
        syncAdditionalQuotaVisibility as EventListener,
      );
      window.removeEventListener(
        CODEX_PLAN_BADGE_STYLE_CHANGED_EVENT,
        syncPlanBadgeStyle as EventListener,
      );
    };
  }, []);

  const resolvePresentation = (account: CodexAccount) => {
    // Read planBadgeStyle so style event rebuilds badge classes on instances.
    void planBadgeStyle;
    const presentation = buildCodexAccountPresentation(account, t);
    return {
      ...presentation,
      quotaItems: presentation.quotaItems.filter((item) => {
        if (!showCodeReviewQuota && item.key === "code_review") return false;
        if (!showAdditionalQuota && item.key.startsWith("additional:")) return false;
        return true;
      }),
    };
  };

  const accountsWithDisplayName = useMemo(
    () =>
      accounts.map((account) => {
        void planBadgeStyle;
        const displayName =
          buildCodexAccountPresentation(account, t).displayName ||
          account.email;
        return { ...account, email: displayName };
      }),
    [accounts, t, planBadgeStyle],
  );

  const resolveApiProviderDisplayName = (account: CodexAccount): string => {
    const baseUrl = normalizeCodexApiBaseUrl(account.api_base_url);
    const isOpenAiBuiltin =
      account.api_provider_mode === "openai_builtin" ||
      (!account.api_provider_mode &&
        (!baseUrl || baseUrl === "https://api.openai.com/v1"));
    if (isOpenAiBuiltin) {
      const preset = findCodexApiProviderPresetById(OPENAI_OFFICIAL_PRESET_ID);
      return preset
        ? t(`codex.api.providers.${preset.id}.name`, preset.name)
        : t("codex.api.provider.custom", "自定义");
    }

    const providerName = account.api_provider_name?.trim();
    if (providerName) return providerName;

    const preset = findCodexApiProviderPresetById(
      resolveCodexApiProviderPresetId(baseUrl),
    );
    if (preset) {
      return t(`codex.api.providers.${preset.id}.name`, preset.name);
    }
    return t("codex.api.provider.custom", "自定义");
  };

  const accountMap = useMemo(() => {
    const map = new Map<string, CodexAccount>();
    accounts.forEach((account) => map.set(account.id, account));
    return map;
  }, [accounts]);

  const defaultInstance = useMemo(
    () => instanceStore.instances.find((instance) => instance.isDefault) ?? null,
    [instanceStore.instances],
  );

  const renderCodexQuotaPreview = (account: CodexAccount) => {
    if (isCodexApiKeyAccount(account)) {
      const providerName = resolveApiProviderDisplayName(account);
      const text = t("codex.api.provider.inlineLabel", {
        provider: providerName,
        defaultValue: "供应商：{{provider}}",
      });
      return (
        <div className="account-quota-preview">
          <span className="account-quota-item account-provider-item">
            <span className="quota-dot" />
            <span className="quota-text account-provider-text" title={text}>
              {text}
            </span>
          </span>
        </div>
      );
    }

    const presentation = resolvePresentation(account);
    const lines = buildQuotaPreviewLines(presentation.quotaItems, 3);
    if (lines.length === 0) {
      return (
        <span className="account-quota-empty">
          {t("instances.quota.empty", "暂无配额缓存")}
        </span>
      );
    }
    return (
      <div className="account-quota-preview">
        {lines.map((line) => (
          <span className="account-quota-item" key={line.key}>
            <span className={`quota-dot ${line.quotaClass}`} />
            <span className={`quota-text ${line.quotaClass}`}>{line.text}</span>
          </span>
        ))}
      </div>
    );
  };

  const renderCodexPlanBadge = (account: CodexAccount) => {
    const presentation = resolvePresentation(account);
    return (
      <span className={`instance-plan-badge ${presentation.planClass}`}>
        {presentation.planLabel}
      </span>
    );
  };

  const handleInstanceStarted = async (instance: InstanceProfile) => {
    if ((instance.launchMode ?? "app") !== "cli") {
      return;
    }

    const launchInfo = await codexInstanceService.getCodexInstanceLaunchCommand(
      instance.id,
      selectedTerminal,
    );
    const boundAccountId = instance.bindAccountId?.startsWith(
      CODEX_PROVIDER_GATEWAY_BIND_PREFIX,
    )
      ? instance.bindAccountId.slice(CODEX_PROVIDER_GATEWAY_BIND_PREFIX.length)
      : instance.bindAccountId;
    const boundAccount = boundAccountId
      ? accountMap.get(boundAccountId)
      : undefined;
    const accountLabel =
      instance.bindAccountId === CODEX_API_SERVICE_BIND_ID
        ? t("codex.localAccess.title", "API 服务")
        : boundAccount
          ? buildCodexAccountPresentation(boundAccount, t).displayName ||
            boundAccount.email
          : null;
    const instanceName = instance.isDefault
      ? t("instances.defaultName", "默认实例")
      : instance.name || t("instances.defaultName", "默认实例");

    setLaunchModal({
      instanceId: instance.id,
      instanceName,
      switchMessage: accountLabel
        ? t("codex.switched", "已切换至 {{email}}", { email: accountLabel })
        : t("instances.messages.launchPrepared", "启动命令已准备"),
      baseLaunchCommand: launchInfo.launchCommand,
      launchCommand: launchInfo.terminalCommand,
      preparing: false,
      copied: false,
      executing: false,
      executeMessage: null,
      executeError: null,
    });
  };

  const handleCopyLaunchCommand = async () => {
    if (!launchModal || launchModal.preparing) return;
    try {
      await navigator.clipboard.writeText(
        launchModal.baseLaunchCommand || launchModal.launchCommand,
      );
      setLaunchModal((prev) => (prev ? { ...prev, copied: true } : prev));
      window.setTimeout(() => {
        setLaunchModal((prev) => (prev ? { ...prev, copied: false } : prev));
      }, 1200);
    } catch {
      setLaunchModal((prev) =>
        prev
          ? {
              ...prev,
              executeError: t(
                "common.shared.export.copyFailed",
                "复制失败，请手动复制",
              ),
            }
          : prev,
      );
    }
  };

  const handleExecuteInTerminal = async () => {
    if (!launchModal || launchModal.preparing || launchModal.executing) return;
    setLaunchModal((prev) =>
      prev
        ? { ...prev, executing: true, executeError: null, executeMessage: null }
        : prev,
    );
    try {
      const result =
        await codexInstanceService.executeCodexInstanceLaunchCommand(
          launchModal.instanceId,
          selectedTerminal,
        );
      setLaunchModal((prev) =>
        prev
          ? {
              ...prev,
              executing: false,
              executeMessage: result,
            }
          : prev,
      );
    } catch (error) {
      setLaunchModal((prev) =>
        prev
          ? {
              ...prev,
              executing: false,
              executeError: String(error),
            }
          : prev,
      );
    }
  };

  useEffect(() => {
    const instanceId = launchModal?.instanceId;
    if (!instanceId || launchModal.executing) return;
    let disposed = false;
    setLaunchModal((prev) =>
      prev
        ? {
            ...prev,
            preparing: true,
            copied: false,
            executeMessage: null,
            executeError: null,
          }
        : prev,
    );
    void codexInstanceService
      .getCodexInstanceLaunchCommand(instanceId, selectedTerminal)
      .then((launchInfo) => {
        if (disposed) return;
        setLaunchModal((prev) =>
          prev && prev.instanceId === instanceId
            ? {
                ...prev,
                baseLaunchCommand: launchInfo.launchCommand,
                launchCommand: launchInfo.terminalCommand,
                preparing: false,
              }
            : prev,
        );
      })
      .catch((error) => {
        if (disposed) return;
        setLaunchModal((prev) =>
          prev && prev.instanceId === instanceId
            ? {
                ...prev,
                preparing: false,
                executeError: String(error).replace(/^Error:\s*/, ""),
              }
            : prev,
        );
      });
    return () => {
      disposed = true;
    };
  }, [launchModal?.instanceId, selectedTerminal]);

  const handleSyncAllLocalRecords = async () => {
    if (syncingAllRecords) return;

    try {
      const latestInstances = await instanceStore.refreshInstances();
      if (latestInstances.length < 2) {
        setSyncRecordsMessage({
          text: t(
            "codex.instances.syncAllRecords.needTwo",
            "至少需要两个实例才能同步记录",
          ),
          tone: "error",
        });
        return;
      }

      const runningCount = latestInstances.filter(
        (instance) => instance.running,
      ).length;
      if (runningCount > 0) {
        setSyncRecordsMessage({
          text: t(
            "codex.instances.syncAllRecords.closeFirst",
            "请先关闭所有 Codex 实例后再同步记录，避免运行中的实例把旧记录写回。",
          ),
          tone: "error",
        });
        return;
      }

      const confirmed = await confirmDialog(
        t(
          "codex.instances.syncAllRecords.confirmMessage",
          "会把所有 Codex 实例中的本地会话记录做一次全量同步；同 ID 会话会进行事件级合并，写入前会备份目标实例关键文件和旧会话文件。确认继续？",
        ),
        {
          title: t(
            "codex.instances.syncAllRecords.title",
            "同步所有实例记录",
          ),
          okLabel: t("common.confirm", "确认"),
          cancelLabel: t("common.cancel", "取消"),
        },
      );
      if (!confirmed) return;

      setSyncingAllRecords(true);
      setSyncRecordsMessage(null);
      const summary = await instanceStore.syncThreadsAcrossInstances();
      setSyncRecordsMessage({ text: summary.message });
    } catch (error) {
      setSyncRecordsMessage({ text: String(error), tone: "error" });
    } finally {
      setSyncingAllRecords(false);
    }
  };

  const handleToggleAutoSyncAllRecords = async () => {
    if (!defaultInstance || autoSyncUpdating) return;

    const nextAutoSyncThreads = !Boolean(defaultInstance.autoSyncThreads);
    setAutoSyncUpdating(true);
    setSyncRecordsMessage(null);
    try {
      await instanceStore.updateInstance({
        instanceId: defaultInstance.id,
        autoSyncThreads: nextAutoSyncThreads,
      });
      setSyncRecordsMessage({
        text: nextAutoSyncThreads
          ? t(
              "codex.instances.syncAllRecords.autoEnabled",
              "已开启自动同步所有实例记录",
            )
          : t(
              "codex.instances.syncAllRecords.autoDisabled",
              "已关闭自动同步所有实例记录",
            ),
      });
    } catch (error) {
      setSyncRecordsMessage({ text: String(error), tone: "error" });
    } finally {
      setAutoSyncUpdating(false);
    }
  };

  const syncAllRecordsSettingsButton = (
    <div className="codex-sync-records-actions">
      <button
        type="button"
        className="btn btn-secondary icon-only"
        onClick={() => setShowSyncSettingsModal(true)}
        title={t(
          "codex.instances.syncAllRecords.settingsTitle",
          "实例记录设置",
        )}
        aria-label={t(
          "codex.instances.syncAllRecords.settingsTitle",
          "实例记录设置",
        )}
      >
        <Settings size={16} />
      </button>
    </div>
  );

  return (
    <>
      <div className="codex-instances-content">
        {syncRecordsMessage && (
          <div
            className={`action-message${syncRecordsMessage.tone ? ` ${syncRecordsMessage.tone}` : ""}`}
          >
            <span className="action-message-text">
              {syncRecordsMessage.text}
            </span>
            <button
              className="action-message-close"
              onClick={() => setSyncRecordsMessage(null)}
              aria-label={t("common.close", "关闭")}
            >
              <X size={14} />
            </button>
          </div>
        )}
        <PlatformInstancesContent
          instanceStore={instanceStore}
          accounts={accountsWithDisplayName}
          fetchAccounts={fetchAccounts}
          renderAccountQuotaPreview={renderCodexQuotaPreview}
          renderAccountBadge={renderCodexPlanBadge}
          getAccountSearchText={(account) => {
            const presentation = resolvePresentation(account);
            const providerText = isCodexApiKeyAccount(account)
              ? resolveApiProviderDisplayName(account)
              : "";
            return `${presentation.displayName} ${presentation.planLabel} ${providerText}`;
          }}
          appType="codex"
          isSupported={isSupportedPlatform}
          unsupportedTitleKey="common.shared.instances.unsupported.title"
          unsupportedTitleDefault="暂不支持当前系统"
          unsupportedDescKey="codex.instances.unsupported.desc"
          unsupportedDescDefault="Codex 应用多开仅支持 macOS 和 Windows。"
          onInstanceStarted={handleInstanceStarted}
          resolveStartSuccessMessage={(instance) =>
            (instance.launchMode ?? "app") === "cli"
              ? t("instances.messages.launchPrepared", "启动命令已准备")
              : t("instances.messages.started", "实例已启动")
          }
          toolbarExtraActions={syncAllRecordsSettingsButton}
        />
      </div>

      {showSyncSettingsModal && (
        <div
          className="codex-sync-settings-overlay"
        >
          <div
            className="codex-sync-settings-modal"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="codex-sync-settings-header">
              <div>
                <div className="codex-sync-settings-title">
                  {t(
                    "codex.instances.syncAllRecords.settingsTitle",
                    "实例记录设置",
                  )}
                </div>
                <div className="codex-sync-settings-subtitle">
                  {t(
                    "codex.instances.syncAllRecords.settingsSubtitle",
                    "管理 Codex 多实例记录同步方式",
                  )}
                </div>
              </div>
              <button
                className="codex-sync-settings-close"
                onClick={() => setShowSyncSettingsModal(false)}
                aria-label={t("common.close", "关闭")}
              >
                <X size={16} />
              </button>
            </div>
            <div className="codex-sync-settings-body">
              <div className="codex-sync-settings-section">
                <div className="codex-sync-settings-section-header">
                  <Settings size={15} />
                  <span>
                    {t(
                      "codex.instances.syncAllRecords.settingsSection",
                      "同步设置",
                    )}
                  </span>
                </div>
                <div className="codex-sync-settings-row">
                  <div className="codex-sync-settings-row-label">
                    <span>
                      {t(
                        "codex.instances.syncAllRecords.action",
                        "同步所有实例记录",
                      )}
                    </span>
                  </div>
                  <div className="codex-sync-settings-row-control">
                    <button
                      className="codex-sync-settings-action"
                      onClick={handleSyncAllLocalRecords}
                      disabled={syncingAllRecords}
                    >
                      <RefreshCw
                        size={14}
                        className={syncingAllRecords ? "icon-spin" : ""}
                      />
                      <span>
                        {syncingAllRecords
                          ? t("common.syncing", "同步中...")
                          : t(
                              "codex.instances.syncAllRecords.action",
                              "同步所有实例记录",
                            )}
                      </span>
                    </button>
                  </div>
                </div>
                <div className="codex-sync-settings-row">
                  <div className="codex-sync-settings-row-label">
                    <span>
                      {t(
                        "codex.instances.syncAllRecords.autoAction",
                        "自动同步所有实例记录",
                      )}
                    </span>
                  </div>
                  <div className="codex-sync-settings-row-control">
                    <label className="codex-sync-settings-switch">
                      <input
                        type="checkbox"
                        checked={Boolean(defaultInstance?.autoSyncThreads)}
                        disabled={!defaultInstance || autoSyncUpdating}
                        onChange={handleToggleAutoSyncAllRecords}
                      />
                      <span className="codex-sync-settings-switch-slider" />
                    </label>
                  </div>
                </div>
                <div className="codex-sync-settings-hint">
                  {t(
                    "codex.instances.syncAllRecords.autoDesc",
                    "关闭时保持多实例记录隔离；开启后仅在所有 Codex 实例已停止时，启动或关闭实例会自动合并本地记录。",
                  )}
                </div>
              </div>
            </div>
          </div>
        </div>
      )}

      {launchModal && (
        <CodexCliLaunchDialog
          subjectLabel={t("instances.columns.instance", "实例")}
          subjectValue={launchModal.instanceName}
          statusMessage={launchModal.switchMessage}
          terminal={selectedTerminal}
          terminalOptions={terminalOptions}
          onTerminalChange={setSelectedTerminal}
          command={launchModal.launchCommand}
          preparing={launchModal.preparing}
          copied={launchModal.copied}
          executing={launchModal.executing}
          successMessage={launchModal.executeMessage}
          errorMessage={launchModal.executeError}
          onBack={() => setLaunchModal(null)}
          onClose={() => setLaunchModal(null)}
          onCopy={() => void handleCopyLaunchCommand()}
          onExecute={() => void handleExecuteInTerminal()}
        />
      )}
    </>
  );
}
