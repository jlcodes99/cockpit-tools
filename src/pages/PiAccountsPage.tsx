import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Check,
  ChevronLeft,
  Copy,
  Download,
  FolderOpen,
  Pencil,
  Play,
  Terminal,
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
import { PiSyncToggle } from "../components/pi/PiSyncToggle";
import { PiAccountDefaultsModal } from "../components/pi/PiAccountDefaultsModal";
import { PiUsageSection } from "../components/pi/PiUsageSection";
import { loadPiAccountUsage } from "../services/piUsageCache";
import {
  PiGatewayFields,
  createPiGatewayState,
  isPiGatewayStateReady,
  type PiGatewayState,
} from "../components/pi/PiGatewayFields";
import { useProviderAccountsPage } from "../hooks/useProviderAccountsPage";
import { useEscClose } from "../hooks/useEscClose";
import { useLaunchTerminalOptions } from "../hooks/useLaunchTerminalOptions";
import * as piInstanceService from "../services/piInstanceService";
import * as piService from "../services/piService";
import { usePiAccountStore } from "../stores/usePiAccountStore";
import {
  getPiAccountDisplayEmail,
  getPiPlanBadge,
  getPiProvidersText,
  getPiQuotaGroups,
  getPiUsage,
  isPiOAuthExpired,
  type PiAccount,
} from "../types/pi";
import { PiInstancesContent } from "./PiInstancesPage";

const FLOW_NOTICE_KEY = "agtools.pi.flow_notice_collapsed";
const CURRENT_ACCOUNT_KEY = "agtools.pi.current_account_id";
const PI_CLI_INSTALL_COMMAND_UNIX = "curl -fsSL https://pi.dev/install.sh | sh";
const PI_CLI_INSTALL_COMMAND_WINDOWS = "irm https://pi.dev/install.ps1 | iex";

function getPiCliInstallCommand(): string {
  if (typeof navigator === "undefined") {
    return PI_CLI_INSTALL_COMMAND_UNIX;
  }
  const platform = `${navigator.platform || ""} ${navigator.userAgent || ""}`;
  return /win/i.test(platform)
    ? PI_CLI_INSTALL_COMMAND_WINDOWS
    : PI_CLI_INSTALL_COMMAND_UNIX;
}


