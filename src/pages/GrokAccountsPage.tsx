import { useCallback, useEffect, useMemo, useState } from "react";
import {
  CalendarDays,
  Check,
  ChevronLeft,
  CircleAlert,
  Copy,
  FolderOpen,
  KeyRound,
  Play,
  X,
} from "lucide-react";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import { ModalErrorMessage } from "../components/ModalErrorMessage";
import { SingleSelectDropdown } from "../components/SingleSelectDropdown";
import {
  PlatformOverviewTabsHeader,
  type PlatformOverviewTab,
} from "../components/platform/PlatformOverviewTabsHeader";
import {
  CodebuddySuiteAccountsSharedView,
  type CodebuddySuiteAccountsPlatformConfig,
} from "../components/codebuddy-suite/CodebuddySuiteAccountsSharedView";
import { useProviderAccountsPage } from "../hooks/useProviderAccountsPage";
import { useEscClose } from "../hooks/useEscClose";
import { useLaunchTerminalOptions } from "../hooks/useLaunchTerminalOptions";
import * as grokInstanceService from "../services/grokInstanceService";
import * as grokService from "../services/grokService";
import { useGrokAccountStore } from "../stores/useGrokAccountStore";
import {
  formatGrokQuotaResetTime,
  formatGrokQuotaUsedTotal,
  getGrokAccountDisplayEmail,
  getGrokPlanBadge,
  getGrokPlanRawValue,
  getGrokQuotaClass,
  getGrokQuotaGroups,
  getGrokQuotaSummaryItems,
  getGrokUsage,
  hasGrokQuotaData,
  isGrokApiKeyAccount,
  type GrokAccount,
} from "../types/grok";
import { GrokInstancesContent } from "./GrokInstancesPage";

const FLOW_NOTICE_KEY = "agtools.grok.flow_notice_collapsed";
const CURRENT_ACCOUNT_KEY = "agtools.grok.current_account_id";
const GROK_CLI_INSTALL_COMMAND_UNIX =
  "curl -fsSL https://x.ai/cli/install.sh | bash";
const GROK_CLI_INSTALL_COMMAND_WINDOWS =
  "irm https://x.ai/cli/install.ps1 | iex";

function getGrokCliInstallCommand(): string {
  if (typeof navigator === "undefined") {
    return GROK_CLI_INSTALL_COMMAND_UNIX;
  }
  const platform = `${navigator.platform || ""} ${navigator.userAgent || ""}`;
  return /win/i.test(platform)
    ? GROK_CLI_INSTALL_COMMAND_WINDOWS
    : GROK_CLI_INSTALL_COMMAND_UNIX;
}

function isValidGrokApiBaseUrl(value: string): boolean {
  try {
    const parsed = new URL(value.trim());
    return (
      (parsed.protocol === "http:" || parsed.protocol === "https:") &&
      !!parsed.hostname &&
      !parsed.username &&
      !parsed.password &&
      !parsed.search &&
      !parsed.hash
    );
  } catch {
    return false;
  }
}

function getGrokReauthorizationReason(account: GrokAccount): string | null {
  if (isGrokApiKeyAccount(account)) return null;
  const reason = account.status_reason?.trim() || "";
  const normalized = `${account.status || ""} ${reason}`.toLowerCase();
  if (
    account.status === "reauth_required" ||
    normalized.includes("invalid_grant") ||
    normalized.includes("refresh token has been revoked") ||
    normalized.includes("refresh_token 为空") ||
    normalized.includes("access_denied")
  ) {
    return reason || "invalid_grant";
  }
  return null;
}

interface GrokAccountLaunchModalState {
  instanceId: string;
  accountId: string;
  accountEmail: string;
  workingDir: string;
  launchCommand: string;
  regeneratingCommand: boolean;
  copied: boolean;
  executing: boolean;
  executeMessage: string | null;
  executeError: string | null;
  errorScrollKey: number;
}

type GrokApiProviderMode = "official" | "custom";

