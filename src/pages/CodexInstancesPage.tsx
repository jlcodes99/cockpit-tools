import { useEffect, useMemo, useState } from "react";
import { Check, ChevronLeft, Copy, Play, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { PlatformInstancesContent } from "../components/platform/PlatformInstancesContent";
import { SingleSelectDropdown } from "../components/SingleSelectDropdown";
import { useLaunchTerminalOptions } from "../hooks/useLaunchTerminalOptions";
import { useCodexInstanceStore } from "../stores/useCodexInstanceStore";
import { useCodexAccountStore } from "../stores/useCodexAccountStore";
import { isCodexApiKeyAccount, type CodexAccount } from "../types/codex";
import {
  CODEX_API_SERVICE_BIND_ID,
  type InstanceProfile,
} from "../types/instance";
import { usePlatformRuntimeSupport } from "../hooks/usePlatformRuntimeSupport";
import {
  buildCodexAccountPresentation,
  buildQuotaPreviewLines,
} from "../presentation/platformAccountPresentation";
import * as codexInstanceService from "../services/codexInstanceService";
import {
  CODEX_CODE_REVIEW_QUOTA_VISIBILITY_CHANGED_EVENT,
  isCodexCodeReviewQuotaVisibleByDefault,
} from "../utils/codexPreferences";
import {
  findCodexApiProviderPresetById,
  resolveCodexApiProviderPresetId,
} from "../utils/codexProviderPresets";
import { useEscClose } from "../hooks/useEscClose";

/**
 * Codex 多开实例内容组件（不包含 header）
 * 用于嵌入到 CodexAccountsPage 中
 */
interface CodexInstancesContentProps {
  accountsForSelect?: CodexAccount[];
}

interface CodexLaunchModalState {
  instanceId: string;
  instanceName: string;
  switchMessage: string;
  launchCommand: string;
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
  const [launchModal, setLaunchModal] = useState<CodexLaunchModalState | null>(
    null,
  );

  useEscClose(!!launchModal, () => setLaunchModal(null));
  const { terminalOptions, selectedTerminal, setSelectedTerminal } =
    useLaunchTerminalOptions(isSupportedPlatform);

  useEffect(() => {
    const syncCodeReviewVisibility = () => {
      setShowCodeReviewQuota(isCodexCodeReviewQuotaVisibleByDefault());
    };

    window.addEventListener(
      CODEX_CODE_REVIEW_QUOTA_VISIBILITY_CHANGED_EVENT,
      syncCodeReviewVisibility as EventListener,
    );
    return () => {
      window.removeEventListener(
        CODEX_CODE_REVIEW_QUOTA_VISIBILITY_CHANGED_EVENT,
        syncCodeReviewVisibility as EventListener,
      );
    };
  }, []);

  const resolvePresentation = (account: CodexAccount) => {
    const presentation = buildCodexAccountPresentation(account, t);
    if (showCodeReviewQuota) {
      return presentation;
    }
    return {
      ...presentation,
      quotaItems: presentation.quotaItems.filter(
        (item) => item.key !== "code_review",
      ),
    };
  };

  const accountsWithDisplayName = useMemo(
    () =>
      accounts.map((account) => {
        const displayName =
          buildCodexAccountPresentation(account, t).displayName ||
          account.email;
        return { ...account, email: displayName };
      }),
    [accounts, t],
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
    );
    const boundAccount = instance.bindAccountId
      ? accountMap.get(instance.bindAccountId)
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
      launchCommand: launchInfo.launchCommand,
      copied: false,
      executing: false,
      executeMessage: null,
      executeError: null,
    });
  };

  const handleCopyLaunchCommand = async () => {
    if (!launchModal) return;
    try {
      await navigator.clipboard.writeText(launchModal.launchCommand);
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
    if (!launchModal || launchModal.executing) return;
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

  return (
    <>
      <div className="codex-instances-content">
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
          unsupportedDescDefault="Codex 多开实例仅支持 macOS 和 Windows。"
          onInstanceStarted={handleInstanceStarted}
          resolveStartSuccessMessage={(instance) =>
            (instance.launchMode ?? "app") === "cli"
              ? t("instances.messages.launchPrepared", "启动命令已准备")
              : t("instances.messages.started", "实例已启动")
          }
        />
      </div>

      {launchModal && (
        <div className="modal-overlay" onClick={() => setLaunchModal(null)}>
          <div
            className="modal modal-lg"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="modal-header">
              <button className="btn btn-secondary icon-only" onClick={() => setLaunchModal(null)} title={t("common.back", "返回")} aria-label={t("common.back", "返回")}><ChevronLeft size={14} /></button>
              <h2>{t("instances.launchDialog.title", "启动实例")}</h2>
              <button
                className="modal-close"
                onClick={() => setLaunchModal(null)}
                aria-label={t("common.close", "关闭")}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              <div className="add-status success">
                <Check size={16} />
                <span>{launchModal.switchMessage}</span>
              </div>
              <div className="form-group">
                <label>{t("instances.columns.instance", "实例")}</label>
                <input
                  className="form-input"
                  value={launchModal.instanceName}
                  readOnly
                />
              </div>
              <div className="form-group">
                <label>{t("instances.launchDialog.command", "启动命令")}</label>
                <textarea
                  className="form-input instance-args-input"
                  value={launchModal.launchCommand}
                  readOnly
                />
                <p className="form-hint">
                  {t(
                    "instances.launchDialog.hint",
                    "可复制命令手动执行，或点击下方按钮直接在终端执行。",
                  )}
                </p>
              </div>
              <div className="form-group">
                <label>{t("instances.launchDialog.terminal", "终端")}</label>
                <SingleSelectDropdown
                  value={selectedTerminal}
                  onChange={setSelectedTerminal}
                  options={terminalOptions}
                  disabled={launchModal.executing}
                  ariaLabel={t("instances.launchDialog.terminal", "终端")}
                />
              </div>
              {launchModal.executeMessage && (
                <div className="add-status success">
                  <Check size={16} />
                  <span>{launchModal.executeMessage}</span>
                </div>
              )}
              {launchModal.executeError && (
                <div className="form-error">{launchModal.executeError}</div>
              )}
            </div>
            <div className="modal-footer">
              <button
                className="btn btn-secondary"
                onClick={handleCopyLaunchCommand}
              >
                <Copy size={16} />
                {launchModal.copied
                  ? t("common.success", "成功")
                  : t("common.copy", "复制")}
              </button>
              <button
                className="btn btn-primary"
                onClick={handleExecuteInTerminal}
                disabled={launchModal.executing}
              >
                <Play size={16} />
                {launchModal.executing
                  ? t("common.loading", "加载中...")
                  : t("instances.launchDialog.runInTerminal", "终端执行")}
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