interface PiAccountLaunchModalState {
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

export function PiAccountsPage() {
  const { t } = useTranslation();
  const [activeTab, setActiveTab] = useState<PlatformOverviewTab>("overview");
  const [launchModal, setLaunchModal] =
    useState<PiAccountLaunchModalState | null>(null);
  const { terminalOptions, selectedTerminal, setSelectedTerminal } =
    useLaunchTerminalOptions();
  const piCliInstallCommand = useMemo(() => getPiCliInstallCommand(), []);
  const [installCommandCopied, setInstallCommandCopied] = useState(false);
  const [installExecuting, setInstallExecuting] = useState(false);
  const [installOpened, setInstallOpened] = useState(false);
  const [installError, setInstallError] = useState<string | null>(null);
  const [installErrorScrollKey, setInstallErrorScrollKey] = useState(0);
  const [gateway, setGateway] = useState<PiGatewayState>(createPiGatewayState);
  const [loginExecuting, setLoginExecuting] = useState(false);
  const [loginOpened, setLoginOpened] = useState(false);
  const [loginError, setLoginError] = useState<string | null>(null);
  const [loginErrorScrollKey, setLoginErrorScrollKey] = useState(0);
  const [editingDefaultsAccount, setEditingDefaultsAccount] =
    useState<PiAccount | null>(null);
  const store = usePiAccountStore();
  // "cli" keeps the old pi /login terminal guide as a fallback.
  const [oauthProvider, setOauthProvider] = useState<
    piService.PiOAuthProvider | "cli"
  >("anthropic");
  const oauthProviderRef = useRef(oauthProvider);
  oauthProviderRef.current = oauthProvider;

  useEscClose(!!launchModal, () => setLaunchModal(null));

  const page = useProviderAccountsPage<PiAccount>({
    platformKey: "pi",
    oauthLogPrefix: "PiLogin",
    flowNoticeCollapsedKey: FLOW_NOTICE_KEY,
    currentAccountIdKey: CURRENT_ACCOUNT_KEY,
    exportFilePrefix: "pi_accounts",
    oauthTabKeys: ["oauth"],
    oauthAutoPrepare: () => oauthProviderRef.current !== "cli",
    oauthService: {
      startLogin: () => {
        const provider = oauthProviderRef.current;
        if (provider === "cli") {
          return Promise.reject(new Error("cli"));
        }
        return piService.startPiOAuthLogin(provider);
      },
      completeLogin: piService.completePiOAuthLogin,
      cancelLogin: piService.cancelPiOAuthLogin,
      submitCallbackUrl: piService.submitPiOAuthCallbackUrl,
    },
    store: {
      accounts: store.accounts,
      currentAccountId: store.currentAccountId,
      loading: store.loading,
      error: store.error,
      fetchAccounts: store.fetchAccounts,
      fetchCurrentAccountId: store.fetchCurrentAccountId,
      deleteAccounts: store.deleteAccounts,
      // Refresh re-queries usage; expired OAuth tokens are refreshed and written back.
      refreshToken: (accountId: string) => loadPiAccountUsage(accountId, true),
      refreshAllTokens: async () => {
        await Promise.all(
          store.accounts.map((account) => loadPiAccountUsage(account.id, true)),
        );
      },
      setCurrentAccountId: store.setCurrentAccountId,
      updateAccountTags: store.updateAccountTags,
    },
    dataService: {
      importFromJson: piService.importPiFromJson,
      importFromLocal: piService.importPiFromLocal,
      exportAccounts: piService.exportPiAccounts,
      injectToVSCode: piService.switchPiAccount,
      addWithToken: (apiKey) =>
        piService.addPiAccountWithApiKey(gateway.builtinProvider, apiKey, {
          displayName: gateway.displayName,
          defaultModel: gateway.defaultModel,
        }),
    },
    getDisplayEmail: getPiAccountDisplayEmail,
    onInjectSuccess: async ({ accountId, account, displayEmail }) => {
      const accountEmail = account
        ? getPiAccountDisplayEmail(account)
        : displayEmail || accountId;
      const workingDir = account?.working_dir?.trim() || "";
      try {
        const launchInfo =
          await piInstanceService.getPiInstanceLaunchCommand("__default__", {
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
          executeError: null,
          errorScrollKey: 0,
        });
      } catch (error) {
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
          executeError: String(error),
          errorScrollKey: 1,
        });
      }
    },
    resolveOauthSuccessMessage: () => t("pi.oauth.success", "pi 登录成功"),
  });

  // Switching provider cancels the running login and restarts it. For "cli"
  // the restart is rejected and the terminal guide is shown instead.
  const previousOauthProviderRef = useRef(oauthProvider);
  useEffect(() => {
    if (previousOauthProviderRef.current === oauthProvider) return;
    previousOauthProviderRef.current = oauthProvider;
    if (page.showAddModal && page.addTab === "oauth") {
      page.handleRetryOauth();
    }
  }, [oauthProvider, page.addTab, page.handleRetryOauth, page.showAddModal]);

  useEffect(() => {
    if (!page.showAddModal) {
      setGateway(createPiGatewayState());
      setLoginOpened(false);
      setLoginError(null);
    }
  }, [page.showAddModal]);

  const persistAccountWorkingDir = useCallback(
    async (accountId: string, workingDir: string) => {
      await piService.updatePiAccountWorkingDir(
        accountId,
        workingDir.trim() || null,
      );
      await store.fetchAccounts();
    },
    [store],
  );

  const regenerateLaunchCommand = useCallback(
    async (modal: PiAccountLaunchModalState, workingDir: string) => {
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
          await piInstanceService.getPiInstanceLaunchCommand(
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
                executeError: message,
                errorScrollKey: current.errorScrollKey + 1,
              }
            : current,
        );
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
      title: t("pi.instances.selectWorkingDir", "选择 pi CLI 工作目录"),
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
    modal: PiAccountLaunchModalState,
  ): Promise<PiAccountLaunchModalState | null> => {
    if (modal.launchCommand.trim() && !modal.regeneratingCommand) {
      return modal;
    }
    try {
      await persistAccountWorkingDir(modal.accountId, modal.workingDir);
      const launchInfo = await piInstanceService.getPiInstanceLaunchCommand(
        modal.instanceId,
        {
          workingDir: modal.workingDir,
          applyWorkingDirOverride: true,
          accountId: modal.accountId,
        },
      );
      const next: PiAccountLaunchModalState = {
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
      const result = await piInstanceService.executePiInstanceLaunchCommand(
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
      await navigator.clipboard.writeText(piCliInstallCommand);
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
      await piInstanceService.executePiCliInstallCommand(selectedTerminal);
      setInstallOpened(true);
    } catch (error) {
      reportInstallError(String(error));
    } finally {
      setInstallExecuting(false);
    }
  };

  const renderPiCliInstallGuide = () => (
    <div className="grok-cli-install-guide">
      <strong>{t("pi.instances.installCommand", "官方安装命令")}</strong>
      <p>
        {t(
          "pi.instances.installLaunchHint",
          "可在终端运行以下官方命令，安装完成后重新点击终端执行。",
        )}
      </p>
      <div className="grok-cli-install-command">
        <code>{piCliInstallCommand}</code>
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
            : t("pi.instances.runInTerminal", "终端执行")}
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

  const handleOpenLoginTerminal = async () => {
    if (loginExecuting) return;
    setLoginError(null);
    setLoginOpened(false);
    setLoginExecuting(true);
    try {
      await piInstanceService.executePiLoginCommand(selectedTerminal);
      setLoginOpened(true);
    } catch (error) {
      setLoginError(String(error));
      setLoginErrorScrollKey((current) => current + 1);
    } finally {
      setLoginExecuting(false);
    }
  };

  const loginMissingCli =
    !!loginError && piInstanceService.isPiCliMissingError(loginError);

  const renderLoginGuide = () => (
    <div className="pi-login-guide">
      <ol className="pi-login-steps">
        <li>
          {t(
            "pi.login.step1",
            "点击下方按钮，在终端中启动 pi（使用官方目录 ~/.pi/agent）。",
          )}
        </li>
        <li>
          {t(
            "pi.login.step2",
            "在 pi 中输入 /login，选择提供商并完成浏览器授权；可重复登录多个提供商。",
          )}
        </li>
        <li>
          {t(
            "pi.login.step3",
            "登录完成后回到这里，点击「从本机导入」保存为账号。",
          )}
        </li>
      </ol>
      <div className="grok-cli-install-actions">
        <div className="grok-cli-install-terminal">
          <label>{t("instances.launchDialog.terminal", "终端")}</label>
          <SingleSelectDropdown
            value={selectedTerminal}
            onChange={(terminal) => {
              setSelectedTerminal(terminal);
              setLoginError(null);
              setLoginOpened(false);
            }}
            options={terminalOptions}
            disabled={loginExecuting}
          />
        </div>
        <button
          type="button"
          className="btn btn-secondary"
          onClick={() => void handleOpenLoginTerminal()}
          disabled={loginExecuting}
        >
          <Terminal size={14} />
          {loginExecuting
            ? t("common.loading", "加载中...")
            : t("pi.login.openTerminal", "在终端启动 pi")}
        </button>
        <button
          type="button"
          className="btn btn-primary"
          onClick={() => void page.handleImportFromLocal?.()}
          disabled={!page.handleImportFromLocal || page.addStatus === "loading"}
        >
          <Download size={14} />
          {t("pi.login.importAfterLogin", "从本机导入")}
        </button>
      </div>
      {loginOpened && (
        <div className="add-status success">
          <Check size={14} />
          <span>
            {t("pi.login.opened", "已打开终端，请在 pi 中输入 /login")}
          </span>
        </div>
      )}
      <ModalErrorMessage message={loginError} scrollKey={loginErrorScrollKey} />
      {loginMissingCli && renderPiCliInstallGuide()}
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

  const renderPiProvidersSection = useCallback(
    (account: PiAccount, variant: "card" | "table") => {
      const providers = account.providers ?? [];
      const now = Date.now();
      return (
        <div className={`pi-providers-summary ${variant}`}>
          {providers.length === 0 ? (
            <div
              className={variant === "card" ? "quota-empty" : ""}
              style={
                variant === "table"
                  ? { color: "var(--text-muted)", fontSize: 13 }
                  : undefined
              }
            >
              {t("pi.providers.empty", "暂无凭据")}
            </div>
          ) : (
            <div className="pi-provider-chips">
              {providers.map((item) => {
                const expired = isPiOAuthExpired(item, now);
                const kindLabel = item.base_url
                  ? t("pi.providers.gateway", "网关")
                  : item.kind === "oauth"
                    ? t("pi.providers.oauth", "OAuth")
                    : t("pi.providers.apiKey", "API Key");
                return (
                  <span
                    key={`${account.id}-${item.provider}`}
                    className={`pi-provider-chip${expired ? " is-expired" : ""}`}
                    title={
                      expired
                        ? t(
                            "pi.providers.expiredHint",
                            "OAuth 令牌已过期，pi 启动时会自动刷新",
                          )
                        : item.base_url
                          ? `${item.provider} · ${item.api ?? ""} · ${item.base_url}`
                          : `${item.provider} · ${kindLabel}`
                    }
                  >
                    {item.provider}
                    <span className="pi-provider-kind">{kindLabel}</span>
                  </span>
                );
              })}
            </div>
          )}
          {account.default_model && (
            <div className="pi-default-model" title={account.default_model}>
              {t("pi.providers.defaultModel", "默认模型")}:{" "}
              {account.default_provider
                ? `${account.default_provider}/${account.default_model}`
                : account.default_model}
            </div>
          )}
          {providers.length > 0 && (
            <PiUsageSection accountId={account.id} variant={variant} />
          )}
        </div>
      );
    },
    [t],
  );

  const platformConfig: CodebuddySuiteAccountsPlatformConfig<PiAccount> = {
    pageClassName: "pi-accounts-page",
    searchPlaceholderKey: "pi.search",
    searchPlaceholderDefault: "搜索 pi 账号...",
    flowNotice: {
      titleKey: "pi.flowNotice.title",
      titleDefault: "pi 账号管理说明",
      descKey: "pi.flowNotice.desc",
      descDefault:
        "每个 pi 账号是一组提供商凭据和默认模型。开启「切号同步官方登录」后，切换账号会把凭据合并写入 ~/.pi/agent/auth.json 并更新默认模型；关闭时使用独立 PI_CODING_AGENT_DIR。",
      permissionKey: "pi.flowNotice.permission",
      permissionDefault:
        "本地范围：读取 ~/.pi/agent/auth.json 与 settings.json 用于导入；切换时仅修改账号涉及的提供商条目，写入前保留一份 .cockpit-backup 备份。",
      networkKey: "pi.flowNotice.network",
      networkDefault:
        "网络范围：内置登录会访问所选提供商的授权与令牌接口（Anthropic / OpenAI / GitHub / Kimi / xAI / OpenRouter），网关「获取模型列表」会请求你填写的 Base URL；令牌刷新由 pi CLI 自行完成。",
    },
    noAccountsKey: "pi.empty",
    noAccountsDefault: "暂无 pi 账号",
    addAccountTitleKey: "pi.addAccount",
    addAccountTitleDefault: "添加 pi 账号",
    oauthDescKey:
      oauthProvider === "cli" ? "pi.oauth.desc" : "pi.oauth.builtinDesc",
    oauthDescDefault:
      oauthProvider === "cli"
        ? "pi 的订阅登录（Claude Pro/Max、ChatGPT、GitHub Copilot、Gemini 等）通过 pi 内置的 /login 完成。"
        : "在浏览器中完成授权后自动保存为 pi 账号，凭据格式与 pi /login 完全一致。",
    oauthCustomContent: oauthProvider === "cli" ? renderLoginGuide() : undefined,
    oauthProviderControl: (
      <div className="form-group">
        <div className="pi-segmented">
          {(
            [
              ["anthropic", "Claude Pro/Max"],
              ["openai-codex", "ChatGPT (Codex)"],
              ["github-copilot", "GitHub Copilot"],
              ["kimi-coding", "Kimi Code"],
              ["xai", "xAI (Grok)"],
              ["openrouter", "OpenRouter"],
              ["cli", t("pi.oauth.otherProviders", "其他（pi /login）")],
            ] as const
          ).map(([id, label]) => (
            <button
              key={id}
              type="button"
              className={`claude-provider-endpoint-chip ${oauthProvider === id ? "active" : ""}`}
              aria-pressed={oauthProvider === id}
              onClick={() => setOauthProvider(id)}
            >
              {label}
            </button>
          ))}
        </div>
      </div>
    ),
    oauthFeatureCardClassName: "grok-oauth-feature-card",
    oauthFeatureTitleKey: "pi.oauth.title",
    oauthFeatureTitleDefault: "pi /login",
    oauthFeatureItem1Key: "pi.oauth.item1",
    oauthFeatureItem1Default: "使用 pi 官方登录流程，凭据由 pi 自行写入。",
    oauthFeatureItem2Key: "pi.oauth.item2",
    oauthFeatureItem2Default: "导入时会把当前所有提供商凭据保存为一个账号。",
    oauthFeatureItem3Key: "pi.oauth.item3",
    oauthFeatureItem3Default: "每个账号可使用独立目录，多开互不影响。",
    oauthUrlInputPlaceholderKey: "pi.oauth.urlPlaceholder",
    oauthUrlInputPlaceholderDefault: "授权地址",
    oauthWaitingKey: "pi.oauth.waiting",
    oauthWaitingDefault: "等待浏览器授权回调...",
    tokenTabLabelKey: "pi.import.apiKeyTab",
    tokenTabLabelDefault: "API Key",
    tokenDescKey: "pi.import.apiKeyDesc",
    tokenDescDefault:
      "接入三方 API 网关（OpenAI / Anthropic 兼容），或直接填写 pi 内置提供商的 API Key。",
    tokenInputPlaceholderKey: "pi.import.apiKeyPlaceholder",
    tokenInputPlaceholderDefault: "粘贴 API Key",
    tokenSubmitLabelKey: "pi.import.apiKeyAction",
    tokenSubmitLabelDefault: "添加 API Key",
    tokenInputSecret: true,
    tokenSubmitDisabled: !isPiGatewayStateReady(gateway),
    tokenFields: (
      <PiGatewayFields
        state={gateway}
        onChange={(patch) => {
          setGateway((prev) => ({ ...prev, ...patch }));
          page.setAddStatus("idle");
          page.setAddMessage(null);
        }}
      />
    ),
    showPasteJsonTab: true,
    pasteJsonTabLabelKey: "common.shared.addModal.token",
    pasteJsonTabLabelDefault: "Token / JSON",
    pasteJsonDescKey: "pi.import.pasteDesc",
    pasteJsonDescDefault:
      "粘贴 pi 的 auth.json，或本应用导出的账号 JSON。凭据仅在本机处理。",
    pasteJsonPlaceholderKey: "pi.import.pastePlaceholder",
    pasteJsonPlaceholderDefault: "粘贴 pi 账号 JSON",
    pasteJsonSubmitLabelKey: "pi.import.pasteAction",
    pasteJsonSubmitLabelDefault: "导入 JSON",
    importLocalDescKey: "pi.import.localDesc",
    importLocalDescDefault:
      "从 ~/.pi/agent/auth.json 导入当前所有提供商凭据和默认模型。",
    importLocalClientKey: "pi.import.localClient",
    importLocalClientDefault: "从本机 pi 导入",
    getDisplayEmail: getPiAccountDisplayEmail,
    getPlanBadge: (account) =>
      getPiPlanBadge(account) || t("common.none", "暂无"),
    getPlanBadgeTitle: (account) =>
      getPiProvidersText(account) || t("common.none", "暂无"),
    getPlanBadgeClass: (_planBadge, account) =>
      (account.providers ?? []).length > 0 ? "pro" : "free",
    getSearchText: (account) =>
      [
        getPiAccountDisplayEmail(account),
        account.default_provider,
        account.default_model,
        ...(account.providers ?? []).map((item) => item.provider),
      ]
        .filter(Boolean)
        .join(" "),
    getUsage: getPiUsage,
    getQuotaGroups: getPiQuotaGroups,
    hasQuotaData: () => false,
    usagePrefix: "pi",
    quotaPrefix: "pi",
    tableUsageClassName: "pi-table-usage",
    showMfaQuickCode: false,
    renderQuotaSection: renderPiProvidersSection,
    hideRefreshAction: true,
    renderAccountActions: (account, variant) => (
      <button
        className={variant === "card" ? "card-action-btn" : "action-btn"}
        onClick={() => setEditingDefaultsAccount(account)}
        title={t("pi.defaults.title", "编辑默认设置")}
        aria-label={t("pi.defaults.title", "编辑默认设置")}
      >
        <Pencil size={14} />
      </button>
    ),
    toolbarExtra: <PiSyncToggle />,
  };

  return (
    <div className="ghcp-accounts-page grok-accounts-page pi-accounts-page">
      <PlatformOverviewTabsHeader
        platform="pi"
        active={activeTab}
        onTabChange={setActiveTab}
      />
      {activeTab === "instances" ? (
        <PiInstancesContent accountsForSelect={accountsForInstances} />
      ) : (
        <CodebuddySuiteAccountsSharedView
          accounts={store.accounts}
          loading={store.loading}
          page={page}
          platformConfig={platformConfig}
          onRefreshAccounts={() => void store.fetchAccounts()}
        />
      )}
      {editingDefaultsAccount && (
        <PiAccountDefaultsModal
          account={editingDefaultsAccount}
          onClose={() => setEditingDefaultsAccount(null)}
          onSaved={() => store.fetchAccounts()}
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
              <h2>{t("pi.instances.launchDialogTitle", "启动实例")}</h2>
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
                piInstanceService.isPiCliMissingError(
                  launchModal.executeError,
                ) &&
                renderPiCliInstallGuide()}
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
                      "pi.instances.selectWorkingDir",
                      "选择 pi CLI 工作目录",
                    )}
                    aria-label={t(
                      "pi.instances.selectWorkingDir",
                      "选择 pi CLI 工作目录",
                    )}
                  >
                    <FolderOpen size={16} />
                  </button>
                </div>
                <p className="form-hint">
                  {t(
                    "pi.instances.workingDirAccountHint",
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
                          "pi.instances.launchCommandPlaceholder",
                          "选择或确认工作目录后生成启动命令",
                        )
                  }
                  readOnly
                />
                <p className="form-hint">
                  {t(
                    "pi.instances.launchHint",
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
                  : t("pi.instances.runInTerminal", "终端执行")}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