export function GrokAccountsPage() {
  const { t } = useTranslation();
  const [activeTab, setActiveTab] = useState<PlatformOverviewTab>("overview");
  const [launchModal, setLaunchModal] =
    useState<GrokAccountLaunchModalState | null>(null);
  const { terminalOptions, selectedTerminal, setSelectedTerminal } =
    useLaunchTerminalOptions();
  const grokCliInstallCommand = useMemo(() => getGrokCliInstallCommand(), []);
  const [installCommandCopied, setInstallCommandCopied] = useState(false);
  const [installExecuting, setInstallExecuting] = useState(false);
  const [installOpened, setInstallOpened] = useState(false);
  const [installError, setInstallError] = useState<string | null>(null);
  const [installErrorScrollKey, setInstallErrorScrollKey] = useState(0);
  const [apiProviderMode, setApiProviderMode] =
    useState<GrokApiProviderMode>("official");
  const [apiBaseUrl, setApiBaseUrl] = useState("");
  const [apiModel, setApiModel] = useState("");
  const [apiBaseUrlTouched, setApiBaseUrlTouched] = useState(false);
  const [apiModelTouched, setApiModelTouched] = useState(false);
  const store = useGrokAccountStore();
  const [reauthTargetAccount, setReauthTargetAccount] =
    useState<GrokAccount | null>(null);

  useEscClose(!!launchModal, () => setLaunchModal(null));

  const page = useProviderAccountsPage<GrokAccount>({
    platformKey: "grok",
    oauthLogPrefix: "GrokOAuth",
    flowNoticeCollapsedKey: FLOW_NOTICE_KEY,
    currentAccountIdKey: CURRENT_ACCOUNT_KEY,
    exportFilePrefix: "grok_accounts",
    oauthTabKeys: ["oauth"],
    store: {
      accounts: store.accounts,
      currentAccountId: store.currentAccountId,
      loading: store.loading,
      error: store.error,
      fetchAccounts: store.fetchAccounts,
      fetchCurrentAccountId: store.fetchCurrentAccountId,
      deleteAccounts: store.deleteAccounts,
      refreshToken: store.refreshToken,
      refreshAllTokens: store.refreshAllTokens,
      setCurrentAccountId: store.setCurrentAccountId,
      updateAccountTags: store.updateAccountTags,
    },
    oauthService: {
      startLogin: grokService.startGrokOAuthLogin,
      completeLogin: (loginId) =>
        grokService.completeGrokOAuthLogin(
          loginId,
          reauthTargetAccount?.id ?? null,
        ),
      cancelLogin: grokService.cancelGrokOAuthLogin,
    },
    dataService: {
      importFromJson: grokService.importGrokFromJson,
      importFromLocal: grokService.importGrokFromLocal,
      exportAccounts: grokService.exportGrokAccounts,
      injectToVSCode: grokService.switchGrokAccount,
      addWithToken: (apiKey) =>
        grokService.addGrokAccountWithApiKey(
          apiKey,
          apiProviderMode === "custom" ? { apiBaseUrl, apiModel } : undefined,
        ),
    },
    getDisplayEmail: getGrokAccountDisplayEmail,
    onInjectSuccess: async ({ accountId, account, displayEmail }) => {
      const accountEmail = account
        ? getGrokAccountDisplayEmail(account)
        : displayEmail || accountId;
      const workingDir = account?.working_dir?.trim() || "";
      try {
        const launchInfo =
          await grokInstanceService.getGrokInstanceLaunchCommand("__default__", {
            workingDir,
            applyWorkingDirOverride: true,
            accountId,
          });
        setLaunchModal({
          instanceId: launchInfo.instanceId || "__default__",
          accountId,
          accountEmail,
          workingDir,
          launchCommand: launchInfo.launchCommand,
          regeneratingCommand: false,
          copied: false,
          executing: false,
          executeMessage: null,
          // 账号授权类问题只体现在账号列表，启动弹框不展示
          executeError: null,
          errorScrollKey: 0,
        });
        // 后端可能已把 reauth 状态写回账号，刷新列表徽章
        void store.fetchAccounts();
      } catch (error) {
        const message = String(error);
        setLaunchModal({
          instanceId: "__default__",
          accountId,
          accountEmail,
          workingDir,
          launchCommand: "",
          regeneratingCommand: false,
          copied: false,
          executing: false,
          executeMessage: null,
          executeError: grokInstanceService.isGrokReauthError(message)
            ? null
            : message,
          errorScrollKey: grokInstanceService.isGrokReauthError(message) ? 0 : 1,
        });
        if (grokInstanceService.isGrokReauthError(message)) {
          void store.fetchAccounts();
        }
      }
    },
    resolveOauthSuccessMessage: () =>
      t("grok.oauth.success", "Grok OAuth 登录成功"),
  });

  useEffect(() => {
    if (!page.showAddModal) {
      setReauthTargetAccount(null);
      setApiProviderMode("official");
      setApiBaseUrl("");
      setApiModel("");
      setApiBaseUrlTouched(false);
      setApiModelTouched(false);
    }
  }, [page.showAddModal]);

  const handleReauthorize = useCallback(
    (account: GrokAccount) => {
      setReauthTargetAccount(account);
      page.openAddModal("oauth");
    },
    [page.openAddModal],
  );

  const persistAccountWorkingDir = useCallback(
    async (accountId: string, workingDir: string) => {
      await grokService.updateGrokAccountWorkingDir(
        accountId,
        workingDir.trim() || null,
      );
      await store.fetchAccounts();
    },
    [store],
  );

  const regenerateLaunchCommand = useCallback(
    async (modal: GrokAccountLaunchModalState, workingDir: string) => {
      setLaunchModal((current) =>
        current && current.accountId === modal.accountId
          ? {
              ...current,
              workingDir,
              regeneratingCommand: true,
              executeError: null,
              executeMessage: null,
            }
          : current,
      );
      try {
        const launchInfo =
          await grokInstanceService.getGrokInstanceLaunchCommand(
            modal.instanceId,
            {
              workingDir,
              applyWorkingDirOverride: true,
              accountId: modal.accountId,
            },
          );
        setLaunchModal((current) =>
          current && current.accountId === modal.accountId
            ? {
                ...current,
                workingDir,
                launchCommand: launchInfo.launchCommand,
                regeneratingCommand: false,
                executeError: null,
              }
            : current,
        );
        void store.fetchAccounts();
      } catch (error) {
        const message = String(error);
        setLaunchModal((current) =>
          current && current.accountId === modal.accountId
            ? {
                ...current,
                workingDir,
                regeneratingCommand: false,
                // 账号授权错误不进启动弹框
                executeError: grokInstanceService.isGrokReauthError(message)
                  ? null
                  : message,
                errorScrollKey: grokInstanceService.isGrokReauthError(message)
                  ? current.errorScrollKey
                  : current.errorScrollKey + 1,
              }
            : current,
        );
        if (grokInstanceService.isGrokReauthError(message)) {
          void store.fetchAccounts();
        }
      }
    },
    [store],
  );

  const updateLaunchWorkingDir = (value: string) => {
    setLaunchModal((current) =>
      current
        ? {
            ...current,
            workingDir: value,
            launchCommand: "",
            executeError: null,
            executeMessage: null,
          }
        : current,
    );
  };

  const handleChooseLaunchWorkingDir = async () => {
    if (!launchModal || launchModal.executing || launchModal.regeneratingCommand)
      return;
    const selected = await openFileDialog({
      directory: true,
      multiple: false,
      title: t("grok.instances.selectWorkingDir", "选择 Grok CLI 工作目录"),
    });
    if (!selected || typeof selected !== "string") return;
    try {
      await persistAccountWorkingDir(launchModal.accountId, selected);
      await regenerateLaunchCommand(launchModal, selected);
    } catch (error) {
      setLaunchModal((current) =>
        current
          ? {
              ...current,
              executeError: String(error),
              errorScrollKey: current.errorScrollKey + 1,
            }
          : current,
      );
    }
  };

  const handleLaunchWorkingDirBlur = async () => {
    if (!launchModal || launchModal.executing || launchModal.regeneratingCommand)
      return;
    const nextWorkingDir = launchModal.workingDir.trim();
    if (launchModal.launchCommand.trim()) {
      // Command already matches current input unless path changed since last gen.
      // Regenerate when input differs from the last successful bound value.
      const bound =
        store.accounts
          .find((item) => item.id === launchModal.accountId)
          ?.working_dir?.trim() || "";
      if (nextWorkingDir === bound) return;
    }
    try {
      // Preview command only; bind to account on copy/execute or folder pick.
      await regenerateLaunchCommand(launchModal, nextWorkingDir);
    } catch (error) {
      setLaunchModal((current) =>
        current
          ? {
              ...current,
              executeError: String(error),
              errorScrollKey: current.errorScrollKey + 1,
            }
          : current,
      );
    }
  };

  const ensureLaunchCommandReady = async (
    modal: GrokAccountLaunchModalState,
  ): Promise<GrokAccountLaunchModalState | null> => {
    if (modal.launchCommand.trim() && !modal.regeneratingCommand) {
      return modal;
    }
    try {
      await persistAccountWorkingDir(modal.accountId, modal.workingDir);
      const launchInfo = await grokInstanceService.getGrokInstanceLaunchCommand(
        modal.instanceId,
        {
          workingDir: modal.workingDir,
          applyWorkingDirOverride: true,
          accountId: modal.accountId,
        },
      );
      const next: GrokAccountLaunchModalState = {
        ...modal,
        launchCommand: launchInfo.launchCommand,
        regeneratingCommand: false,
        executeError: null,
      };
      setLaunchModal((current) =>
        current && current.accountId === modal.accountId ? next : current,
      );
      return next;
    } catch (error) {
      setLaunchModal((current) =>
        current && current.accountId === modal.accountId
          ? {
              ...current,
              regeneratingCommand: false,
              executeError: String(error),
              errorScrollKey: current.errorScrollKey + 1,
            }
          : current,
      );
      return null;
    }
  };

  const handleCopyLaunchCommand = async () => {
    if (!launchModal) return;
    const prepared = await ensureLaunchCommandReady(launchModal);
    if (!prepared?.launchCommand) return;
    try {
      await navigator.clipboard.writeText(prepared.launchCommand);
      setLaunchModal((current) =>
        current ? { ...current, copied: true, executeError: null } : current,
      );
      window.setTimeout(() => {
        setLaunchModal((current) =>
          current ? { ...current, copied: false } : current,
        );
      }, 1200);
    } catch {
      setLaunchModal((current) =>
        current
          ? {
              ...current,
              executeError: t(
                "common.shared.export.copyFailed",
                "复制失败，请手动复制",
              ),
              errorScrollKey: current.errorScrollKey + 1,
            }
          : current,
      );
    }
  };

  const handleExecuteInTerminal = async () => {
    if (!launchModal || launchModal.executing || launchModal.regeneratingCommand)
      return;
    setLaunchModal((current) =>
      current
        ? {
            ...current,
            executing: true,
            executeError: null,
            executeMessage: null,
          }
        : current,
    );
    try {
      const prepared = await ensureLaunchCommandReady(launchModal);
      if (!prepared) {
        setLaunchModal((current) =>
          current ? { ...current, executing: false } : current,
        );
        return;
      }
      const result = await grokInstanceService.executeGrokInstanceLaunchCommand(
        prepared.instanceId,
        selectedTerminal,
        {
          workingDir: prepared.workingDir,
          applyWorkingDirOverride: true,
          accountId: prepared.accountId,
        },
      );
      setLaunchModal((current) =>
        current
          ? {
              ...current,
              executing: false,
              launchCommand: prepared.launchCommand,
              executeMessage: result,
            }
          : current,
      );
    } catch (error) {
      setLaunchModal((current) =>
        current
          ? {
              ...current,
              executing: false,
              executeError: String(error),
              errorScrollKey: current.errorScrollKey + 1,
            }
          : current,
      );
    }
  };

  const handleTerminalChange = (terminal: string) => {
    setSelectedTerminal(terminal);
    setLaunchModal((current) =>
      current
        ? { ...current, executeError: null, executeMessage: null }
        : current,
    );
  };

  const renderGrokQuotaSection = useCallback(
    (account: GrokAccount, variant: "card" | "table") => {
      const items = getGrokQuotaSummaryItems(
        account,
        t as (key: string, defaultValue?: string) => string,
      );
      const reauthorizationReason = getGrokReauthorizationReason(account);
      const errorMessage = account.quota_query_last_error?.trim() || "";
      const showError = !!errorMessage && items.length === 0;

      return (
        <div className={`grok-quota-summary ${variant}`}>
          {reauthorizationReason && (
            <div
              className={`quota-error-inline ${variant === "table" ? "table" : ""}`}
            >
              <CircleAlert size={14} />
              <span title={reauthorizationReason}>{reauthorizationReason}</span>
              <button
                type="button"
                className="btn btn-sm btn-outline quota-error-action"
                onClick={() => handleReauthorize(account)}
              >
                <KeyRound size={12} />
                {t("common.reauthorize", "重新授权")}
              </button>
            </div>
          )}
          <div className="grok-quota-items">
            {isGrokApiKeyAccount(account) && items.length === 0 && !errorMessage && (
              <div
                className={variant === "card" ? "quota-empty" : ""}
                style={
                  variant === "table"
                    ? { color: "var(--text-muted)", fontSize: 13 }
                    : undefined
                }
              >
                {t(
                  "grok.quota.apiKeyUnsupported",
                  "API Key 账号无套餐配额",
                )}
              </div>
            )}
            {items.map((item) => {
              // item.percentage 为 used%；文案与进度条均展示剩余%（与 Gemini 等平台一致：越用越短）
              const usedPercent = Math.max(
                0,
                Math.min(100, Math.round(item.percentage)),
              );
              const remainingPercent = Math.max(0, Math.min(100, 100 - usedPercent));
              // 颜色按已用比例（越用越红），条长度按剩余
              const quotaClass = getGrokQuotaClass(usedPercent);
              const amountText = formatGrokQuotaUsedTotal(item.used, item.total);
              const remainingLabel = t(
                "common.shared.quota.leftPercent",
                "{{value}}% left",
                { value: remainingPercent },
              );
              const resetText = formatGrokQuotaResetTime(item.resetAtMs);
              const resetDisplay = resetText || "-";
              const titleParts = [
                item.label,
                amountText || null,
                remainingLabel,
                resetText
                  ? t("grok.quota.resetAt", "{{label}} 重置：{{time}}", {
                      label: item.label,
                      time: resetText,
                    })
                  : null,
              ].filter(Boolean);
              const title = titleParts.join(" · ");

              if (variant === "card") {
                return (
                  <div
                    className="quota-item"
                    key={`${account.id}-${item.key}`}
                    title={title}
                  >
                    <div className="quota-header">
                      <CalendarDays size={14} />
                      <span className="quota-label">{item.label}</span>
                      <span className={`quota-pct ${quotaClass}`}>
                        {amountText ? (
                          <>
                            <span className="grok-quota-amount">{amountText}</span>
                            <span className="grok-quota-pct-sep">·</span>
                          </>
                        ) : null}
                        {remainingLabel}
                      </span>
                    </div>
                    <div className="quota-bar-track">
                      <div
                        className={`quota-bar ${quotaClass}`}
                        style={{ width: `${remainingPercent}%` }}
                      />
                    </div>
                    <span className="quota-reset">{resetDisplay}</span>
                  </div>
                );
              }

              return (
                <div
                  className="quota-item"
                  key={`${account.id}-${item.key}`}
                  title={title}
                >
                  <div className="quota-header">
                    <span className="quota-name">{item.label}</span>
                    <span className={`quota-value ${quotaClass}`}>
                      {amountText ? (
                        <>
                          <span className="grok-quota-amount">{amountText}</span>
                          <span className="grok-quota-pct-sep">·</span>
                        </>
                      ) : null}
                      {remainingLabel}
                    </span>
                  </div>
                  <div className="quota-progress-track">
                    <div
                      className={`quota-progress-bar ${quotaClass}`}
                      style={{ width: `${remainingPercent}%` }}
                    />
                  </div>
                  <div className="quota-footer">
                    <span className="quota-reset">{resetDisplay}</span>
                  </div>
                </div>
              );
            })}
            {items.length === 0 &&
              !showError &&
              !isGrokApiKeyAccount(account) && (
              <div
                className={variant === "card" ? "quota-empty" : ""}
                style={
                  variant === "table"
                    ? { color: "var(--text-muted)", fontSize: 13 }
                    : undefined
                }
              >
                {t("grok.quota.empty", "暂无额度")}
              </div>
            )}
            {errorMessage && (
              <div
                className={`quota-error-inline ${variant === "table" ? "table" : ""}`}
                title={errorMessage}
              >
                <CircleAlert size={variant === "table" ? 12 : 14} />
                <span>{errorMessage}</span>
              </div>
            )}
          </div>
        </div>
      );
    },
    [handleReauthorize, t],
  );

  const handleInstallTerminalChange = (terminal: string) => {
    setSelectedTerminal(terminal);
    setInstallError(null);
    setInstallOpened(false);
  };

  const reportInstallError = (message: string) => {
    setInstallError(message);
    setInstallErrorScrollKey((current) => current + 1);
  };

  const handleCopyInstallCommand = async () => {
    setInstallError(null);
    setInstallCommandCopied(false);
    try {
      await navigator.clipboard.writeText(grokCliInstallCommand);
      setInstallCommandCopied(true);
      window.setTimeout(() => setInstallCommandCopied(false), 1200);
    } catch {
      reportInstallError(
        t("common.shared.export.copyFailed", "复制失败，请手动复制"),
      );
    }
  };

  const handleExecuteInstallCommand = async () => {
    if (installExecuting) return;
    setInstallError(null);
    setInstallOpened(false);
    setInstallExecuting(true);
    try {
      await grokInstanceService.executeGrokCliInstallCommand(selectedTerminal);
      setInstallOpened(true);
    } catch (error) {
      reportInstallError(String(error));
    } finally {
      setInstallExecuting(false);
    }
  };

  const renderGrokCliInstallGuide = () => (
    <div className="grok-cli-install-guide">
      <strong>{t("grok.instances.installCommand", "官方安装命令")}</strong>
      <p>
        {t(
          "grok.instances.installLaunchHint",
          "可在终端运行以下官方命令，安装完成后重新点击终端执行。",
        )}
      </p>
      <div className="grok-cli-install-command">
        <code>{grokCliInstallCommand}</code>
        <button
          type="button"
          className="btn btn-secondary icon-only"
          onClick={() => void handleCopyInstallCommand()}
          title={
            installCommandCopied
              ? t("common.success", "成功")
              : t("common.copy", "复制")
          }
          aria-label={t("common.copy", "复制")}
        >
          {installCommandCopied ? <Check size={14} /> : <Copy size={14} />}
        </button>
      </div>
      <div className="grok-cli-install-actions">
        <div className="grok-cli-install-terminal">
          <label>{t("instances.launchDialog.terminal", "终端")}</label>
          <SingleSelectDropdown
            value={selectedTerminal}
            onChange={handleInstallTerminalChange}
            options={terminalOptions}
            disabled={installExecuting || !!launchModal?.executing}
          />
        </div>
        <button
          type="button"
          className="btn btn-primary"
          onClick={() => void handleExecuteInstallCommand()}
          disabled={installExecuting || !!launchModal?.executing}
        >
          <Play size={14} />
          {installExecuting
            ? t("common.loading", "加载中...")
            : t("grok.instances.runInTerminal", "终端执行")}
        </button>
      </div>
      {installOpened && (
        <div className="add-status success">
          <Check size={14} />
          <span>{t("common.success", "成功")}</span>
        </div>
      )}
      <ModalErrorMessage
        message={installError}
        scrollKey={installErrorScrollKey}
      />
    </div>
  );

  const accountsForInstances = useMemo(
    () =>
      [...store.accounts].sort((left, right) => {
        const createdDiff = right.created_at - left.created_at;
        return page.sortDirection === "desc" ? createdDiff : -createdDiff;
      }),
    [page.sortDirection, store.accounts],
  );

  const customApiBaseUrlValid = isValidGrokApiBaseUrl(apiBaseUrl);
  const customApiFieldsIncomplete =
    apiProviderMode === "custom" &&
    (!apiBaseUrl.trim() || !apiModel.trim() || !customApiBaseUrlValid);
  const apiBaseUrlError =
    apiProviderMode === "custom" && apiBaseUrlTouched
      ? !apiBaseUrl.trim()
        ? t("grok.import.apiFieldRequired", "请填写{{field}}", {
            field: t("grok.import.apiBaseUrl", "接口地址"),
          })
        : !customApiBaseUrlValid
          ? t(
              "grok.import.apiBaseUrlInvalid",
              "请输入完整的 http:// 或 https:// 地址，且不要包含用户名、查询参数或片段",
            )
          : null
      : null;
  const apiModelError =
    apiProviderMode === "custom" && apiModelTouched && !apiModel.trim()
      ? t("grok.import.apiFieldRequired", "请填写{{field}}", {
          field: t("grok.import.apiModel", "模型 ID"),
        })
      : null;

  const platformConfig: CodebuddySuiteAccountsPlatformConfig<GrokAccount> = {
    pageClassName: "grok-accounts-page",
    quickSettingsType: "grok",
    searchPlaceholderKey: "grok.search",
    searchPlaceholderDefault: "搜索 Grok CLI 账号...",
    flowNotice: {
      titleKey: "grok.flowNotice.title",
      titleDefault: "Grok CLI 账号管理说明",
      descKey: "grok.flowNotice.desc",
      descDefault:
        "默认使用独立 GROK_HOME；开启“切号同步官方登录”后，默认实例切换 OAuth 账号会写入官方 ~/.grok/auth.json，多开实例仍保持隔离。",
      permissionKey: "grok.flowNotice.permission",
      permissionDefault:
        "本地范围：可读取默认 ~/.grok/auth.json 用于导入；仅在开关开启且默认实例切换 OAuth 账号时写入该文件，API Key 不会覆盖官方登录。",
      networkKey: "grok.flowNotice.network",
      networkDefault:
        "网络范围：OAuth 授权、凭据刷新及账号用量查询；不会上传凭据到 Cockpit 服务。",
    },
    noAccountsKey: "grok.empty",
    noAccountsDefault: "暂无 Grok CLI 账号",
    addAccountTitleKey: "grok.addAccount",
    addAccountTitleDefault: "添加 Grok CLI 账号",
    oauthDescKey: "grok.oauth.desc",
    oauthDescDefault: "打开 xAI 授权页并输入设备验证码，完成后账号会自动保存。",
    oauthFeatureCardClassName: "grok-oauth-feature-card",
    oauthFeatureTitleKey: "grok.oauth.title",
    oauthFeatureTitleDefault: "Grok Device OAuth",
    oauthFeatureItem1Key: "grok.oauth.item1",
    oauthFeatureItem1Default:
      "使用 Grok CLI 官方 device flow，不占用本地回调端口。",
    oauthFeatureItem2Key: "grok.oauth.item2",
    oauthFeatureItem2Default:
      "授权成功后保存独立 GROK_HOME，并维护凭据有效状态。",
    oauthFeatureItem3Key: "grok.oauth.item3",
    oauthFeatureItem3Default: "每个账号独立目录，可同时启动多个账号互不影响。",
    oauthUrlInputPlaceholderKey: "grok.oauth.urlPlaceholder",
    oauthUrlInputPlaceholderDefault: "Grok OAuth 授权地址",
    oauthWaitingKey: "grok.oauth.waiting",
    oauthWaitingDefault: "等待 Grok OAuth 授权...",
    oauthOpenButtonKey: "grok.oauth.openWindow",
    oauthOpenButtonDefault: "打开授权页",
    // 与 Codex 对齐：Token / JSON（粘贴）| API Key
    tokenTabLabelKey: "grok.import.apiKeyTab",
    tokenTabLabelDefault: "API Key",
    tokenDescKey:
      apiProviderMode === "custom"
        ? "grok.import.customApiDesc"
        : "grok.import.apiKeyDesc",
    tokenDescDefault:
      apiProviderMode === "custom"
        ? "配置 OpenAI 兼容的 Base URL、模型 ID 与 API Key。启动时写入账号专属 config.toml，并仅向该 CLI 进程注入密钥。"
        : "粘贴 xAI API Key（官方 XAI_API_KEY / xai-…）。启动 CLI 时通过账号专属 GROK_HOME 注入，不会覆盖官方 OAuth 登录。",
    tokenInputPlaceholderKey:
      apiProviderMode === "custom"
        ? "grok.import.customApiKeyPlaceholder"
        : "grok.import.apiKeyPlaceholder",
    tokenInputPlaceholderDefault:
      apiProviderMode === "custom"
        ? "粘贴第三方 API Key"
        : "粘贴 xai- 开头的密钥",
    tokenSubmitLabelKey: "grok.import.apiKeyAction",
    tokenSubmitLabelDefault: "添加 API Key",
    tokenInputSecret: true,
    tokenSubmitDisabled: customApiFieldsIncomplete,
    tokenControl: (
      <div className="grok-api-provider-control">
        <div
          className="grok-api-provider-modes"
          role="tablist"
          aria-label={t("grok.import.apiMode", "API 类型")}
        >
          <button
            type="button"
            role="tab"
            aria-selected={apiProviderMode === "official"}
            className={`btn grok-api-provider-mode ${apiProviderMode === "official" ? "active" : ""}`}
            onClick={() => {
              setApiProviderMode("official");
              setApiBaseUrlTouched(false);
              setApiModelTouched(false);
              page.setAddStatus("idle");
              page.setAddMessage(null);
            }}
          >
            {t("grok.import.apiModeOfficial", "xAI 官方")}
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={apiProviderMode === "custom"}
            className={`btn grok-api-provider-mode ${apiProviderMode === "custom" ? "active" : ""}`}
            onClick={() => {
              setApiProviderMode("custom");
              setApiBaseUrlTouched(false);
              setApiModelTouched(false);
              page.setAddStatus("idle");
              page.setAddMessage(null);
            }}
          >
            {t("grok.import.apiModeCustom", "第三方兼容")}
          </button>
        </div>
      </div>
    ),
    tokenFields:
      apiProviderMode === "custom" ? (
        <div className="grok-api-provider-fields">
          <div className="grok-api-provider-field">
            <label htmlFor="grok-api-base-url">
              {t("grok.import.apiBaseUrl", "Base URL")}
            </label>
            <input
              id="grok-api-base-url"
              className={`form-input ${apiBaseUrlError ? "error" : ""}`}
              type="url"
              value={apiBaseUrl}
              autoComplete="off"
              aria-invalid={!!apiBaseUrlError}
              aria-describedby={
                apiBaseUrlError ? "grok-api-base-url-error" : undefined
              }
              placeholder={t(
                "grok.import.apiBaseUrlPlaceholder",
                "https://api.example.com/v1",
              )}
              onChange={(event) => {
                setApiBaseUrl(event.target.value);
                page.setAddStatus("idle");
                page.setAddMessage(null);
              }}
              onBlur={() => setApiBaseUrlTouched(true)}
            />
            {apiBaseUrlError && (
              <span
                id="grok-api-base-url-error"
                className="grok-api-field-error"
              >
                {apiBaseUrlError}
              </span>
            )}
          </div>
          <div className="grok-api-provider-field">
            <label htmlFor="grok-api-model">
              {t("grok.import.apiModel", "模型 ID")}
            </label>
            <input
              id="grok-api-model"
              className={`form-input ${apiModelError ? "error" : ""}`}
              value={apiModel}
              autoComplete="off"
              aria-invalid={!!apiModelError}
              aria-describedby={
                apiModelError ? "grok-api-model-error" : undefined
              }
              placeholder={t(
                "grok.import.apiModelPlaceholder",
                "例如 grok-4.1-fast",
              )}
              onChange={(event) => {
                setApiModel(event.target.value);
                page.setAddStatus("idle");
                page.setAddMessage(null);
              }}
              onBlur={() => setApiModelTouched(true)}
            />
            {apiModelError && (
              <span
                id="grok-api-model-error"
                className="grok-api-field-error"
              >
                {apiModelError}
              </span>
            )}
          </div>
        </div>
      ) : null,
    showPasteJsonTab: true,
    pasteJsonTabLabelKey: "common.shared.addModal.token",
    pasteJsonTabLabelDefault: "Token / JSON",
    pasteJsonDescKey: "grok.import.pasteDesc",
    pasteJsonDescDefault:
      "粘贴官方 Grok CLI auth.json，或本应用导出的完整账号 JSON（含凭据，可再导入恢复）。凭据仅在本机处理。",
    pasteJsonPlaceholderKey: "grok.import.pastePlaceholder",
    pasteJsonPlaceholderDefault: "粘贴 Grok 账号 JSON",
    pasteJsonSubmitLabelKey: "grok.import.pasteAction",
    pasteJsonSubmitLabelDefault: "导入 JSON",
    importLocalDescKey: "grok.import.localDesc",
    importLocalDescDefault:
      "从默认 ~/.grok/auth.json 导入；也可选择官方 auth.json 或本应用导出的完整账号 JSON 文件。",
    importLocalClientKey: "grok.import.localClient",
    importLocalClientDefault: "从本机 Grok CLI 导入",
    getDisplayEmail: getGrokAccountDisplayEmail,
    getPlanBadge: (account) =>
      getGrokPlanBadge(account) || t("common.none", "暂无"),
    getPlanBadgeTitle: (account) =>
      getGrokPlanRawValue(account) || t("common.none", "暂无"),
    getPlanBadgeClass: (planBadge, account) => {
      if (isGrokApiKeyAccount(account) || planBadge === "API_KEY") return "pro";
      // Missing tier (暂无) uses Free styling — not the red "unknown" tone.
      if (!getGrokPlanRawValue(account)) return "free";
      if (planBadge === "Free") return "free";
      if (planBadge === "--") return "free";
      return "pro";
    },
    getSearchText: (account) =>
      [
        getGrokAccountDisplayEmail(account),
        account.first_name,
        account.last_name,
        account.principal_id,
        account.team_id,
        account.api_base_url,
        account.api_model,
        account.quota?.subscriptionStatus,
        getGrokPlanBadge(account) || t("common.none", "暂无"),
      ]
        .filter(Boolean)
        .join(" "),
    getUsage: getGrokUsage,
    getQuotaGroups: getGrokQuotaGroups,
    hasQuotaData: (account) => hasGrokQuotaData(account),
    usagePrefix: "grok",
    quotaPrefix: "grok",
    tableUsageClassName: "grok-table-usage",
    showMfaQuickCode: false,
    getReauthorizationReason: getGrokReauthorizationReason,
    reauthorizingAccount: reauthTargetAccount,
    onReauthorize: handleReauthorize,
    renderQuotaSection: renderGrokQuotaSection,
  };

  return (
    <div className="ghcp-accounts-page grok-accounts-page">
      <PlatformOverviewTabsHeader
        platform="grok"
        active={activeTab}
        onTabChange={setActiveTab}
      />
      {activeTab === "instances" ? (
        <GrokInstancesContent accountsForSelect={accountsForInstances} />
      ) : (
        <CodebuddySuiteAccountsSharedView
          accounts={store.accounts}
          loading={store.loading}
          page={page}
          platformConfig={platformConfig}
          onRefreshAccounts={() => void store.fetchAccounts()}
        />
      )}
      {launchModal && (
        <div className="modal-overlay">
          <div
            className="modal modal-lg"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="modal-header">
              <button
                className="btn btn-secondary icon-only"
                onClick={() => setLaunchModal(null)}
                title={t("common.back", "返回")}
                aria-label={t("common.back", "返回")}
              >
                <ChevronLeft size={14} />
              </button>
              <h2>{t("grok.instances.launchDialogTitle", "启动实例")}</h2>
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
                <span>
                  {t("accounts.switched", "已切换至 {{email}}", {
                    email: launchModal.accountEmail,
                  })}
                </span>
              </div>
              <ModalErrorMessage
                message={launchModal.executeError}
                scrollKey={launchModal.errorScrollKey}
              />
              {launchModal.executeError &&
                grokInstanceService.isGrokCliMissingError(
                  launchModal.executeError,
                ) &&
                renderGrokCliInstallGuide()}
              <div className="form-group">
                <label>{t("instances.columns.instance", "实例")}</label>
                <input
                  className="form-input"
                  value={t("instances.defaultName", "默认实例")}
                  readOnly
                />
              </div>
              <div className="form-group">
                <label>{t("instances.form.workingDir", "工作目录")}</label>
                <div className="grok-launch-working-dir-row">
                  <input
                    className="form-input"
                    value={launchModal.workingDir}
                    placeholder={t(
                      "instances.form.workingDirPlaceholder",
                      "默认当前路径",
                    )}
                    onChange={(event) =>
                      updateLaunchWorkingDir(event.target.value)
                    }
                    onBlur={() => void handleLaunchWorkingDirBlur()}
                    disabled={
                      launchModal.executing || launchModal.regeneratingCommand
                    }
                  />
                  <button
                    className="btn btn-secondary"
                    type="button"
                    onClick={() => void handleChooseLaunchWorkingDir()}
                    disabled={
                      launchModal.executing || launchModal.regeneratingCommand
                    }
                    title={t(
                      "grok.instances.selectWorkingDir",
                      "选择 Grok CLI 工作目录",
                    )}
                    aria-label={t(
                      "grok.instances.selectWorkingDir",
                      "选择 Grok CLI 工作目录",
                    )}
                  >
                    <FolderOpen size={16} />
                  </button>
                </div>
                <p className="form-hint">
                  {t(
                    "grok.instances.workingDirAccountHint",
                    "工作目录与该账号绑定，下次从该账号启动会自动回填。",
                  )}
                </p>
              </div>
              <div className="form-group">
                <label>{t("instances.launchDialog.command", "启动命令")}</label>
                <textarea
                  className="form-input instance-args-input"
                  value={launchModal.launchCommand}
                  placeholder={
                    launchModal.regeneratingCommand
                      ? t("common.loading", "加载中...")
                      : t(
                          "grok.instances.launchCommandPlaceholder",
                          "选择或确认工作目录后生成启动命令",
                        )
                  }
                  readOnly
                />
                <p className="form-hint">
                  {t(
                    "grok.instances.launchHint",
                    "可复制命令手动执行，或点击下方按钮直接在终端执行。",
                  )}
                </p>
              </div>
              <div className="form-group">
                <label>{t("instances.launchDialog.terminal", "终端")}</label>
                <SingleSelectDropdown
                  value={selectedTerminal}
                  onChange={handleTerminalChange}
                  options={terminalOptions}
                  disabled={
                    launchModal.executing || launchModal.regeneratingCommand
                  }
                  ariaLabel={t("instances.launchDialog.terminal", "终端")}
                />
              </div>
              {launchModal.executeMessage && (
                <div className="add-status success">
                  <Check size={16} />
                  <span>{launchModal.executeMessage}</span>
                </div>
              )}
            </div>
            <div className="modal-footer">
              <button
                className="btn btn-secondary"
                onClick={() => void handleCopyLaunchCommand()}
                disabled={
                  launchModal.executing || launchModal.regeneratingCommand
                }
              >
                <Copy size={16} />
                {launchModal.copied
                  ? t("common.success", "成功")
                  : t("common.copy", "复制")}
              </button>
              <button
                className="btn btn-primary"
                onClick={() => void handleExecuteInTerminal()}
                disabled={
                  launchModal.executing || launchModal.regeneratingCommand
                }
              >
                <Play size={16} />
                {launchModal.executing
                  ? t("common.loading", "加载中...")
                  : t("grok.instances.runInTerminal", "终端执行")}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
