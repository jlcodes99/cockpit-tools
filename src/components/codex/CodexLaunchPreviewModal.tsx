import {
  ArrowRight,
  BarChart3,
  Check,
  ChevronDown,
  CircleAlert,
  ImagePlus,
  KeyRound,
  Link2,
  Play,
  RefreshCw,
  Route,
  Save,
  Server,
  SlidersHorizontal,
  UserRound,
  Wrench,
  X,
} from "lucide-react";
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { useTranslation } from "react-i18next";
import { confirm as confirmDialog } from "@tauri-apps/plugin-dialog";
import { useEscClose } from "../../hooks/useEscClose";
import {
  saveCodexInstanceQuickConfig,
  saveCodexInstanceConfiguration,
  getCodexInstanceQuickConfig,
} from "../../services/codexInstanceService";
import { useCodexAccountStore } from "../../stores/useCodexAccountStore";
import { useCodexInstanceStore } from "../../stores/useCodexInstanceStore";
import type { CodexInstanceApiRoute } from "../../types/instance";
import {
  CodexModelRoutingFields,
  buildCodexModelRoutingValue,
  createCodexModelSourceResolver,
  collectRouteUpstreamModels,
  eligibleCodexModelRoutingAccounts,
  normalizeCodexModelRoutingRoutes,
  shortCodexRouteAccountLabel,
  syncExperimentalModelsWithRouting,
  toggleRouteModelInRoutes,
} from "./CodexModelRoutingFields";
import { forceRefreshCodexTokens } from "../../services/codexService";
import { requestCodexOpenAddAccount } from "../../utils/codexAddAccountRequest";
import { areCodexModelRoutingsEqual, resolveRoutingCatalog } from "../../utils/codexModelRoutingValue";
import type {
  CodexAccount,
  CodexExperimentalModelDefinition,
  CodexQuickConfig,
} from "../../types/codex";
import {
  formatCodexLoginProvider,
  getCodexAuthMetadata,
  getCodexSubscriptionPresentationForAccount,
  isCodexApiKeyAccount,
  isStandardCodexOAuthAccount,
} from "../../types/codex";
import {
  DEEPSEEK_ACCESS_MODE_CDP,
  DEEPSEEK_ACCESS_MODE_DIRECT,
  DEEPSEEK_ACCESS_MODE_GATEWAY,
  isDeepSeekAccount,
  isDeepSeekResponsesAccount,
  resolveDeepSeekAccessMode,
  type DeepSeekAccessMode,
} from "../../utils/codexDeepSeekAccess";
import { CodexImageAccountPickerModal } from "./CodexImageAccountPickerModal";
import { getCodexJwtExpiration } from "../../utils/codexSwitchAuthFailure";
import type { UnifiedQuotaMetric } from "../../presentation/platformAccountPresentation";
import { buildCodexAccountPresentation } from "../../presentation/platformAccountPresentation";
import { ModalErrorMessage, useModalErrorState } from "../ModalErrorMessage";
import {
  SingleSelectDropdown,
  type SingleSelectOption,
} from "../SingleSelectDropdown";
import { CodexQuotaMiniRows } from "./CodexQuotaMiniRows";
import {
  CodexContextOverrideEditor,
  resolveCodexContextOverridePreset,
} from "./CodexContextOverrideEditor";
import { CodexExperimentalModelEditor } from "./CodexExperimentalModelEditor";
import { CodexSessionVisibilityRepairModal } from "./CodexSessionVisibilityRepairModal";
import "./CodexLaunchPreviewModal.css";

export const DEFAULT_CODEX_INSTANCE_ID = "__default__";

export interface CodexLaunchPreviewFact {
  label: string;
  value: string;
  monospace?: boolean;
  wide?: boolean;
  tone?: "warning" | "danger";
}

export interface CodexLaunchPreviewUsage {
  label: string;
  requests?: string | null;
  tokens?: string | null;
  cost?: string | null;
  extraLabel?: string | null;
  extraValue?: string | null;
}

export interface CodexLaunchPreviewSummary {
  badgeLabel?: string;
  contextText?: string;
  statusLabel?: string;
  statusTone?: "success" | "warning" | "neutral";
  facts?: CodexLaunchPreviewFact[];
  quotaItems?: UnifiedQuotaMetric[];
  usage?: CodexLaunchPreviewUsage | null;
  tags?: string[];
  footerText?: string;
}

export interface CodexLaunchPreviewAction {
  id: string;
  label: string;
  description: string;
  actionLabel?: string;
  control?: ReactNode;
  disabled?: boolean;
  tone?: "default" | "danger";
  onAction?: () => void | Promise<void>;
}

/** 启动预览里选好的启动配置，确认时交给调用方落盘。 */
export interface CodexLaunchPreviewLaunchOptions {
  deepSeekAccessMode?: DeepSeekAccessMode;
  imageGenerationAccountIds?: string[];
}

/** 启动预览里的 OAuth 绑定状态（仅 API Key 账号展示）。 */
export interface CodexLaunchPreviewOAuthBinding {
  /** 已绑定的 OAuth 账号展示名；未绑定时为 null。 */
  boundAccountLabel?: string | null;
  /** 绑定的 OAuth 账号需要重新授权。 */
  needsReauth?: boolean;
  reauthDescription?: string | null;
}

interface CodexLaunchPreviewModalProps {
  account?: CodexAccount | null;
  accountLabel: string;
  accountMetaLabel?: string;
  oauthBinding?: CodexLaunchPreviewOAuthBinding | null;
  onBindOAuth?: () => void;
  onReauthorizeOAuth?: () => void;
  summary?: CodexLaunchPreviewSummary;
  actions?: CodexLaunchPreviewAction[];
  instanceId?: string;
  instanceLabel?: string;
  instanceOptions?: SingleSelectOption[];
  onInstanceChange?: (instanceId: string) => void | Promise<void>;
  mode?: "account" | "instance" | "apiService";
  onClose: () => void;
  onExecute: (
    launchAfterSwitch: boolean,
    launchOptions?: CodexLaunchPreviewLaunchOptions,
  ) => Promise<boolean>;
}

interface ModelConfigSnapshot {
  enabled: boolean;
  models: CodexExperimentalModelDefinition[];
  defaultModelId: string | null;
}

interface ContextConfigSnapshot {
  enabled: boolean;
  contextWindowInput: string;
  compactLimitInput: string;
}

