import {
  ArrowRight,
  BarChart3,
  CircleAlert,
  KeyRound,
  Play,
  RefreshCw,
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
  useState,
  type ReactNode,
} from "react";
import { useTranslation } from "react-i18next";
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
  shortCodexRouteAccountLabel,
  syncExperimentalModelsWithRouting,
  toggleRouteModelInRoutes,
} from "./CodexModelRoutingFields";
import { forceRefreshCodexTokens } from "../../services/codexService";
import { requestCodexOpenAddAccount } from "../../utils/codexAddAccountRequest";
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
import { getCodexJwtExpiration } from "../../utils/codexSwitchAuthFailure";
import type { UnifiedQuotaMetric } from "../../presentation/platformAccountPresentation";
import { buildCodexAccountPresentation } from "../../presentation/platformAccountPresentation";
import { ModalErrorMessage, useModalErrorState } from "../ModalErrorMessage";
import {
  SingleSelectDropdown,
  type SingleSelectOption,
} from "../SingleSelectDropdown";
import { CodexQuotaMiniRows } from "./CodexQuotaMiniRows";
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

interface CodexLaunchPreviewModalProps {
  account?: CodexAccount | null;
  accountLabel: string;
  accountMetaLabel?: string;
  summary?: CodexLaunchPreviewSummary;
  actions?: CodexLaunchPreviewAction[];
  instanceId?: string;
  instanceLabel?: string;
  instanceOptions?: SingleSelectOption[];
  onInstanceChange?: (instanceId: string) => void | Promise<void>;
  mode?: "account" | "instance" | "apiService";
  onClose: () => void;
  onExecute: (launchAfterSwitch: boolean) => Promise<boolean>;
}

interface ModelConfigSnapshot {
  enabled: boolean;
  models: CodexExperimentalModelDefinition[];
  defaultModelId: string | null;
}

export function CodexLaunchPreviewModal({
  account,
  accountLabel,
  accountMetaLabel,
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
  const [modelsError, setModelsError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [changingInstance, setChangingInstance] = useState(false);
  const [runningActionId, setRunningActionId] = useState<string | null>(null);
  const [executing, setExecuting] = useState<"switch" | "launch" | null>(null);
  const [repairOpen, setRepairOpen] = useState(false);
  const [modelConfigOpen, setModelConfigOpen] = useState(false);
  const [forceRefreshing, setForceRefreshing] = useState(false);
  const [manualRefreshResult, setManualRefreshResult] = useState<{
    status: "running" | "success" | "error";
    error?: string;
  } | null>(null);
  const [manualRefreshedAccount, setManualRefreshedAccount] =
    useState<CodexAccount | null>(null);
  const [modelConfigSnapshot, setModelConfigSnapshot] =
    useState<ModelConfigSnapshot | null>(null);
  const [routingEnabled, setRoutingEnabled] = useState(false);
  const [routingRoutes, setRoutingRoutes] = useState<CodexInstanceApiRoute[]>(
    [],
  );
  const [notice, setNotice] = useState<string | null>(null);
  const accounts = useCodexAccountStore((state) => state.accounts);
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
        routingRoutes,
        accounts,
        t,
      ),
    [accounts, routingRoutes, t],
  );
  const availableChannels = useMemo(() => {
    const providerAccounts = eligibleCodexModelRoutingAccounts(accounts);
    return routingRoutes
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
  }, [accounts, routingRoutes]);
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
    !busy && !repairOpen && !modelConfigOpen && !manualRefreshResult,
    requestClose,
  );

  const applyLoadedConfig = useCallback((config: CodexQuickConfig) => {
    setLoadedConfig(config);
    setCatalogEnabled(config.experimental_model_catalog_enabled);
    setModels(config.experimental_model_catalog_models);
    setDefaultModelId(
      config.experimental_model_catalog_default_model_id ?? null,
    );
    setModelsError(null);
  }, []);

  useEffect(() => {
    let active = true;
    setLoading(true);
    setLoadedConfig(null);
    setCatalogEnabled(false);
    setModels([]);
    setDefaultModelId(null);
    setModelsError(null);
    setNotice(null);
    setManualRefreshResult(null);
    setManualRefreshedAccount(null);
    const routing = selectedInstance?.modelRouting;
    setRoutingEnabled(Boolean(routing?.enabled));
    setRoutingRoutes(routing?.routes?.map((route) => ({ ...route })) ?? []);
    void getCodexInstanceQuickConfig(instanceId)
      .then((config) => {
        if (active) {
          applyLoadedConfig(config);
          if (routing?.enabled && routing.routes?.length) {
            setModels(
              syncExperimentalModelsWithRouting(
                config.experimental_model_catalog_models,
                routing.routes,
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

  const nextModelRouting = useMemo(
    () => buildCodexModelRoutingValue(routingEnabled, routingRoutes),
    [routingEnabled, routingRoutes],
  );
  const routingDirty = useMemo(
    () =>
      JSON.stringify(selectedInstance?.modelRouting ?? null) !==
      JSON.stringify(nextModelRouting),
    [nextModelRouting, selectedInstance?.modelRouting],
  );
  const dirty = useMemo(() => {
    if (!loadedConfig && !routingDirty) return false;
    return (
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
    defaultModelId,
    loadedConfig,
    models,
    routingDirty,
  ]);

  const persistDraft = useCallback(async () => {
    if (!loadedConfig || (catalogEnabled && modelsError && !routingEnabled)) {
      if (catalogEnabled && modelsError) setError(modelsError);
      return false;
    }
    if (!dirty) return true;
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
      for (const route of routingRoutes) {
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
        if (
          route.enabled &&
          route.selectedModels !== undefined &&
          route.selectedModels.filter((model) => model.trim()).length === 0
        ) {
          setError(
            t(
              "instances.form.modelRouting.modelRequired",
              "每个已启用的 API 路由至少需要选择一个模型。",
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
          routingRoutes,
          accounts,
          true,
        );
        nextCatalogEnabled = true;
      }
      const saved = routingDirty
        ? (
            await saveCodexInstanceConfiguration({
              instanceId,
              modelRouting: nextModelRouting,
              deferBindAccountApplication: true,
              experimentalModelCatalogEnabled: nextCatalogEnabled,
              experimentalModelCatalogModels: nextModels,
              experimentalModelCatalogDefaultModelId: defaultModelId,
            })
          ).quickConfig
        : await saveCodexInstanceQuickConfig(
            instanceId,
            undefined,
            undefined,
            nextCatalogEnabled,
            nextModels,
            defaultModelId,
          );
      applyLoadedConfig(saved);
      setNotice(
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
    defaultModelId,
    dirty,
    loadedConfig,
    models,
    modelsError,
    instanceId,
    nextModelRouting,
    routingDirty,
    routingEnabled,
    routingRoutes,
    setError,
    t,
  ]);

  const handleExecute = useCallback(
    async (launchAfterSwitch: boolean) => {
      if (busy) return;
      const saved = await persistDraft();
      if (!saved) return;
      setExecuting(launchAfterSwitch ? "launch" : "switch");
      setNotice(null);
      setError(null);
      try {
        const started = await onExecute(launchAfterSwitch);
        if (!started) setExecuting(null);
      } catch (executeError) {
        setError(String(executeError).replace(/^Error:\s*/, ""));
        setExecuting(null);
      }
    },
    [busy, onExecute, persistDraft, setError],
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
  const footerToolActions = displayActions.filter(
    (action) => action.id !== "delete" && action.id !== "speed",
  );
  const subjectIcon =
    mode === "apiService" ? (
      <Server size={19} />
    ) : isApiKeySubject ? (
      <KeyRound size={19} />
    ) : (
      <UserRound size={19} />
    );

  const openModelConfig = useCallback(() => {
    if (busy || unavailable) return;
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
  }, [busy, catalogEnabled, defaultModelId, models, setError, unavailable]);

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
                  disabled={busy || !account || !isStandardCodexOAuthAccount(account)}
                >
                  {forceRefreshing
                    ? t("codex.launchPreview.forceRefreshRunning")
                    : t("codex.launchPreview.forceRefreshAction")}
                </button>
              </section>
              <section className="codex-launch-preview-tool-row">
                <div className="codex-launch-preview-tool-icon">
                  <SlidersHorizontal size={16} />
                </div>
                <div className="codex-launch-preview-tool-copy">
                  <h3>
                    {t(
                      "codex.experimentalModelCatalog.models.contextConfig",
                      "上下文与压缩",
                    )}
                  </h3>
                  <p>{t("codex.launchPreview.modelConfigDialogDescription")}</p>
                  <div className="codex-launch-preview-tool-meta">
                    <span className={catalogEnabled ? "is-enabled" : ""}>
                      {loading
                        ? t("common.loading", "加载中...")
                        : catalogEnabled
                          ? t("codex.launchPreview.modelConfigEnabled")
                          : t("codex.launchPreview.modelConfigDisabled")}
                    </span>
                    <span>
                      {t("codex.launchPreview.defaultModel", "默认模型")}：
                      {defaultModelLabel}
                    </span>
                    {models.length > 0 && (
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
                  onClick={openModelConfig}
                  disabled={busy || Boolean(unavailable)}
                >
                  {catalogEnabled
                    ? t("codex.launchPreview.manageModelConfig")
                    : t("codex.launchPreview.enablePerModel")}
                </button>
              </section>

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
              <div className="add-status success">
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
                  disabled={busy || (catalogEnabled && Boolean(modelsError))}
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
                <h2>{t("codex.launchPreview.modelConfigTitle")}</h2>
                <p>{t("codex.launchPreview.modelConfigDialogDescription")}</p>
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

    </>
  );
}