export function CodexLaunchPreviewModal({
  account,
  accountLabel,
  accountMetaLabel,
  oauthBinding,
  onBindOAuth,
  onReauthorizeOAuth,
  summary,
  actions,
  instanceId = DEFAULT_CODEX_INSTANCE_ID,
  instanceLabel,
  instanceOptions,
  onInstanceChange,
  mode = "account",
  onClose,
  onExecute,
}: CodexLaunchPreviewModalProps) {
  const { t, i18n } = useTranslation();
  const [loadedConfig, setLoadedConfig] = useState<CodexQuickConfig | null>(
    null,
  );
  const [catalogEnabled, setCatalogEnabled] = useState(false);
  const [models, setModels] = useState<CodexExperimentalModelDefinition[]>([]);
  const [defaultModelId, setDefaultModelId] = useState<string | null>(null);
  const [contextOverrideEnabled, setContextOverrideEnabled] = useState(false);
  const [contextWindowInput, setContextWindowInput] = useState("");
  const [compactLimitInput, setCompactLimitInput] = useState("");
  const [modelsError, setModelsError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [changingInstance, setChangingInstance] = useState(false);
  const [runningActionId, setRunningActionId] = useState<string | null>(null);
  const [executing, setExecuting] = useState<"switch" | "launch" | null>(null);
  const [repairOpen, setRepairOpen] = useState(false);
  const [modelConfigOpen, setModelConfigOpen] = useState(false);
  const [contextConfigOpen, setContextConfigOpen] = useState(false);
  const [imageModelConfigOpen, setImageModelConfigOpen] = useState(false);
  const [forceRefreshing, setForceRefreshing] = useState(false);
  const [manualRefreshResult, setManualRefreshResult] = useState<{
    status: "running" | "success" | "error";
    error?: string;
  } | null>(null);
  const [manualRefreshedAccount, setManualRefreshedAccount] =
    useState<CodexAccount | null>(null);
  const [modelConfigSnapshot, setModelConfigSnapshot] =
    useState<ModelConfigSnapshot | null>(null);
  const [contextConfigSnapshot, setContextConfigSnapshot] =
    useState<ContextConfigSnapshot | null>(null);
  const [routingEnabled, setRoutingEnabled] = useState(false);
  const persistRoutingDisableRef = useRef(false);
  const [routingRoutes, setRoutingRoutes] = useState<CodexInstanceApiRoute[]>(
    [],
  );
  const [notice, setNotice] = useState<string | null>(null);
  const initialImageGenerationAccountIds = useMemo(
    () =>
      (account?.api_image_generation_account_ids ?? []).filter(
        (value): value is string =>
          typeof value === "string" && value.trim().length > 0,
      ),
    [account?.api_image_generation_account_ids],
  );
  const [deepSeekAccessMode, setDeepSeekAccessMode] =
    useState<DeepSeekAccessMode>(() =>
      resolveDeepSeekAccessMode(account ?? undefined),
    );
  const [deepSeekAccessModeDialogOpen, setDeepSeekAccessModeDialogOpen] =
    useState(false);
  const [imageGenEnabled, setImageGenEnabled] = useState(
    () => initialImageGenerationAccountIds.length > 0,
  );
  const [imageGenAccountIds, setImageGenAccountIds] = useState<string[]>(
    () => initialImageGenerationAccountIds,
  );
  const [imageGenPickerOpen, setImageGenPickerOpen] = useState(false);
  const [imageGenModeSwitchOpen, setImageGenModeSwitchOpen] = useState(false);
  const accountId = account?.id ?? null;

  useEffect(() => {
    setDeepSeekAccessMode(resolveDeepSeekAccessMode(account ?? undefined));
    setImageGenEnabled(initialImageGenerationAccountIds.length > 0);
    setImageGenAccountIds(initialImageGenerationAccountIds);
    // 只在切换账号时重置草稿，避免账号列表刷新覆盖用户正在编辑的内容。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [accountId]);

  // 只有网关模式支持 GPT 生图；切到直连 / CDP 时自动取消勾选并清空账号。
  const deepSeekImageGenAvailable =
    deepSeekAccessMode === DEEPSEEK_ACCESS_MODE_GATEWAY;
  useEffect(() => {
    if (
      !(account && isDeepSeekAccount(account)) ||
      deepSeekImageGenAvailable ||
      !imageGenEnabled
    ) {
      return;
    }
    setImageGenEnabled(false);
    setImageGenAccountIds([]);
  }, [account, deepSeekImageGenAvailable, imageGenEnabled]);

  const accounts = useCodexAccountStore((state) => state.accounts);
  const currentAccount = useCodexAccountStore((state) => state.currentAccount);
  const fetchAccounts = useCodexAccountStore((state) => state.fetchAccounts);
  const instances = useCodexInstanceStore((state) => state.instances);
  const selectedInstance = useMemo(
    () =>
      instances.find((item) => item.id === instanceId) ??
      (instanceId === DEFAULT_CODEX_INSTANCE_ID
        ? instances.find((item) => item.isDefault)
        : undefined),
    [instanceId, instances],
  );
  const resolveModelSource = useMemo(
    () =>
      createCodexModelSourceResolver(
        normalizeCodexModelRoutingRoutes(routingRoutes, accounts),
        accounts,
        t,
      ),
    [accounts, routingRoutes, t],
  );
  const availableChannels = useMemo(() => {
    if (!routingEnabled) return [];
    const providerAccounts = eligibleCodexModelRoutingAccounts(accounts);
    return normalizeCodexModelRoutingRoutes(routingRoutes, accounts)
      .filter((route) => route.enabled)
      .map((route) => {
        const provider = providerAccounts.find(
          (acc) => acc.id === route.providerAccountId,
        );
        const upstreamModels = collectRouteUpstreamModels(route, provider);
        const name = shortCodexRouteAccountLabel(
          provider,
          provider?.email,
        );
        return {
          id: route.id,
          namespace: route.namespace,
          providerName: name,
          models: route.selectedModels ?? upstreamModels,
        };
      })
      .filter((group) => group.models.length > 0);
  }, [accounts, routingRoutes, routingEnabled]);
  const {
    message: error,
    scrollKey: errorScrollKey,
    set: setError,
  } = useModalErrorState();

  const busy =
    loading ||
    saving ||
    changingInstance ||
    runningActionId !== null ||
    forceRefreshing ||
    executing !== null;
  const requestClose = useCallback(() => {
    const hasStackedModal = Array.from(
      document.querySelectorAll<HTMLElement>(".modal-overlay"),
    ).some(
      (element) => !element.classList.contains("codex-launch-preview-overlay"),
    );
    if (!hasStackedModal) onClose();
  }, [onClose]);
  useEscClose(
    !busy &&
      !repairOpen &&
      !modelConfigOpen &&
      !contextConfigOpen &&
      !imageModelConfigOpen &&
      !deepSeekAccessModeDialogOpen &&
      !imageGenPickerOpen &&
      !imageGenModeSwitchOpen &&
      !manualRefreshResult,
    requestClose,
  );
  useEscClose(imageModelConfigOpen, () => setImageModelConfigOpen(false));
  useEscClose(deepSeekAccessModeDialogOpen, () =>
    setDeepSeekAccessModeDialogOpen(false),
  );
  useEscClose(imageGenModeSwitchOpen, () => setImageGenModeSwitchOpen(false));

  const applyLoadedConfig = useCallback((config: CodexQuickConfig) => {
    setLoadedConfig(config);
    setCatalogEnabled(config.experimental_model_catalog_enabled);
    setModels(config.experimental_model_catalog_models);
    setDefaultModelId(
      config.experimental_model_catalog_default_model_id ?? null,
    );
    const contextWindow = config.detected_model_context_window;
    const compactLimit = config.detected_auto_compact_token_limit;
    setContextOverrideEnabled(
      contextWindow !== undefined || compactLimit !== undefined,
    );
    setContextWindowInput(contextWindow?.toString() ?? "");
    setCompactLimitInput(compactLimit?.toString() ?? "");
    setModelsError(null);
  }, []);

  useEffect(() => {
    let active = true;
    setLoading(true);
    setLoadedConfig(null);
    setCatalogEnabled(false);
    setModels([]);
    setDefaultModelId(null);
    setContextOverrideEnabled(false);
    setContextWindowInput("");
    setCompactLimitInput("");
    setModelsError(null);
    setNotice(null);
    setManualRefreshResult(null);
    setManualRefreshedAccount(null);
    const routing = selectedInstance?.modelRouting;
    const loadedRoutes = normalizeCodexModelRoutingRoutes(
      routing?.routes ?? [],
      accounts,
    );
    setRoutingEnabled(Boolean(routing?.enabled));
    setRoutingRoutes(loadedRoutes);
    void getCodexInstanceQuickConfig(instanceId)
      .then((config) => {
        if (active) {
          applyLoadedConfig(config);
          if (routing?.enabled && loadedRoutes.length) {
            setModels(
              syncExperimentalModelsWithRouting(
                config.experimental_model_catalog_models,
                loadedRoutes,
                accounts,
                true,
              ),
            );
          }
        }
      })
      .catch((loadError) => {
        if (!active) return;
        setError(
          t("instances.form.codexQuickConfig.loadFailed", {
            defaultValue: "加载当前 Codex 配置失败：{{error}}",
            error: String(loadError).replace(/^Error:\s*/, ""),
          }),
        );
      })
      .finally(() => {
        if (active) setLoading(false);
      });
    return () => {
      active = false;
    };
  }, [account?.id, applyLoadedConfig, instanceId, selectedInstance, setError, t]);

  const normalizedRoutingRoutes = useMemo(
    () => normalizeCodexModelRoutingRoutes(routingRoutes, accounts),
    [accounts, routingRoutes],
  );
  const effectiveLaunchAccount = account ?? currentAccount;
  const mixedRoutingOAuthAccount = useMemo(() => {
    if (!routingEnabled) return null;
    if (effectiveLaunchAccount && isStandardCodexOAuthAccount(effectiveLaunchAccount)) {
      return effectiveLaunchAccount;
    }
    return accounts.find((item) => isStandardCodexOAuthAccount(item)) ?? null;
  }, [accounts, effectiveLaunchAccount, routingEnabled]);
  const mixedRoutingBindAccountId =
    routingEnabled && mixedRoutingOAuthAccount?.id !== effectiveLaunchAccount?.id
      ? mixedRoutingOAuthAccount?.id
      : undefined;
  const routingBindingNeedsRepair =
    routingEnabled && mixedRoutingOAuthAccount == null;
  const contextOverridePreset = resolveCodexContextOverridePreset(
    contextOverrideEnabled,
    contextWindowInput,
    compactLimitInput,
  );

  const nextModelRouting = useMemo(
    () => buildCodexModelRoutingValue(routingEnabled, normalizedRoutingRoutes),
    [normalizedRoutingRoutes, routingEnabled],
  );
  const routingDirty = useMemo(
    () =>
      !areCodexModelRoutingsEqual(selectedInstance?.modelRouting, nextModelRouting) ||
      routingBindingNeedsRepair ||
      mixedRoutingBindAccountId !== undefined,
    [
      mixedRoutingBindAccountId,
      nextModelRouting,
      routingBindingNeedsRepair,
      selectedInstance?.modelRouting,
    ],
  );
  const dirty = useMemo(() => {
    if (!loadedConfig && !routingDirty) return false;
    const contextDirty =
      loadedConfig != null &&
      (contextOverrideEnabled !==
        (loadedConfig.detected_model_context_window !== undefined ||
          loadedConfig.detected_auto_compact_token_limit !== undefined) ||
        (contextOverrideEnabled &&
          (contextWindowInput !==
            (loadedConfig.detected_model_context_window?.toString() ?? "") ||
            compactLimitInput !==
              (loadedConfig.detected_auto_compact_token_limit?.toString() ?? ""))));
    return (
      contextDirty ||
      routingDirty ||
      (loadedConfig != null &&
        (loadedConfig.experimental_model_catalog_enabled !== catalogEnabled ||
          JSON.stringify(loadedConfig.experimental_model_catalog_models) !==
            JSON.stringify(models) ||
          (loadedConfig.experimental_model_catalog_default_model_id ?? null) !==
            defaultModelId))
    );
  }, [
    catalogEnabled,
    compactLimitInput,
    contextOverrideEnabled,
    contextWindowInput,
    defaultModelId,
    loadedConfig,
    models,
    routingDirty,
  ]);

  const persistDraft = useCallback(async () => {
    if (!loadedConfig || (catalogEnabled && modelsError && models.length > 0 && !routingEnabled)) {
      if (catalogEnabled && modelsError) setError(modelsError);
      return false;
    }
    if (!dirty) return true;
    const contextWindow = Number.parseInt(contextWindowInput, 10);
    const compactLimit = compactLimitInput.trim()
      ? Number.parseInt(compactLimitInput, 10)
      : undefined;
    if (
      contextOverrideEnabled &&
      (!Number.isSafeInteger(contextWindow) || contextWindow <= 0)
    ) {
      setError(t("codex.experimentalModelCatalog.models.validation.contextWindow"));
      return false;
    }
    if (
      contextOverrideEnabled &&
      compactLimit !== undefined &&
      (!Number.isSafeInteger(compactLimit) || compactLimit <= 0)
    ) {
      setError(t("codex.experimentalModelCatalog.models.validation.autoCompact"));
      return false;
    }
    if (
      contextOverrideEnabled &&
      compactLimit !== undefined &&
      compactLimit >= contextWindow
    ) {
      setError(t("codex.experimentalModelCatalog.models.validation.autoCompactRange"));
      return false;
    }
    if (routingEnabled) {
      if (routingRoutes.length === 0) {
        setError(
          t(
            "instances.form.modelRouting.routeRequired",
            "请至少添加一个 API 模型路由。",
          ),
        );
        return false;
      }
      const providerAccounts = eligibleCodexModelRoutingAccounts(accounts);
      for (const route of normalizedRoutingRoutes) {
        const namespace = route.namespace.trim().toLowerCase();
        if (
          !/^[a-z0-9][a-z0-9_-]{1,31}$/.test(namespace) ||
          ["official", "subscription", "openai", "codex", "oauth"].includes(
            namespace,
          )
        ) {
          setError(
            t(
              "instances.form.modelRouting.invalidNamespace",
              "命名空间需为 2-32 位小写字母、数字、下划线或连字符，且不能使用保留名称。",
            ),
          );
          return false;
        }
        if (
          !providerAccounts.some(
            (account) => account.id === route.providerAccountId,
          )
        ) {
          setError(
            t(
              "instances.form.modelRouting.providerRequired",
              "每个模型路由都必须选择一个 API 账号。",
            ),
          );
          return false;
        }
      }
    }
    setSaving(true);
    setNotice(null);
    setError(null);
    try {
      let nextModels = models;
      let nextCatalogEnabled = catalogEnabled;
      if (routingEnabled) {
        nextModels = syncExperimentalModelsWithRouting(
          models,
          normalizedRoutingRoutes,
          accounts,
          true,
        );
        nextCatalogEnabled = true;
      }
      const nextCatalog = resolveRoutingCatalog(nextModels, nextCatalogEnabled, defaultModelId);
      const saved = routingDirty
        ? (
            await saveCodexInstanceConfiguration({
              instanceId,
              bindAccountId: mixedRoutingBindAccountId,
              modelRouting: nextModelRouting,
              deferBindAccountApplication: true,
              updateContextOverride: true,
              modelContextWindow: contextOverrideEnabled ? contextWindow : null,
              autoCompactTokenLimit: contextOverrideEnabled ? (compactLimit ?? null) : null,
              experimentalModelCatalogEnabled: nextCatalog.enabled,
              experimentalModelCatalogModels: nextCatalog.models,
              experimentalModelCatalogDefaultModelId: nextCatalog.defaultModelId,
            })
          ).quickConfig
        : await saveCodexInstanceQuickConfig(
            instanceId,
            contextOverrideEnabled ? contextWindow : undefined,
            contextOverrideEnabled ? compactLimit : undefined,
            nextCatalog.enabled,
            nextCatalog.models,
            nextCatalog.defaultModelId,
          );
      applyLoadedConfig(saved);
      setRoutingRoutes(normalizedRoutingRoutes);
      setNotice(routingDirty
        ? t("instances.form.modelRouting.savedForNextLaunch", "已保存，下次通过 Cockpit 启动 Codex 时生效；当前会话保持不变。")
        :
        t(
          "codex.modelProviders.quickConfig.saveSuccess",
          "当前 Codex 配置已保存",
        ),
      );
      return true;
    } catch (saveError) {
      setError(
        t("instances.form.codexQuickConfig.saveFailed", {
          defaultValue: "保存 Codex 配置失败：{{error}}",
          error: String(saveError).replace(/^Error:\s*/, ""),
        }),
      );
      return false;
    } finally {
      setSaving(false);
    }
  }, [
    accounts,
    applyLoadedConfig,
    catalogEnabled,
    compactLimitInput,
    contextOverrideEnabled,
    contextWindowInput,
    defaultModelId,
    dirty,
    loadedConfig,
    models,
    modelsError,
    instanceId,
    nextModelRouting,
    routingDirty,
    routingEnabled,
    mixedRoutingBindAccountId,
    normalizedRoutingRoutes,
    setError,
    t,
  ]);

  useEffect(() => {
    if (!persistRoutingDisableRef.current || routingEnabled) return;
    persistRoutingDisableRef.current = false;
    void persistDraft();
  }, [persistDraft, routingEnabled]);

  const handleExecute = useCallback(
    async (launchAfterSwitch: boolean) => {
      if (busy) return;
      const saved = await persistDraft();
      if (!saved) return;
      setExecuting(launchAfterSwitch ? "launch" : "switch");
      setNotice(null);
      setError(null);
      try {
        const launchOptions: CodexLaunchPreviewLaunchOptions | undefined =
          account && isDeepSeekAccount(account)
            ? {
                deepSeekAccessMode,
                imageGenerationAccountIds: imageGenEnabled
                  ? imageGenAccountIds.filter((id) =>
                      accounts.some((item) => item.id === id),
                    )
                  : [],
              }
            : undefined;
        const started = await onExecute(launchAfterSwitch, launchOptions);
        if (!started) setExecuting(null);
      } catch (executeError) {
        setError(String(executeError).replace(/^Error:\s*/, ""));
        setExecuting(null);
      }
    },
    [
      accounts,
      busy,
      deepSeekAccessMode,
      imageGenAccountIds,
      imageGenEnabled,
      onExecute,
      persistDraft,
      setError,
    ],
  );

  const unavailable =
    loadedConfig &&
    !loadedConfig.experimental_model_catalog_available &&
    !loadedConfig.experimental_model_catalog_enabled;
  const defaultModel = defaultModelId
    ? models.find((model) => model.model_id === defaultModelId)
    : null;
  const defaultModelLabel =
    defaultModel?.display_name ||
    defaultModel?.model_id ||
    t("codex.experimentalModelCatalog.models.followOfficial", "跟随官方");

  const isApiKeySubject = Boolean(account && isCodexApiKeyAccount(account));
  // DeepSeek 走自己的模型目录与 API Key 鉴权：不显示 Token 刷新与官方模型管理。
  const isDeepSeekSubject = Boolean(account && isDeepSeekAccount(account));
  const canChooseDeepSeekAccessMode = Boolean(
    account && isDeepSeekResponsesAccount(account),
  );
  const imageGenAvailable = deepSeekImageGenAvailable;

  const accountPresentation = useMemo(
    () => (account ? buildCodexAccountPresentation(account, t) : null),
    [account, t],
  );
  const accountAuthMetadata = useMemo(
    () => (account && !isApiKeySubject ? getCodexAuthMetadata(account) : null),
    [account, isApiKeySubject],
  );
  const accountSubscription = useMemo(
    () =>
      account && !isApiKeySubject
        ? getCodexSubscriptionPresentationForAccount(account, t)
        : null,
    [account, isApiKeySubject, t],
  );
  const fallbackContextText = useMemo(() => {
    if (!account) return "";
    if (isApiKeySubject) {
      return (
        account.api_provider_name?.trim() ||
        account.api_base_url?.trim() ||
        account.auth_mode?.trim() ||
        "API Key"
      );
    }
    const organizationId = account.organization_id?.trim();
    const workspace =
      accountAuthMetadata?.workspaces.find(
        (item) => organizationId && item.id === organizationId,
      ) ||
      accountAuthMetadata?.workspaces.find((item) => item.is_default) ||
      accountAuthMetadata?.workspaces[0];
    const loginProvider = formatCodexLoginProvider(
      accountAuthMetadata?.authProvider,
    );
    return (
      account.account_name?.trim() ||
      workspace?.title?.trim() ||
      loginProvider ||
      account.auth_mode?.trim() ||
      "Codex"
    );
  }, [account, accountAuthMetadata, isApiKeySubject]);
  const fallbackFacts = useMemo<CodexLaunchPreviewFact[]>(() => {
    if (!account) return [];
    if (isApiKeySubject) {
      const modelCount = account.api_model_catalog?.length ?? 0;
      return [
        {
          label: t("codex.api.provider.label", "供应商"),
          value:
            account.api_provider_name?.trim() ||
            account.api_provider_id?.trim() ||
            t("codex.api.provider.custom", "自定义"),
        },
        {
          label: t("codex.api.baseUrl", "Base URL"),
          value: account.api_base_url?.trim() || "-",
          monospace: true,
          wide: true,
        },
        {
          label: t("codex.api.modelCatalog.label", "模型列表"),
          value:
            modelCount > 0
              ? t("codex.api.modelCatalog.count", {
                  count: modelCount,
                  defaultValue: "{{count}} 个模型",
                })
              : t("common.none", "暂无"),
        },
      ];
    }
    return [
      {
        label: t("kiro.account.userId", "用户 ID"),
        value:
          accountAuthMetadata?.userId?.trim() ||
          account.user_id?.trim() ||
          t("common.none", "暂无"),
        monospace: true,
      },
      {
        label: t("codex.apiSwitchNotice.type.account", "账号"),
        value:
          accountAuthMetadata?.chatgptAccountId?.trim() ||
          account.account_id?.trim() ||
          t("common.none", "暂无"),
        monospace: true,
      },
      {
        label: t("codex.subscription.label", "有效期"),
        value: accountSubscription
          ? `${accountSubscription.valueText}${
              accountSubscription.detailText
                ? ` · ${accountSubscription.detailText}`
                : ""
            }`
          : t("common.none", "暂无"),
      },
    ];
  }, [account, accountAuthMetadata, accountSubscription, isApiKeySubject, t]);
  const tokenExpiryFacts = useMemo<CodexLaunchPreviewFact[]>(() => {
    const tokenAccount = manualRefreshedAccount ?? account;
    if (!tokenAccount || !isStandardCodexOAuthAccount(tokenAccount)) return [];

    const nowSeconds = Math.floor(Date.now() / 1000);
    const locale = i18n.resolvedLanguage || i18n.language;
    const relativeTime = new Intl.RelativeTimeFormat(locale, {
      numeric: "auto",
    });
    const formatExpiry = (expiresAt: number | null) => {
      if (expiresAt === null) {
        return t("codex.switchProgress.detail.expiryUnknown");
      }
      const diffSeconds = expiresAt - nowSeconds;
      const absoluteSeconds = Math.abs(diffSeconds);
      let unit: Intl.RelativeTimeFormatUnit = "minute";
      let divisor = 60;
      if (absoluteSeconds >= 36 * 60 * 60) {
        unit = "day";
        divisor = 24 * 60 * 60;
      } else if (absoluteSeconds >= 90 * 60) {
        unit = "hour";
        divisor = 60 * 60;
      }
      const relativeValue =
        diffSeconds < 0
          ? Math.floor(diffSeconds / divisor)
          : Math.max(1, Math.ceil(diffSeconds / divisor));
      return t("codex.switchProgress.detail.expiresAt", {
        time: new Date(expiresAt * 1000).toLocaleString(locale),
        relative: relativeTime.format(relativeValue, unit),
      });
    };
    const buildFact = (
      label: string,
      token: string | undefined,
      refreshLeadSeconds: number,
    ): CodexLaunchPreviewFact => {
      const expiresAt = getCodexJwtExpiration(token?.trim() || "");
      return {
        label,
        value: formatExpiry(expiresAt),
        tone:
          expiresAt !== null && expiresAt <= nowSeconds
            ? "danger"
            : expiresAt !== null && expiresAt <= nowSeconds + refreshLeadSeconds
              ? "warning"
              : undefined,
      };
    };

    return [
      buildFact("access_token", tokenAccount.tokens?.access_token, 5 * 60),
      buildFact("id_token", tokenAccount.tokens?.id_token, 10 * 60),
    ];
  }, [account, i18n.language, i18n.resolvedLanguage, manualRefreshedAccount, t]);
  const displayFacts = [
    ...(summary?.facts ?? fallbackFacts),
    ...tokenExpiryFacts,
  ];
  const displayQuotaItems =
    summary?.quotaItems ?? accountPresentation?.quotaItems.slice(0, 3) ?? [];
  const displayActions = actions ?? [];
  const displayBadgeLabel =
    summary?.badgeLabel ||
    accountMetaLabel ||
    account?.plan_type ||
    accountPresentation?.planLabel ||
    (mode === "apiService" ? "API Key" : "Codex");
  const displayContextText = summary?.contextText || fallbackContextText;
  const speedAction = displayActions.find((action) => action.id === "speed");
  const imageModelAction = displayActions.find((action) => action.id === "image-model");
  const footerToolActions = displayActions.filter(
    (action) => action.id !== "delete" && action.id !== "speed" && action.id !== "image-model",
  );
  const subjectIcon =
    mode === "apiService" ? (
      <Server size={19} />
    ) : isApiKeySubject ? (
      <KeyRound size={19} />
    ) : (
      <UserRound size={19} />
    );

  const openModelConfig = useCallback(async () => {
    if (busy || unavailable) return;
    if (!catalogEnabled) {
      const confirmed = await confirmDialog(
        t(
          "codex.modelManagement.enableConfirmDescription",
          "开启后，Codex 将以这里配置的模型目录为准。你可以添加、删除和调整模型，但模型列表不会再自动跟随官方变化。",
        ),
        {
          title: t("codex.modelManagement.enableConfirmTitle", "开启模型管理？"),
          okLabel: t("codex.modelManagement.enableConfirmAction", "开启并配置"),
          cancelLabel: t("common.cancel", "取消"),
          kind: "warning",
        },
      );
      if (!confirmed) return;
    }
    setModelConfigSnapshot({
      enabled: catalogEnabled,
      models: models.map((model) => ({
        ...model,
        reasoning_efforts: model.reasoning_efforts
          ? [...model.reasoning_efforts]
          : undefined,
      })),
      defaultModelId,
    });
    setCatalogEnabled(true);
    setNotice(null);
    setError(null);
    setModelConfigOpen(true);
  }, [
    busy,
    catalogEnabled,
    defaultModelId,
    models,
    setError,
    unavailable,
  ]);

  const handleInstanceChange = useCallback(
    async (nextInstanceId: string) => {
      if (
        busy ||
        !onInstanceChange ||
        !nextInstanceId ||
        nextInstanceId === instanceId
      ) {
        return;
      }
      const saved = await persistDraft();
      if (!saved) return;
      setChangingInstance(true);
      setNotice(null);
      setError(null);
      try {
        await onInstanceChange(nextInstanceId);
      } catch (changeError) {
        setError(String(changeError).replace(/^Error:\s*/, ""));
      } finally {
        setChangingInstance(false);
      }
    },
    [busy, instanceId, onInstanceChange, persistDraft, setError],
  );

  const closeModelConfig = useCallback(
    (apply: boolean) => {
      if (apply) {
        setCatalogEnabled(true);
      } else if (modelConfigSnapshot) {
        setCatalogEnabled(modelConfigSnapshot.enabled);
        setModels(modelConfigSnapshot.models);
        setDefaultModelId(modelConfigSnapshot.defaultModelId);
      }
      setModelConfigSnapshot(null);
      setModelConfigOpen(false);
      setModelsError(null);
    },
    [modelConfigSnapshot],
  );

  const openContextConfig = useCallback(() => {
    if (busy) return;
    setContextConfigSnapshot({
      enabled: contextOverrideEnabled,
      contextWindowInput,
      compactLimitInput,
    });
    setNotice(null);
    setError(null);
    setContextConfigOpen(true);
  }, [busy, compactLimitInput, contextOverrideEnabled, contextWindowInput, setError]);

  const closeContextConfig = useCallback(
    (apply: boolean) => {
      if (!apply && contextConfigSnapshot) {
        setContextOverrideEnabled(contextConfigSnapshot.enabled);
        setContextWindowInput(contextConfigSnapshot.contextWindowInput);
        setCompactLimitInput(contextConfigSnapshot.compactLimitInput);
      }
      setContextConfigSnapshot(null);
      setContextConfigOpen(false);
    },
    [contextConfigSnapshot],
  );

  const handleAuxiliaryAction = useCallback(
    async (action: CodexLaunchPreviewAction) => {
      if (busy || action.disabled || !action.onAction) return;
      setRunningActionId(action.id);
      setNotice(null);
      setError(null);
      try {
        await action.onAction();
      } catch (actionError) {
        setError(String(actionError).replace(/^Error:\s*/, ""));
      } finally {
        setRunningActionId(null);
      }
    },
    [busy, setError],
  );

  const handleForceRefresh = useCallback(async () => {
    if (!account || !isStandardCodexOAuthAccount(account) || forceRefreshing) {
      return;
    }
    setForceRefreshing(true);
    setManualRefreshResult({ status: "running" });
    setNotice(null);
    setError(null);
    try {
      const refreshed = await forceRefreshCodexTokens(account.id);
      useCodexAccountStore.getState().applyAccountSnapshot(refreshed);
      setManualRefreshedAccount(refreshed);
      setManualRefreshResult({ status: "success" });
    } catch (refreshError) {
      setManualRefreshResult({
        status: "error",
        error: String(refreshError).replace(/^Error:\s*/, ""),
      });
    } finally {
      setForceRefreshing(false);
    }
  }, [account, forceRefreshing, setError]);

  const closeManualRefreshResult = useCallback(() => {
    if (forceRefreshing) return;
    setManualRefreshResult(null);
  }, [forceRefreshing]);

  const handleManualRefreshReauthorize = useCallback(() => {
    if (!account || forceRefreshing) return;
    setManualRefreshResult(null);
    onClose();
    window.dispatchEvent(
      new CustomEvent("app-request-navigate", { detail: "codex" }),
    );
    requestCodexOpenAddAccount({
      tab: "oauth",
      targetAccountId: account.id,
      ...(mode === "instance"
        ? {
            retryInstanceLaunchAfterOAuth: true,
            retryInstanceId: instanceId,
          }
        : {}),
    });
  }, [account, forceRefreshing, instanceId, mode, onClose]);

  const renderFooterAction = (action: CodexLaunchPreviewAction) => (
    <div
      key={action.id}
      className={`codex-launch-preview-action-item ${
        action.control ? "has-control" : ""
      }`}
      title={action.description}
    >
      {action.control ? (
        <>
          <span className="codex-launch-preview-action-label">
            {action.label}
          </span>
          {action.control}
        </>
      ) : (
        <button
          type="button"
          className={`codex-launch-preview-action-button ${
            action.tone === "danger" ? "is-danger" : ""
          }`}
          onClick={() => void handleAuxiliaryAction(action)}
          disabled={busy || action.disabled || !action.onAction}
        >
          {runningActionId === action.id
            ? t("common.loading", "加载中...")
            : action.label}
        </button>
      )}
    </div>
  );

  const deepSeekAccessModeLabel =
    deepSeekAccessMode === DEEPSEEK_ACCESS_MODE_DIRECT
      ? t("codex.deepSeek.start.directMode", "直连官方")
      : deepSeekAccessMode === DEEPSEEK_ACCESS_MODE_CDP
        ? t("codex.deepSeek.start.cdpMode", "CDP 注入")
        : t("codex.deepSeek.start.gatewayMode", "网关列出");

  const deepSeekAccessModeOptions: {
    id: DeepSeekAccessMode;
    label: string;
    pros: string;
    cons: string;
  }[] = [
    {
      id: DEEPSEEK_ACCESS_MODE_GATEWAY,
      label: t("codex.deepSeek.start.gatewayMode", "网关列出"),
      pros: t(
        "codex.launchPreview.accessModeGatewayPros",
        "优点：支持在 Codex 内切换模型，可绑定 OAuth 的 GPT 账号并转发生图。",
      ),
      cons: t(
        "codex.launchPreview.accessModeGatewayCons",
        "缺点：需要实例本地网关，速度略低于直连。",
      ),
    },
    {
      id: DEEPSEEK_ACCESS_MODE_CDP,
      label: t("codex.deepSeek.start.cdpMode", "CDP 注入"),
      pros: t(
        "codex.launchPreview.accessModeCdpPros",
        "优点：速度与直连一致，可在官方模型列表切换 Flash / Pro。",
      ),
      cons: t(
        "codex.launchPreview.accessModeCdpCons",
        "缺点：需要注入官方客户端，不支持绑定 OAuth 与生图转发。",
      ),
    },
    {
      id: DEEPSEEK_ACCESS_MODE_DIRECT,
      label: t("codex.deepSeek.start.directMode", "直连官方"),
      pros: t(
        "codex.launchPreview.accessModeDirectPros",
        "优点：不经网关，速度最快。",
      ),
      cons: t(
        "codex.launchPreview.accessModeDirectCons",
        "缺点：不能在 Codex 内切换模型，也没有 OAuth 与生图转发。",
      ),
    },
  ];

  const selectedImageGenAccounts = imageGenAccountIds
    .map((id) => accounts.find((item) => item.id === id))
    .filter((item): item is CodexAccount => Boolean(item));
  // 保存时以列表里勾选的现存账号为准：账号总览里删掉的账号不再参与提交，
  // 否则后端会逐个校验并报「生图账号不存在」。
  const existingImageGenAccountIds = selectedImageGenAccounts.map(
    (item) => item.id,
  );

  return (
    <>
      <div className="modal-overlay codex-launch-preview-overlay">
        <div className="modal codex-launch-preview-modal">
          <div className="modal-header">
            <div className="codex-launch-preview-title-icon">
              <Play size={18} />
            </div>
            <div className="codex-launch-preview-heading">
              <h2>{t("codex.launchPreview.title")}</h2>
            </div>
            <button
              type="button"
              className="modal-close"
              onClick={requestClose}
              disabled={busy}
              aria-label={t("common.close", "关闭")}
            >
              <X />
            </button>
          </div>

          <div className="modal-body">
            <ModalErrorMessage message={error} scrollKey={errorScrollKey} />

            <section className="codex-launch-preview-summary-card">
              <div className="codex-launch-preview-summary-head">
                <div className="codex-launch-preview-subject">
                  <div className="codex-launch-preview-subject-icon">
                    {subjectIcon}
                  </div>
                  <div className="codex-launch-preview-subject-copy">
                    <div className="codex-launch-preview-subject-title-row">
                      <strong title={accountLabel}>{accountLabel}</strong>
                      <span className="codex-launch-preview-plan-badge">
                        {displayBadgeLabel}
                      </span>
                      {summary?.statusLabel && (
                        <span
                          className={`codex-launch-preview-status-badge ${
                            summary.statusTone ?? "neutral"
                          }`}
                        >
                          {summary.statusLabel}
                        </span>
                      )}
                    </div>
                    {displayContextText && (
                      <span title={displayContextText}>
                        {displayContextText}
                      </span>
                    )}
                  </div>
                </div>
                <div className="codex-launch-preview-summary-controls">
                  <div
                    className={`codex-launch-preview-target${
                      instanceOptions?.length && onInstanceChange
                        ? " is-switchable"
                        : ""
                    }`}
                  >
                    <span>
                      {t(
                        "codex.sessionManager.repairModal.targetInstance",
                        "目标实例",
                      )}
                    </span>
                    {instanceOptions?.length && onInstanceChange ? (
                      <SingleSelectDropdown
                        value={instanceId}
                        options={instanceOptions}
                        onChange={(nextInstanceId) =>
                          void handleInstanceChange(nextInstanceId)
                        }
                        className="codex-launch-preview-instance-select"
                        menuClassName="codex-launch-preview-instance-menu"
                        disabled={busy}
                        ariaLabel={t(
                          "codex.sessionManager.repairModal.targetInstance",
                          "目标实例",
                        )}
                        menuWidth={260}
                      />
                    ) : (
                      <>
                        <strong>
                          {instanceLabel ||
                            t("instances.defaultName", "默认实例")}
                        </strong>
                        <ArrowRight size={16} />
                      </>
                    )}
                  </div>
                  {speedAction?.control && (
                    <div
                      className="codex-launch-preview-header-speed"
                      title={speedAction.description}
                    >
                      <span>{speedAction.label}</span>
                      {speedAction.control}
                    </div>
                  )}
                </div>
              </div>

              {displayFacts.length > 0 && (
                <div className="codex-launch-preview-facts">
                  {displayFacts.map((fact, index) => (
                    <div
                      key={`${fact.label}-${index}`}
                      className={[
                        fact.wide ? "is-wide" : "",
                        fact.tone ? `is-${fact.tone}` : "",
                      ]
                        .filter(Boolean)
                        .join(" ")}
                    >
                      <span>{fact.label}</span>
                      <strong
                        className={fact.monospace ? "is-monospace" : undefined}
                        title={fact.value}
                      >
                        {fact.value}
                      </strong>
                    </div>
                  ))}
                </div>
              )}

              {summary?.usage &&
                (summary.usage.requests ||
                  summary.usage.tokens ||
                  summary.usage.cost ||
                  summary.usage.extraValue) && (
                  <div className="codex-launch-preview-usage">
                    <div className="codex-launch-preview-usage-title">
                      <BarChart3 size={15} />
                      <span>{summary.usage.label}</span>
                    </div>
                    <div className="codex-launch-preview-usage-grid">
                      {summary.usage.requests && (
                        <div>
                          <span>
                            {t("codex.localAccess.stats.requests", "总请求数")}
                          </span>
                          <strong>{summary.usage.requests}</strong>
                        </div>
                      )}
                      {summary.usage.tokens && (
                        <div>
                          <span>
                            {t("codex.localAccess.stats.tokens", "总 Token 数")}
                          </span>
                          <strong>{summary.usage.tokens}</strong>
                        </div>
                      )}
                      {(summary.usage.cost || summary.usage.extraValue) && (
                        <div>
                          {summary.usage.cost && (
                            <>
                              <span>
                                {t(
                                  "codex.localAccess.stats.estimatedCost",
                                  "估算价值",
                                )}
                              </span>
                              <strong>{summary.usage.cost}</strong>
                            </>
                          )}
                          {summary.usage.extraValue && (
                            <div className="codex-launch-preview-usage-extra">
                              <span>{summary.usage.extraLabel}</span>
                              <strong>{summary.usage.extraValue}</strong>
                            </div>
                          )}
                        </div>
                      )}
                    </div>
                  </div>
                )}

              {displayQuotaItems.length > 0 && (
                <div className="codex-launch-preview-quota">
                  <CodexQuotaMiniRows items={displayQuotaItems} t={t} />
                </div>
              )}

              {summary?.tags && summary.tags.length > 0 && (
                <div className="codex-launch-preview-tags">
                  {summary.tags.slice(0, 8).map((tag) => (
                    <span key={tag}>{tag}</span>
                  ))}
                  {summary.tags.length > 8 && (
                    <span>+{summary.tags.length - 8}</span>
                  )}
                </div>
              )}
            </section>

            <div className="codex-launch-preview-tool-list">
              {isDeepSeekSubject && (
                <section className="codex-launch-preview-tool-row">
                  <div className="codex-launch-preview-tool-icon">
                    <Route size={16} />
                  </div>
                  <div className="codex-launch-preview-tool-copy">
                    <h3>{t("codex.launchPreview.accessModeTitle", "启动方式")}</h3>
                    <p>
                      {t(
                        "codex.launchPreview.accessModeDescription",
                        "选择 DeepSeek 的启动方式；不同方式在模型切换、OAuth 与生图转发上不同。",
                      )}
                    </p>
                    {!canChooseDeepSeekAccessMode && (
                      <div className="codex-launch-preview-tool-meta">
                        <span>
                          {t(
                            "codex.deepSeek.start.chatOnlyHint",
                            "Chat Completions 只能走本地网关，并在此选择启动模型。",
                          )}
                        </span>
                      </div>
                    )}
                  </div>
                  <button
                    type="button"
                    className="btn btn-outline btn-sm codex-launch-preview-tool-action codex-launch-preview-dropdown-trigger"
                    onClick={() => setDeepSeekAccessModeDialogOpen(true)}
                    disabled={busy || !canChooseDeepSeekAccessMode}
                  >
                    <span>{deepSeekAccessModeLabel}</span>
                    <ChevronDown size={14} />
                  </button>
                </section>
              )}

              {isDeepSeekSubject && (
                <section className="codex-launch-preview-tool-row">
                  <div className="codex-launch-preview-tool-icon">
                    <ImagePlus size={16} />
                  </div>
                  <div className="codex-launch-preview-tool-copy">
                    <h3>
                      {t("codex.launchPreview.imageGenTitle", "启用 GPT 生图")}
                    </h3>
                    <p>
                      {t(
                        "codex.deepSeek.start.imageGenHint",
                        "对话仍由该供应商的模型处理；生图走 gpt-image 原链路，由所选 GPT 账号执行并消耗其额度。",
                      )}
                    </p>
                    <div className="codex-launch-preview-tool-meta">
                      {imageGenEnabled && (
                        <span className="is-enabled">
                          {selectedImageGenAccounts.length > 0
                            ? t("codex.deepSeek.start.imageGenSelected", {
                                count: selectedImageGenAccounts.length,
                                defaultValue: "已选 {{count}} 个账号",
                              })
                            : t(
                                "codex.deepSeek.start.imageGenEmpty",
                                "尚未选择账号",
                              )}
                        </span>
                      )}
                      {imageGenEnabled &&
                        selectedImageGenAccounts.slice(0, 4).map((item) => (
                          <span key={item.id}>
                            {item.email || item.account_name || item.id}
                          </span>
                        ))}
                    </div>
                  </div>
                  <div className="codex-launch-preview-tool-controls">
                    <label className="codex-launch-preview-checkbox">
                      <input
                        type="checkbox"
                        checked={imageGenEnabled}
                        disabled={busy}
                        onChange={(event) => {
                          if (!event.target.checked) {
                            setImageGenEnabled(false);
                            setImageGenAccountIds([]);
                            return;
                          }
                          if (!imageGenAvailable) {
                            setImageGenModeSwitchOpen(true);
                            return;
                          }
                          setImageGenEnabled(true);
                          setImageGenPickerOpen(true);
                        }}
                      />
                      <span>{t("common.enable", "启用")}</span>
                    </label>
                    <button
                      type="button"
                      className="btn btn-outline btn-sm codex-launch-preview-tool-action"
                      disabled={busy || !imageGenEnabled}
                      onClick={() => setImageGenPickerOpen(true)}
                    >
                      {t("codex.deepSeek.start.imageGenPick", "选择 GPT 账号")}
                    </button>
                  </div>
                </section>
              )}

              {mode !== "apiService" &&
                account &&
                isStandardCodexOAuthAccount(account) &&
                (selectedInstance?.launchMode ?? "app") !== "cli" && (
                  <CodexModelRoutingFields
                    variant="row"
                    enabled={routingEnabled}
                    routes={routingRoutes}
                    accounts={accounts}
                    running={Boolean(selectedInstance?.running)}
                    onEnabledChange={(nextEnabled) => {
                      const catalog = resolveRoutingCatalog(
                        syncExperimentalModelsWithRouting(models, routingRoutes, accounts, nextEnabled),
                        nextEnabled || catalogEnabled,
                        defaultModelId,
                      );
                      setCatalogEnabled(catalog.enabled);
                      setDefaultModelId(catalog.defaultModelId);
                      if (!nextEnabled) {
                        persistRoutingDisableRef.current = true;
                      }
                      setRoutingEnabled(nextEnabled);
                      setModels((prevModels) =>
                        syncExperimentalModelsWithRouting(
                          prevModels,
                          routingRoutes,
                          accounts,
                          nextEnabled,
                        ),
                      );
                    }}
                    onRoutesChange={(nextRoutes) => {
                      setRoutingRoutes(nextRoutes);
                      setModels((prevModels) =>
                        syncExperimentalModelsWithRouting(
                          prevModels,
                          nextRoutes,
                          accounts,
                          routingEnabled,
                        ),
                      );
                    }}
                    onAccountsRefresh={fetchAccounts}
                  />
                )}
              {imageModelAction?.control && (
                <section className="codex-launch-preview-tool-row">
                  <div className="codex-launch-preview-tool-icon">
                    <SlidersHorizontal size={16} />
                  </div>
                  <div className="codex-launch-preview-tool-copy">
                    <h3>{imageModelAction.label}</h3>
                    <p>{t("codex.localAccess.imageGenerationModel.description")}</p>
                    <div className="codex-launch-preview-tool-meta">
                      <span>{imageModelAction.description}</span>
                    </div>
                  </div>
                  <button
                    type="button"
                    className="btn btn-outline btn-sm codex-launch-preview-tool-action"
                    disabled={busy}
                    onClick={() => setImageModelConfigOpen(true)}
                  >
                    {t("nav.settings")}
                  </button>
                </section>
              )}
              {oauthBinding && account && isCodexApiKeyAccount(account) && (
                <section className="codex-launch-preview-tool-row">
                  <div className="codex-launch-preview-tool-icon">
                    <Link2 size={16} />
                  </div>
                  <div className="codex-launch-preview-tool-copy">
                    <h3>{t("codex.api.oauthBinding.label", "OAuth 绑定")}</h3>
                    <p>
                      {t(
                        "codex.launchPreview.oauthBindingHint",
                        "绑定后启动与普通账号没有任何差异，可使用 OAuth 的全部能力（远端压缩，浏览器操作，各类插件等）；对话仍由当前 API Key 供应商处理。",
                      )}
                    </p>
                    <div className="codex-launch-preview-tool-meta">
                      <span>
                        {oauthBinding.boundAccountLabel ||
                          t("codex.api.oauthBinding.unbound", "未绑定")}
                      </span>
                      {oauthBinding.needsReauth && (
                        <span
                          className="codex-status-pill quota-error"
                          title={oauthBinding.reauthDescription || undefined}
                        >
                          <CircleAlert size={12} />
                          {t("codex.authError.badge", "授权异常")}
                        </span>
                      )}
                    </div>
                  </div>
                  <div className="codex-launch-preview-tool-controls">
                    {oauthBinding.needsReauth && onReauthorizeOAuth && (
                      <button
                        type="button"
                        className="btn btn-outline btn-sm codex-launch-preview-tool-action"
                        onClick={onReauthorizeOAuth}
                        disabled={busy}
                      >
                        {t("common.reauthorize", "重新授权")}
                      </button>
                    )}
                    {onBindOAuth && (
                      <button
                        type="button"
                        className="btn btn-outline btn-sm codex-launch-preview-tool-action"
                        onClick={onBindOAuth}
                        disabled={busy}
                      >
                        {oauthBinding.boundAccountLabel
                          ? t(
                              "codex.launchPreview.oauthBindingChange",
                              "更换",
                            )
                          : t("codex.api.oauthBinding.action", "绑定 OAuth")}
                      </button>
                    )}
                  </div>
                </section>
              )}
              {!isDeepSeekSubject && (
                <section className="codex-launch-preview-tool-row">
                  <div className="codex-launch-preview-tool-icon">
                    <RefreshCw size={16} />
                  </div>
                  <div className="codex-launch-preview-tool-copy">
                    <h3>{t("codex.launchPreview.forceRefreshTitle")}</h3>
                    <p>{t("codex.launchPreview.forceRefreshDescription")}</p>
                    <div className="codex-launch-preview-tool-meta">
                      <span>
                        {account && isStandardCodexOAuthAccount(account)
                          ? t("codex.launchPreview.forceRefreshReady")
                          : t("codex.launchPreview.forceRefreshUnavailable")}
                      </span>
                    </div>
                  </div>
                  <button
                    type="button"
                    className="btn btn-outline btn-sm codex-launch-preview-tool-action"
                    onClick={() => void handleForceRefresh()}
                    disabled={
                      busy || !account || !isStandardCodexOAuthAccount(account)
                    }
                  >
                    {forceRefreshing
                      ? t("codex.launchPreview.forceRefreshRunning")
                      : t("codex.launchPreview.forceRefreshAction")}
                  </button>
                </section>
              )}
              <section className="codex-launch-preview-tool-row">
                <div className="codex-launch-preview-tool-icon">
                  <SlidersHorizontal size={16} />
                </div>
                <div className="codex-launch-preview-tool-copy">
                  <h3>
                    {t("codex.contextOverride.title", "上下文管理")}
                  </h3>
                  <p>
                    {t(
                      "codex.contextOverride.dialogDescription",
                      "对当前 Codex 实例的所有账号生效，切号后保持不变；可选择跟随官方、预设或自定义。",
                    )}
                  </p>
                  <div className="codex-launch-preview-tool-meta">
                    <span className={contextOverrideEnabled ? "is-enabled" : ""}>
                      {loading
                        ? t("common.loading", "加载中...")
                        : contextOverridePreset === "preset_516k"
                          ? "516K / 460K"
                          : contextOverridePreset === "preset_1m"
                            ? "1M / 900K"
                            : contextOverridePreset === "custom"
                              ? t("codex.contextOverride.custom", "自定义上下文")
                              : t("codex.contextOverride.followOfficial", "跟随官方")}
                    </span>
                    {contextOverridePreset === "custom" && contextWindowInput && (
                      <span>
                        {t(
                          "codex.experimentalModelCatalog.models.contextWindow",
                          "上下文窗口",
                        )}：{contextWindowInput}
                      </span>
                    )}
                  </div>
                </div>
                <button
                  type="button"
                  className="btn btn-outline btn-sm codex-launch-preview-tool-action"
                  onClick={openContextConfig}
                  disabled={busy}
                >
                  {contextOverrideEnabled
                    ? t("codex.contextOverride.manage", "管理上下文")
                    : t("codex.contextOverride.adjust", "调整上下文")}
                </button>
              </section>
              {!isDeepSeekSubject && (
                <section className="codex-launch-preview-tool-row">
                  <div className="codex-launch-preview-tool-icon">
                    <SlidersHorizontal size={16} />
                  </div>
                  <div className="codex-launch-preview-tool-copy">
                    <h3>{t("codex.modelManagement.title", "模型管理")}</h3>
                    <p>
                      {catalogEnabled
                        ? t(
                            "codex.modelManagement.enabledDescription",
                            "当前以配置模型目录为准，可添加或减少模型，不再自动跟随官方目录。",
                          )
                        : t(
                            "codex.modelManagement.disabledDescription",
                            "默认关闭；关闭时模型列表、顺序、默认模型和推理强度均跟随官方。",
                          )}
                    </p>
                    <div className="codex-launch-preview-tool-meta">
                      <span className={catalogEnabled ? "is-enabled" : ""}>
                        {loading
                          ? t("common.loading", "加载中...")
                          : catalogEnabled
                            ? t("codex.modelManagement.enabled", "已开启")
                            : t("codex.modelManagement.disabled", "跟随官方")}
                      </span>
                      {catalogEnabled && (
                        <span>
                          {t("codex.launchPreview.defaultModel", "默认模型")}：
                          {defaultModelLabel}
                        </span>
                      )}
                      {catalogEnabled && models.length > 0 && (
                        <span>
                          {t("codex.api.modelCatalog.count", {
                            count: models.length,
                            defaultValue: "{{count}} 个模型",
                          })}
                        </span>
                      )}
                    </div>
                  </div>
                  <button
                    type="button"
                    className="btn btn-outline btn-sm codex-launch-preview-tool-action"
                    onClick={() => void openModelConfig()}
                    disabled={busy || Boolean(unavailable)}
                  >
                    {catalogEnabled
                      ? t("codex.modelManagement.manage", "管理模型")
                      : t("codex.modelManagement.enable", "开启模型管理")}
                  </button>
                </section>
              )}

              <section className="codex-launch-preview-tool-row">
                <div className="codex-launch-preview-tool-icon">
                  <Wrench size={16} />
                </div>
                <div className="codex-launch-preview-tool-copy">
                  <h3>
                    {t(
                      "codex.sessionManager.actions.repairVisibility",
                      "修复可见性",
                    )}
                  </h3>
                  <p>
                    {t(
                      "codex.sessionManager.repairModal.modeQuickDesc",
                      "只校正官方 state DB 和会话文件首条元数据，适合日常切号后恢复。",
                    )}
                  </p>
                  <div className="codex-launch-preview-tool-meta">
                    <span>
                      {t(
                        "codex.sessionManager.repairModal.modeQuick",
                        "快速修复",
                      )}
                    </span>
                    <span>{t("codex.launchPreview.notApplied")}</span>
                  </div>
                </div>
                <button
                  type="button"
                  className="btn btn-outline btn-sm codex-launch-preview-tool-action"
                  onClick={() => setRepairOpen(true)}
                  disabled={busy}
                >
                  {t(
                    "codex.sessionManager.actions.repairVisibility",
                    "修复可见性",
                  )}
                </button>
              </section>
            </div>

            {notice && (
              <div className="codex-launch-preview-notice">
                <Save size={14} />
                <span>{notice}</span>
              </div>
            )}
            {dirty && !notice && (
              <div className="codex-launch-preview-dirty">
                <CircleAlert size={14} />
                <span>{t("codex.launchPreview.unsavedConfig")}</span>
              </div>
            )}
          </div>

          <div className="modal-footer codex-launch-preview-footer">
            {footerToolActions.length > 0 && (
              <div className="codex-launch-preview-footer-tools">
                {footerToolActions.map(renderFooterAction)}
              </div>
            )}
            <div className="codex-launch-preview-footer-main">
              <div className="codex-launch-preview-footer-start">
                {summary?.footerText && (
                  <span className="codex-launch-preview-action-meta">
                    {summary.footerText}
                  </span>
                )}
                <button
                  type="button"
                  className="btn btn-secondary"
                  onClick={requestClose}
                  disabled={busy}
                >
                  {t("common.close", "关闭")}
                </button>
              </div>
              <div className="codex-launch-preview-footer-primary">
                {dirty && (
                  <button
                    type="button"
                    className="btn btn-outline"
                    onClick={() => void persistDraft()}
                    disabled={busy || (catalogEnabled && Boolean(modelsError))}
                  >
                    <Save size={15} />
                    {saving
                      ? t("common.saving", "保存中...")
                      : t("common.save", "保存")}
                  </button>
                )}
                {mode === "account" && (
                  <button
                    type="button"
                    className="btn btn-outline"
                    onClick={() => void handleExecute(false)}
                    disabled={busy || (catalogEnabled && Boolean(modelsError))}
                  >
                    {executing === "switch"
                      ? t("common.loading", "加载中...")
                      : t("codex.switch", "切换")}
                  </button>
                )}
                <button
                  type="button"
                  className="btn btn-primary"
                  onClick={() => void handleExecute(mode !== "instance")}
                  disabled={busy || (catalogEnabled && models.length > 0 && Boolean(modelsError))}
                >
                  {mode !== "instance" && <Play size={15} />}
                  {executing !== null
                    ? t("common.loading", "加载中...")
                    : mode === "account"
                      ? t("codex.launchPreview.switchAndStart")
                      : mode === "apiService"
                        ? t("codex.localAccess.activateAction", "启动 API 服务")
                        : t("codex.launchPreview.startInstance")}
                </button>
              </div>
            </div>
          </div>
        </div>
      </div>

      {deepSeekAccessModeDialogOpen && (
        <div className="modal-overlay codex-launch-preview-refresh-overlay">
          <div
            className="modal codex-launch-preview-mode-modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="codex-launch-preview-mode-title"
          >
            <div className="modal-header">
              <h2 id="codex-launch-preview-mode-title">
                {t("codex.launchPreview.accessModeTitle", "启动方式")}
              </h2>
              <button
                type="button"
                className="modal-close"
                onClick={() => setDeepSeekAccessModeDialogOpen(false)}
                aria-label={t("common.close", "关闭")}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              <p className="form-hint">
                {t(
                  "codex.launchPreview.accessModeDescription",
                  "选择 DeepSeek 的启动方式；不同方式在模型切换、OAuth 与生图转发上不同。",
                )}
              </p>
              <div className="codex-launch-preview-mode-list">
                {deepSeekAccessModeOptions.map((option) => (
                  <button
                    key={option.id}
                    type="button"
                    className={`codex-launch-preview-mode-card ${
                      deepSeekAccessMode === option.id ? "active" : ""
                    }`}
                    onClick={() => {
                      setDeepSeekAccessMode(option.id);
                      setDeepSeekAccessModeDialogOpen(false);
                    }}
                  >
                    <span className="codex-launch-preview-mode-card-head">
                      <span className="codex-launch-preview-mode-card-title">
                        {option.label}
                      </span>
                      {deepSeekAccessMode === option.id && <Check size={15} />}
                    </span>
                    <span className="codex-launch-preview-mode-card-section">
                      <span className="codex-launch-preview-mode-card-label is-pros">
                        {t("codex.launchPreview.prosLabel", "优点：")}
                      </span>
                      <span className="codex-launch-preview-mode-card-pros">
                        {option.pros}
                      </span>
                    </span>
                    <span className="codex-launch-preview-mode-card-section">
                      <span className="codex-launch-preview-mode-card-label is-cons">
                        {t("codex.launchPreview.consLabel", "缺点：")}
                      </span>
                      <span className="codex-launch-preview-mode-card-cons">
                        {option.cons}
                      </span>
                    </span>
                  </button>
                ))}
              </div>
            </div>
          </div>
        </div>
      )}

      {imageGenModeSwitchOpen && (
        <div className="modal-overlay codex-launch-preview-refresh-overlay">
          <div
            className="modal codex-launch-preview-refresh-modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="codex-launch-preview-imagegen-gateway-title"
          >
            <div className="modal-header">
              <h2 id="codex-launch-preview-imagegen-gateway-title">
                {t(
                  "codex.launchPreview.imageGenSwitchTitle",
                  "仅网关模式支持",
                )}
              </h2>
              <button
                type="button"
                className="modal-close"
                onClick={() => setImageGenModeSwitchOpen(false)}
                aria-label={t("common.close", "关闭")}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              <p className="form-hint">
                {t(
                  "codex.launchPreview.imageGenSwitchDescription",
                  "启用 GPT 生图需要先切换到「网关列出」模式，是否切换并勾选？",
                )}
              </p>
            </div>
            <div className="modal-footer">
              <button
                type="button"
                className="btn btn-secondary"
                onClick={() => setImageGenModeSwitchOpen(false)}
              >
                {t("codex.launchPreview.imageGenSwitchNo", "否")}
              </button>
              <button
                type="button"
                className="btn btn-primary"
                onClick={() => {
                  setDeepSeekAccessMode(DEEPSEEK_ACCESS_MODE_GATEWAY);
                  setImageGenEnabled(true);
                  setImageGenModeSwitchOpen(false);
                  setImageGenPickerOpen(true);
                }}
              >
                {t("codex.launchPreview.imageGenSwitchYes", "是")}
              </button>
            </div>
          </div>
        </div>
      )}

      {imageGenPickerOpen && (
        <CodexImageAccountPickerModal
          accounts={accounts}
          selectedIds={existingImageGenAccountIds}
          contextLabel={
            accountPresentation?.displayName ||
            account?.email ||
            account?.id ||
            ""
          }
          onCancel={() => {
            setImageGenPickerOpen(false);
            if (existingImageGenAccountIds.length === 0) {
              setImageGenEnabled(false);
            }
          }}
          onConfirm={(ids) => {
            setImageGenPickerOpen(false);
            setImageGenAccountIds(ids);
            setImageGenEnabled(ids.length > 0);
          }}
        />
      )}

      {imageModelConfigOpen && imageModelAction?.control && (
        <div className="modal-overlay codex-launch-preview-refresh-overlay">
          <div
            className="modal codex-launch-preview-refresh-modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="codex-image-model-config-title"
          >
            <div className="modal-header">
              <h2 id="codex-image-model-config-title">{imageModelAction.label}</h2>
              <button
                type="button"
                className="modal-close"
                onClick={() => setImageModelConfigOpen(false)}
                aria-label={t("common.close")}
              >
                <X />
              </button>
            </div>
            <div className="modal-body codex-launch-preview-image-model">
              {imageModelAction.control}
            </div>
            <div className="modal-footer">
              <button type="button" className="btn btn-secondary" onClick={() => setImageModelConfigOpen(false)}>
                {t("common.close")}
              </button>
            </div>
          </div>
        </div>
      )}

      {manualRefreshResult && (
        <div className="modal-overlay codex-launch-preview-refresh-overlay">
          <div
            className="modal codex-launch-preview-refresh-modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="codex-launch-preview-refresh-title"
          >
            <div className="modal-header">
              <div className="codex-launch-preview-refresh-heading">
                <RefreshCw size={18} />
                <div>
                  <h2 id="codex-launch-preview-refresh-title">
                    {t("codex.launchPreview.forceRefreshTitle")}
                  </h2>
                  <p>
                    {manualRefreshResult.status === "running"
                      ? t("codex.launchPreview.forceRefreshRunning")
                      : manualRefreshResult.status === "success"
                        ? t("codex.launchPreview.forceRefreshSuccess")
                        : t("codex.launchPreview.forceRefreshDescription")}
                  </p>
                </div>
              </div>
              <button
                type="button"
                className="modal-close"
                onClick={closeManualRefreshResult}
                disabled={forceRefreshing}
                aria-label={t("common.close", "关闭")}
              >
                <X />
              </button>
            </div>
            <div className="modal-body codex-launch-preview-refresh-body">
              {manualRefreshResult.status === "error" ? (
                <ModalErrorMessage message={manualRefreshResult.error} />
              ) : (
                <div className="codex-launch-preview-refresh-status success">
                  <Save size={16} />
                  <span>
                    {manualRefreshResult.status === "running"
                      ? t("codex.launchPreview.forceRefreshRunning")
                      : t("codex.launchPreview.forceRefreshSuccess")}
                  </span>
                </div>
              )}
            </div>
            <div className="modal-footer codex-launch-preview-refresh-footer">
              <button
                type="button"
                className="btn btn-secondary"
                onClick={closeManualRefreshResult}
                disabled={forceRefreshing}
              >
                {t("common.cancel", "取消")}
              </button>
              <button
                type="button"
                className="btn btn-outline"
                onClick={() => void handleForceRefresh()}
                disabled={forceRefreshing}
              >
                {forceRefreshing
                  ? t("codex.launchPreview.forceRefreshRunning")
                  : t("codex.launchPreview.forceRefreshRetry", "重新检测")}
              </button>
              {manualRefreshResult.status === "error" && (
                <button
                  type="button"
                  className="btn btn-primary"
                  onClick={() => void handleManualRefreshReauthorize()}
                >
                  {t("common.reauthorize", "重新授权")}
                </button>
              )}
            </div>
          </div>
        </div>
      )}

      <CodexSessionVisibilityRepairModal
        open={repairOpen}
        onClose={() => setRepairOpen(false)}
        onRepaired={() => {
          setNotice(t("codex.launchPreview.repairCompleted"));
        }}
      />

      {modelConfigOpen && (
        <div className="modal-overlay codex-launch-preview-model-config-overlay">
          <div className="modal codex-launch-preview-model-config-modal">
            <div className="modal-header">
              <div>
                <h2>{t("codex.modelManagement.title", "模型管理")}</h2>
                <p>{t("codex.modelManagement.enabledDescription")}</p>
              </div>
              <button
                type="button"
                className="modal-close"
                onClick={() => closeModelConfig(false)}
                disabled={busy}
                aria-label={t("common.close", "关闭")}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              <ModalErrorMessage
                message={catalogEnabled ? modelsError : null}
                scrollKey={errorScrollKey}
              />
              <CodexExperimentalModelEditor
                models={models}
                defaultModelId={defaultModelId}
                resetModels={loadedConfig?.experimental_model_catalog_reset_models}
                resetDefaultModelId={
                  loadedConfig?.experimental_model_catalog_reset_default_model_id ?? null
                }
                mode="inline"
                availableChannels={availableChannels}
                resolveModelSource={resolveModelSource}
                onChange={(nextModels) => {
                  setModels(nextModels);
                  setNotice(null);
                  setError(null);
                }}
                onDefaultModelChange={(modelId) => {
                  setDefaultModelId(modelId);
                  setNotice(null);
                  setError(null);
                }}
                onValidationChange={setModelsError}
                onModelRemoved={(removedId) => {
                  setRoutingRoutes((prevRoutes) =>
                    toggleRouteModelInRoutes(prevRoutes, removedId, accounts, "remove"),
                  );
                }}
                onModelAdded={(addedId) => {
                  setRoutingRoutes((prevRoutes) =>
                    toggleRouteModelInRoutes(prevRoutes, addedId, accounts, "add"),
                  );
                }}
                disabled={busy}
              />
            </div>
            <div className="modal-footer">
              <button
                type="button"
                className="btn btn-outline"
                onClick={() => {
                  setCatalogEnabled(false);
                  setModelConfigSnapshot(null);
                  setModelConfigOpen(false);
                  setModelsError(null);
                }}
                disabled={busy}
              >
                {t("codex.modelManagement.disable", "关闭模型管理")}
              </button>
              <button
                type="button"
                className="btn btn-secondary"
                onClick={() => closeModelConfig(false)}
                disabled={busy}
              >
                {t("common.cancel", "取消")}
              </button>
              <button
                type="button"
                className="btn btn-primary"
                onClick={() => closeModelConfig(true)}
                disabled={busy || (catalogEnabled && Boolean(modelsError))}
              >
                {t("codex.launchPreview.applyModelConfig")}
              </button>
            </div>
          </div>
        </div>
      )}

      {contextConfigOpen && (
        <div className="modal-overlay codex-launch-preview-model-config-overlay">
          <div className="modal codex-launch-preview-model-config-modal codex-launch-preview-context-config-modal">
            <div className="modal-header">
              <div>
                <h2>{t("codex.contextOverride.title", "上下文管理")}</h2>
                <p>{t("codex.contextOverride.dialogDescription")}</p>
              </div>
              <button
                type="button"
                className="modal-close"
                onClick={() => closeContextConfig(false)}
                disabled={busy}
                aria-label={t("common.close", "关闭")}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              <CodexContextOverrideEditor
                enabled={contextOverrideEnabled}
                contextWindow={contextWindowInput}
                compactLimit={compactLimitInput}
                disabled={busy}
                onChange={(value) => {
                  setContextOverrideEnabled(value.enabled);
                  setContextWindowInput(value.contextWindow);
                  setCompactLimitInput(value.compactLimit);
                }}
              />
            </div>
            <div className="modal-footer">
              <button
                type="button"
                className="btn btn-secondary"
                onClick={() => closeContextConfig(false)}
                disabled={busy}
              >
                {t("common.cancel", "取消")}
              </button>
              <button
                type="button"
                className="btn btn-primary"
                onClick={() => closeContextConfig(true)}
                disabled={busy}
              >
                {t("codex.contextOverride.apply", "应用上下文")}
              </button>
            </div>
          </div>
        </div>
      )}

    </>
  );
}
