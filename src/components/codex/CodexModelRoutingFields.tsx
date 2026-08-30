import type { TFunction } from "i18next";
import { confirm as confirmDialog } from "@tauri-apps/plugin-dialog";
import { GitBranch, Info, Plus, RefreshCw, SlidersHorizontal, Trash2, X } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { useEscClose } from "../../hooks/useEscClose";
import { updateCodexApiKeyCredentials } from "../../services/codexService";
import { listModelProviderModels } from "../../services/modelProviderUsageService";
import type {
  CodexApiProviderMode,
  CodexExperimentalModelDefinition,
  CodexProviderWireApi,
} from "../../types/codex";
import {
  CodexInstanceApiRoute,
  CodexInstanceModelRouting,
} from "../../types/instance";
import { SingleSelectDropdown } from "../SingleSelectDropdown";
import type { CodexExperimentalModelSource } from "./CodexExperimentalModelEditor";
import "./CodexModelRoutingFields.css";

export const CODEX_MODEL_ROUTE_NAMESPACE_PATTERN =
  /^[a-z0-9][a-z0-9_-]{1,31}$/;
export const CODEX_RESERVED_MODEL_ROUTE_NAMESPACES = new Set([
  "official",
  "subscription",
  "openai",
  "codex",
  "oauth",
]);

export type CodexModelRoutingAccount = {
  id: string;
  email: string;
  auth_mode?: string;
  openai_api_key?: string | null;
  api_base_url?: string | null;
  api_provider_mode?: string | null;
  api_provider_id?: string | null;
  api_provider_name?: string | null;
  api_model_catalog?: string[] | null;
  api_wire_api?: string | null;
};

const PINYIN_TABLE: Record<string, string> = {
  卫: "wei", 龙: "long", 智: "zhi", 谱: "pu", 华: "hua", 为: "wei",
  阿: "a", 里: "li", 腾: "teng", 讯: "xun", 百: "bai", 度: "du",
  豆: "dou", 包: "bao", 火: "huo", 山: "shan", 月: "yue", 之: "zhi",
  暗: "an", 面: "mian", 阶: "jie", 跃: "yue", 星: "xing", 辰: "chen",
  零: "ling", 一: "yi", 万: "wan", 物: "wu", 川: "chuan",
  商: "shang", 汤: "tang", 旷: "kuang", 视: "shi", 深: "shen",
  求: "qiu", 索: "suo", 幻: "huan", 方: "fang", 昆: "kun", 仑: "lun",
  天: "tian", 工: "gong", 飞: "fei", 科: "ke", 大: "da",
  通: "tong", 义: "yi", 千: "qian", 问: "wen", 文: "wen", 心: "xin",
  言: "yan", 测: "ce", 试: "shi", 中: "zhong", 转: "zhuan", 代: "dai",
  理: "li", 专: "zhuan", 线: "xian", 备: "bei", 用: "yong", 正: "zheng",
  式: "shi", 官: "guan", 主: "zhu", 本: "ben", 地: "di", 私: "si",
  有: "you", 云: "yun", 端: "duan", 服: "fu", 务: "wu", 渠: "qu",
  道: "dao", 账: "zhang", 号: "hao", 池: "chi", 快: "kuai", 速: "su",
  高: "gao", 稳: "wen", 定: "ding", 新: "xin", 旧: "jiu", 优: "you",
  选: "xuan", 极: "ji", 简: "jian", 特: "te", 惠: "hui", 企: "qi",
  业: "ye", 个: "ge", 人: "ren", 国: "guo", 内: "nei", 外: "wai",
  海: "hai", 港: "gang", 美: "mei", 日: "ri", 欧: "ou", 亚: "ya",
  二: "er", 三: "san", 四: "si", 五: "wu", 六: "liu", 七: "qi", 八: "ba", 九: "jiu", 十: "shi",
};

const transliteratePinyin = (input: string): string => {
  let result = "";
  for (const char of input) {
    if (PINYIN_TABLE[char]) {
      result += PINYIN_TABLE[char];
    } else {
      result += char;
    }
  }
  return result;
};

const slugifyRouteNamespace = (source: string): string => {
  const converted = transliteratePinyin(source);
  return converted
    .toLowerCase()
    .replace(/^https?:\/\//, "")
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 24);
};

const isSystemGeneratedId = (val: string): boolean =>
  /^(cmp|acc|account|uuid|usr|user|inst|instance)[-_0-9]/i.test(val) ||
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}/i.test(val) ||
  /^\d+$/.test(val);

const isLocalOrIpHost = (host: string): boolean => {
  const h = host.toLowerCase().trim();
  if (
    h === "localhost" ||
    h === "127.0.0.1" ||
    h === "0.0.0.0" ||
    h.startsWith("192.168.") ||
    h.startsWith("10.") ||
    /^\d+(\.\d+)*$/.test(h)
  ) {
    return true;
  }
  return false;
};

const hostFromAccount = (account?: CodexModelRoutingAccount | null): string => {
  const raw = account?.api_base_url?.trim();
  if (!raw) return "";
  try {
    return new URL(raw.includes("://") ? raw : `https://${raw}`).hostname;
  } catch {
    return "";
  }
};

export const shortCodexRouteAccountLabel = (
  account?: CodexModelRoutingAccount | null,
  fallback?: string,
): string => {
  const named = account?.api_provider_name?.trim() ?? "";
  if (named && !/^https?:\/\//i.test(named) && !named.includes(".")) {
    return named;
  }
  const host = hostFromAccount(account);
  if (host && !isLocalOrIpHost(host)) return host;
  return fallback || account?.email || named || "";
};

export const suggestCodexRouteNamespace = (
  account?: CodexModelRoutingAccount | null,
  displayText?: string,
): string => {
  if (!account && !displayText) return "";

  // 1. 下拉框 / 外部传入的明确展示名称 (如 "CPA", "卫龙", "Sub2API")
  const display = displayText?.trim() ?? "";

  // 2. 服务商自定义名称 (如 CPA, 卫龙)
  const named = account?.api_provider_name?.trim() ?? "";

  // 3. 服务商类型 ID (如 cpa, deepseek)
  const providerId = (account?.api_provider_id?.trim() ?? "").toLowerCase();
  const validProviderId =
    providerId &&
    providerId !== "custom" &&
    providerId !== "standard" &&
    providerId !== "default"
      ? providerId
      : "";

  // 4. Email 账号前缀 (过滤系统内部 ID 如 cmp-1783101683979-1)
  const emailPrefix = account?.email?.split("@")[0]?.trim() ?? "";
  const validEmailPrefix =
    emailPrefix &&
    !emailPrefix.startsWith("http") &&
    !isSystemGeneratedId(emailPrefix)
      ? emailPrefix
      : "";

  // 5. 远程域名主名 (如 api.sub2api.com -> sub2api, api.deepseek.com -> deepseek)
  const host = hostFromAccount(account);
  let domainSlug = "";
  if (host && !isLocalOrIpHost(host)) {
    const parts = host.split(".");
    if (parts.length > 1 && parts[0].toLowerCase() === "api") {
      domainSlug = parts[1] ?? "";
    } else {
      domainSlug = parts[0] ?? "";
    }
  }

  const candidates = [display, named, validProviderId, domainSlug, validEmailPrefix];
  for (const source of candidates) {
    if (!source) continue;
    const slug = slugifyRouteNamespace(source);
    if (
      CODEX_MODEL_ROUTE_NAMESPACE_PATTERN.test(slug) &&
      !CODEX_RESERVED_MODEL_ROUTE_NAMESPACES.has(slug) &&
      !isSystemGeneratedId(slug)
    ) {
      return slug;
    }
  }

  return "api";
};

export const createCodexModelRoute = (
  account?: CodexModelRoutingAccount | null,
  displayText?: string,
): CodexInstanceApiRoute => ({
  id:
    typeof crypto !== "undefined" && "randomUUID" in crypto
      ? crypto.randomUUID()
      : `route-${Date.now()}-${Math.random().toString(16).slice(2)}`,
  namespace: suggestCodexRouteNamespace(account, displayText),
  providerAccountId: account?.id ?? "",
  enabled: true,
  extraModels: [],
});

export const collectCodexModelRouteNamespaces = (
  routes: CodexInstanceApiRoute[],
): Set<string> =>
  new Set(
    routes
      .map((route) => route.namespace.trim().toLowerCase())
      .filter(Boolean),
  );

export const isCodexRouteManagedModel = (
  modelId: string,
  namespaces: Set<string>,
): boolean => {
  const normalized = modelId.trim().toLowerCase();
  const separator = normalized.indexOf("/");
  return separator > 0 && namespaces.has(normalized.slice(0, separator));
};

export const eligibleCodexModelRoutingAccounts = (
  accounts: CodexModelRoutingAccount[],
): CodexModelRoutingAccount[] =>
  accounts.filter(
    (account) => account.auth_mode?.trim().toLowerCase() === "apikey",
  );

export const collectRouteUpstreamModels = (
  route: CodexInstanceApiRoute,
  account?: CodexModelRoutingAccount | null,
): string[] => {
  const seen = new Set<string>();
  const models: string[] = [];
  for (const value of [
    ...(account?.api_model_catalog ?? []),
    ...(route.extraModels ?? []),
  ]) {
    const model = value.trim();
    if (!model || !seen.add(model.toLowerCase())) continue;
    models.push(model);
  }
  return models;
};


export const createCodexModelSourceResolver = (
  routes: CodexInstanceApiRoute[],
  accounts: CodexModelRoutingAccount[],
  t: (key: string, defaultValue?: string | any) => string,
  getAccountDisplayText?: (account: CodexModelRoutingAccount) => string,
) => (modelId: string): CodexExperimentalModelSource => {
  const normalized = modelId.trim();
  const separator = normalized.indexOf("/");
  if (separator < 0) {
    return {
      label: t("instances.form.modelRouting.subscriptionSource", "订阅"),
      kind: "subscription",
      managed: false,
    };
  }
  const namespace = normalized.slice(0, separator).trim().toLowerCase();
  const route = routes.find(
    (item) => item.namespace.trim().toLowerCase() === namespace,
  );
  if (!route) {
    return {
      label: t("instances.form.modelRouting.missingSource", "路由缺失"),
      kind: "missing",
      managed: false,
    };
  }
  if (!route.enabled) {
    return {
      label: t("instances.form.modelRouting.disabledSource", "路由已停用"),
      kind: "missing",
      managed: true,
    };
  }
  const provider = accounts.find(
    (account) => account.id === route.providerAccountId,
  );
  if (!provider) {
    return {
      label: t("instances.form.modelRouting.unavailableSource", "账号不可用"),
      kind: "missing",
      managed: true,
    };
  }
  const upstreamModel = normalized.slice(separator + 1).trim();
  const modelAvailable = (provider.api_model_catalog ?? []).some(
    (model) => model.trim().toLowerCase() === upstreamModel.toLowerCase(),
  );
  if (
    !modelAvailable &&
    (route.extraModels ?? []).every(
      (m) => m.toLowerCase() !== upstreamModel.toLowerCase(),
    )
  ) {
    return {
      label: t(
        "instances.form.modelRouting.modelUnavailable",
        "模型已不在 API 目录",
      ),
      kind: "missing",
      managed: true,
    };
  }
  return {
    label:
      provider.api_provider_name?.trim() ||
      getAccountDisplayText?.(provider) ||
      provider.email ||
      route.namespace,
    kind: "api",
    managed: true,
  };
};

export const buildCodexModelRoutingValue = (
  enabled: boolean,
  routes: CodexInstanceApiRoute[],
): CodexInstanceModelRouting | null =>
  enabled
    ? {
        enabled: true,
        version: 1,
        routes: routes.map((route) => ({
          id: route.id,
          namespace: route.namespace,
          providerAccountId: route.providerAccountId,
          enabled: route.enabled,
          selectedModels: route.selectedModels,
          extraModels: route.extraModels,
        })),
      }
    : null;

export function syncExperimentalModelsWithRouting(
  currentModels: CodexExperimentalModelDefinition[],
  routes: CodexInstanceApiRoute[],
  accounts: CodexModelRoutingAccount[],
  routingEnabled: boolean,
): CodexExperimentalModelDefinition[] {
  // 1. 如果未启用第三方路由，清理所有带前缀的第三方模型，只保留官方订阅模型
  if (!routingEnabled) {
    return currentModels.filter((model) => !model.model_id.includes("/"));
  }

  const providerAccounts = eligibleCodexModelRoutingAccounts(accounts);
  const activeChannelFullModelMap = new Map<
    string,
    { model_id: string; display_name: string }
  >();

  // 2. 收集当前所有已启用渠道勾选的模型
  for (const route of routes) {
    if (!route.enabled) continue;
    const ns = route.namespace.trim().toLowerCase();
    if (!ns) continue;

    const provider = providerAccounts.find(
      (a) => a.id === route.providerAccountId,
    );
    const allUpstream = collectRouteUpstreamModels(route, provider);
    const selected = route.selectedModels ?? allUpstream;

    for (const upstream of selected) {
      const up = upstream.trim();
      if (!up) continue;
      const fullId = `${route.namespace}/${up}`;
      activeChannelFullModelMap.set(fullId.toLowerCase(), {
        model_id: fullId,
        display_name: `${route.namespace} / ${up}`,
      });
    }
  }

  // 3. 过滤现有模型列表：
  //    - 不带 "/" 的模型（官方订阅模型）无条件保留；
  //    - 带 "/" 的第三方模型：只有在 activeChannelFullModelMap 中存在的才保留（其余历史残留、无效前缀一律清除）！
  const preservedModels: CodexExperimentalModelDefinition[] = [];
  const handledChannelIds = new Set<string>();

  for (const model of currentModels) {
    const norm = model.model_id.trim().toLowerCase();
    if (!norm.includes("/")) {
      // 官方订阅模型
      preservedModels.push(model);
    } else if (activeChannelFullModelMap.has(norm)) {
      // 当前启用的第三方渠道模型，保留用户的配置（如上下文窗口、推理强度等）
      preservedModels.push(model);
      handledChannelIds.add(norm);
    }
    // 否则直接丢弃（清理历史残留的前缀模型）
  }

  // 4. 将新勾选但在当前列表中还不存在的渠道模型追加进来
  for (const [key, item] of activeChannelFullModelMap.entries()) {
    if (!handledChannelIds.has(key)) {
      preservedModels.push({
        model_id: item.model_id,
        display_name: item.display_name,
      });
    }
  }

  return preservedModels;
}

export function toggleRouteModelInRoutes(
  routes: CodexInstanceApiRoute[],
  fullModelId: string,
  accounts: CodexModelRoutingAccount[],
  action: "remove" | "add",
): CodexInstanceApiRoute[] {
  const norm = fullModelId.trim();
  const sep = norm.indexOf("/");
  if (sep <= 0) return routes;

  const targetNs = norm.slice(0, sep).toLowerCase();
  const targetUpstream = norm.slice(sep + 1).trim();
  const providerAccounts = eligibleCodexModelRoutingAccounts(accounts);

  return routes.map((route) => {
    if (route.namespace.trim().toLowerCase() !== targetNs) return route;

    const provider = providerAccounts.find((a) => a.id === route.providerAccountId);
    const allUpstream = collectRouteUpstreamModels(route, provider);
    const currentSelected = route.selectedModels ?? allUpstream;

    if (action === "remove") {
      const nextSelected = currentSelected.filter(
        (m) => m.trim().toLowerCase() !== targetUpstream.toLowerCase(),
      );
      return {
        ...route,
        selectedModels: nextSelected,
      };
    } else {
      const alreadyHas = currentSelected.some(
        (m) => m.trim().toLowerCase() === targetUpstream.toLowerCase(),
      );
      if (alreadyHas) return route;
      return {
        ...route,
        selectedModels: [...currentSelected, targetUpstream],
      };
    }
  });
}

interface CodexModelRoutingModalProps {
  open: boolean;
  enabled?: boolean;
  routes: CodexInstanceApiRoute[];
  accounts: CodexModelRoutingAccount[];
  onClose: () => void;
  onEnabledChange?: (enabled: boolean) => void;
  onRoutesChange: (routes: CodexInstanceApiRoute[]) => void;
  onAccountsRefresh?: () => Promise<void> | void;
  getAccountDisplayText?: (account: CodexModelRoutingAccount) => string;
}

interface CodexModelRoutingFieldsProps {
  enabled: boolean;
  routes: CodexInstanceApiRoute[];
  accounts: CodexModelRoutingAccount[];
  hint?: string;
  variant?: "card" | "row";
  mode?: "summary" | "inline";
  running?: boolean;
  onEnabledChange: (enabled: boolean) => void;
  onRoutesChange: (routes: CodexInstanceApiRoute[]) => void;
  onAccountsRefresh?: () => Promise<void> | void;
  getAccountDisplayText?: (account: CodexModelRoutingAccount) => string;
}

interface CodexModelRoutingEditorProps {
  routes: CodexInstanceApiRoute[];
  accounts: CodexModelRoutingAccount[];
  onRoutesChange: (routes: CodexInstanceApiRoute[]) => void;
  onAccountsRefresh?: () => Promise<void> | void;
  getAccountDisplayText?: (account: CodexModelRoutingAccount) => string;
}

export function CodexModelRoutingEditor({
  routes,
  accounts,
  onRoutesChange,
  onAccountsRefresh,
  getAccountDisplayText,
}: CodexModelRoutingEditorProps) {
  const { t } = useTranslation();
  const [fetchingAccountId, setFetchingAccountId] = useState<string | null>(null);
  const [routeActionError, setRouteActionError] = useState<string | null>(null);
  const [manualDraftByRoute, setManualDraftByRoute] = useState<Record<string, string>>({});
  const [addingManualRouteId, setAddingManualRouteId] = useState<string | null>(null);

  const providerAccounts = useMemo(
    () => eligibleCodexModelRoutingAccounts(accounts),
    [accounts],
  );

  const providerOptions = useMemo(
    () =>
      providerAccounts.map((account) => {
        const name = shortCodexRouteAccountLabel(
          account,
          getAccountDisplayText?.(account) || account.email,
        );
        const catalogCount = account.api_model_catalog?.length ?? 0;
        return {
          value: account.id,
          label:
            catalogCount > 0
              ? `${name} · ${catalogCount} 个模型`
              : `${name} · 未获取列表`,
        };
      }),
    [getAccountDisplayText, providerAccounts],
  );

  const addManualModels = (routeId: string) => {
    const draft = (manualDraftByRoute[routeId] ?? "")
      .split(/[\n,，\s]+/)
      .map((item) => item.trim())
      .filter(Boolean);
    if (draft.length === 0) return;
    onRoutesChange(
      routes.map((item) => {
        if (item.id !== routeId) return item;
        const seen = new Set(
          (item.extraModels ?? []).map((model) => model.toLowerCase()),
        );
        const extraModels = [...(item.extraModels ?? [])];
        const selectedModels = item.selectedModels
          ? [...item.selectedModels]
          : undefined;
        const selectedSeen = new Set(
          selectedModels?.map((model) => model.toLowerCase()) ?? [],
        );
        for (const model of draft) {
          const normalized = model.toLowerCase();
          if (seen.add(normalized)) extraModels.push(model);
          if (selectedModels && selectedSeen.add(normalized)) {
            selectedModels.push(model);
          }
        }
        return { ...item, extraModels, selectedModels };
      }),
    );
    setManualDraftByRoute((current) => ({ ...current, [routeId]: "" }));
    setRouteActionError(null);
  };

  const removeManualModel = (routeId: string, modelToRemove: string) => {
    onRoutesChange(
      routes.map((item) => {
        if (item.id !== routeId) return item;
        const extraModels = (item.extraModels ?? []).filter(
          (model) => model.toLowerCase() !== modelToRemove.toLowerCase(),
        );
        const selectedModels = item.selectedModels?.filter(
          (model) => model.toLowerCase() !== modelToRemove.toLowerCase(),
        );
        return { ...item, extraModels, selectedModels };
      }),
    );
  };

  const fetchAccountCatalog = async (account: CodexModelRoutingAccount) => {
    const apiKey = account.openai_api_key?.trim();
    const baseUrl = account.api_base_url?.trim();
    if (!apiKey || !baseUrl) {
      setRouteActionError(
        t(
          "instances.form.modelRouting.fetchCredentialsRequired",
          "这个 API 账号缺少 Key 或 Base URL，无法获取模型列表。",
        ),
      );
      return;
    }
    setFetchingAccountId(account.id);
    setRouteActionError(null);
    try {
      const result = await listModelProviderModels({ baseUrl, apiKey });
      const models = Array.from(
        new Set(
          result.models
            .map((model) => model.id.trim())
            .filter(Boolean),
        ),
      );
      if (models.length === 0) {
        setRouteActionError(
          t(
            "instances.form.modelRouting.fetchEmpty",
            "上游没有返回模型，可以在下面手动填写。",
          ),
        );
        return;
      }
      await updateCodexApiKeyCredentials(
        account.id,
        apiKey,
        baseUrl,
        (account.api_provider_mode as CodexApiProviderMode | undefined) ??
          "custom",
        account.api_provider_id ?? undefined,
        account.api_provider_name ?? undefined,
        models,
        undefined,
        undefined,
        undefined,
        (account.api_wire_api as CodexProviderWireApi | undefined) ?? undefined,
      );
      await onAccountsRefresh?.();
    } catch (error) {
      setRouteActionError(
        t("instances.form.modelRouting.fetchFailed", {
          defaultValue: "获取模型列表失败：{{error}}",
          error: String(error).replace(/^Error:\s*/, ""),
        }),
      );
    } finally {
      setFetchingAccountId(null);
    }
  };

  const [searchQueryByRoute, setSearchQueryByRoute] = useState<Record<string, string>>({});

  const toggleModelSelection = (routeId: string, modelToToggle: string, allModels: string[]) => {
    onRoutesChange(
      routes.map((item) => {
        if (item.id !== routeId) return item;
        const currentSelected = item.selectedModels ?? allModels;
        const isSelected = currentSelected.some(
          (m) => m.toLowerCase() === modelToToggle.toLowerCase(),
        );
        const nextSelected = isSelected
          ? currentSelected.filter(
              (m) => m.toLowerCase() !== modelToToggle.toLowerCase(),
            )
          : [...currentSelected, modelToToggle];
        return {
          ...item,
          selectedModels:
            nextSelected.length === allModels.length ? undefined : nextSelected,
        };
      }),
    );
  };

  const selectAllModels = (routeId: string) => {
    onRoutesChange(
      routes.map((item) =>
        item.id === routeId ? { ...item, selectedModels: undefined } : item,
      ),
    );
  };

  const clearAllModels = (routeId: string) => {
    onRoutesChange(
      routes.map((item) =>
        item.id === routeId ? { ...item, selectedModels: [] } : item,
      ),
    );
  };

  if (providerAccounts.length === 0) {
    return (
      <div className="codex-model-routing-empty-accounts">
        <p>
          {t(
            "instances.form.modelRouting.noProviderAccounts",
            "未检测到可用的 API 账号。请先在「API 账号」页面添加支持的模型服务。",
          )}
        </p>
      </div>
    );
  }

  return (
    <div className="codex-model-routing-editor">
      <div className="codex-model-routing-channels">
        {routes.map((route) => {
          const provider = providerAccounts.find(
            (account) => account.id === route.providerAccountId,
          );
          const models = collectRouteUpstreamModels(route, provider);
          const showManual = addingManualRouteId === route.id;
          const extraSet = new Set(
            (route.extraModels ?? []).map((m) => m.toLowerCase()),
          );
          const currentSelected = route.selectedModels ?? models;
          const selectedSet = new Set(
            currentSelected.map((m) => m.toLowerCase()),
          );
          const query = (searchQueryByRoute[route.id] ?? "").trim().toLowerCase();
          const filteredModels = query
            ? models.filter((m) => m.toLowerCase().includes(query))
            : models;

          return (
            <div className="codex-model-routing-card" key={route.id}>
              <div className="codex-model-routing-card__header">
                <div className="codex-model-routing-card__provider">
                  <SingleSelectDropdown
                    value={route.providerAccountId}
                    onChange={(providerAccountId) =>
                      onRoutesChange(
                        routes.map((item) => {
                          if (item.id !== route.id) return item;
                          const nextProvider =
                            providerAccounts.find(
                              (account) => account.id === providerAccountId,
                            ) ?? null;
                          const nextLabel = nextProvider
                            ? shortCodexRouteAccountLabel(
                                nextProvider,
                                getAccountDisplayText?.(nextProvider) ||
                                  nextProvider.email,
                              )
                            : "";
                          const nextNamespace = suggestCodexRouteNamespace(
                            nextProvider,
                            nextLabel,
                          );
                          return {
                            ...item,
                            providerAccountId,
                            namespace: nextNamespace || item.namespace,
                            selectedModels: undefined,
                          };
                        }),
                      )
                    }
                    options={providerOptions}
                    placeholder={t(
                      "instances.form.modelRouting.provider",
                      "选择 API 账号",
                    )}
                    ariaLabel={t(
                      "instances.form.modelRouting.provider",
                      "选择 API 账号",
                    )}
                  />
                </div>
                <div className="codex-model-routing-card__namespace-wrap">
                  <input
                    className="codex-model-routing-card__namespace-input"
                    value={route.namespace}
                    onChange={(event) => {
                      const namespace = event.target.value
                        .toLowerCase()
                        .replace(/[^a-z0-9_-]/g, "");
                      onRoutesChange(
                        routes.map((item) =>
                          item.id === route.id ? { ...item, namespace } : item,
                        ),
                      );
                    }}
                    placeholder={t(
                      "instances.form.modelRouting.namespacePlaceholder",
                      "前缀 (如 cpa)",
                    )}
                    maxLength={32}
                  />
                  <span className="codex-model-routing-card__namespace-suffix">/</span>
                </div>
                <button
                  type="button"
                  className="codex-model-routing-card__delete"
                  onClick={() =>
                    onRoutesChange(routes.filter((item) => item.id !== route.id))
                  }
                  title={t("instances.form.modelRouting.deleteRoute", "删除渠道")}
                >
                  <Trash2 size={14} />
                </button>
              </div>

              <div className="codex-model-routing-card__models-section">
                <div className="codex-model-routing-card__models-bar">
                  <span className="codex-model-routing-card__models-label">
                    {t("instances.form.modelRouting.loadedModels", "已加载模型")}
                    <span className="codex-model-routing-summary__count-badge" title="已启用 / 总数">
                      {route.selectedModels ? `${route.selectedModels.length}/${models.length}` : models.length}
                    </span>
                  </span>
                  <div className="codex-model-routing-card__models-actions">
                    {models.length > 5 && (
                      <div className="codex-model-routing-card__filter-wrap">
                        <input
                          type="text"
                          className="codex-model-routing-card__filter-input"
                          value={searchQueryByRoute[route.id] ?? ""}
                          onChange={(e) =>
                            setSearchQueryByRoute((prev) => ({
                              ...prev,
                              [route.id]: e.target.value,
                            }))
                          }
                          placeholder={t("common.filter", "过滤...")}
                        />
                      </div>
                    )}
                    {models.length > 0 && (
                      <>
                        <button
                          type="button"
                          className="codex-model-routing-card__action-btn"
                          onClick={() => selectAllModels(route.id)}
                          title="全选该渠道所有模型"
                        >
                          {t("common.selectAll", "全选")}
                        </button>
                        <button
                          type="button"
                          className="codex-model-routing-card__action-btn"
                          onClick={() => clearAllModels(route.id)}
                          title="清空已选模型"
                        >
                          {t("common.clear", "清空")}
                        </button>
                      </>
                    )}
                    <button
                      type="button"
                      className="codex-model-routing-card__action-btn"
                      onClick={() => {
                        if (provider) void fetchAccountCatalog(provider);
                      }}
                      disabled={!provider || fetchingAccountId === provider?.id}
                    >
                      <RefreshCw
                        size={12}
                        className={
                          fetchingAccountId === provider?.id
                            ? "loading-spinner"
                            : undefined
                        }
                      />
                      {fetchingAccountId === provider?.id
                        ? t("common.loading", "获取中...")
                        : t("instances.form.modelRouting.fetchModels", "获取列表")}
                    </button>
                    {!showManual && (
                      <button
                        type="button"
                        className="codex-model-routing-card__action-btn"
                        onClick={() => setAddingManualRouteId(route.id)}
                      >
                        <Plus size={12} />
                        {t("instances.form.modelRouting.addModels", "添加模型")}
                      </button>
                    )}
                  </div>
                </div>

                {showManual && (
                  <div className="codex-model-routing-card__manual-input-row">
                    <input
                      className="codex-model-routing-card__manual-input"
                      value={manualDraftByRoute[route.id] ?? ""}
                      autoFocus
                      onChange={(event) =>
                        setManualDraftByRoute((current) => ({
                          ...current,
                          [route.id]: event.target.value,
                        }))
                      }
                      onKeyDown={(event) => {
                        if (event.key === "Enter") {
                          event.preventDefault();
                          addManualModels(route.id);
                          setAddingManualRouteId(null);
                        }
                        if (event.key === "Escape") {
                          setAddingManualRouteId(null);
                        }
                      }}
                      placeholder={t(
                        "instances.form.modelRouting.manualPlaceholder",
                        "gpt-5.5, grok-4.6",
                      )}
                    />
                    <button
                      type="button"
                      className="btn btn-primary btn-xs"
                      onClick={() => {
                        addManualModels(route.id);
                        setAddingManualRouteId(null);
                      }}
                    >
                      {t("common.add", "添加")}
                    </button>
                    <button
                      type="button"
                      className="btn btn-secondary btn-xs"
                      onClick={() => setAddingManualRouteId(null)}
                    >
                      {t("common.cancel", "取消")}
                    </button>
                  </div>
                )}

                {models.length === 0 ? (
                  <p className="codex-model-routing-card__empty-models">
                    {t(
                      "instances.form.modelRouting.noCatalog",
                      "尚未获取上游模型，点击「获取列表」或手动添加。",
                    )}
                  </p>
                ) : (
                  <div className="codex-model-routing-card__pills">
                    {filteredModels.map((model) => {
                      const isExtra = extraSet.has(model.toLowerCase());
                      const isSelected = selectedSet.has(model.toLowerCase());
                      const formatted = route.namespace
                        ? `${route.namespace}/${model}`
                        : model;
                      return (
                        <span
                          className={`codex-model-routing-card__pill${
                            isSelected ? " is-selected" : ""
                          }`}
                          key={model}
                          onClick={() => toggleModelSelection(route.id, model, models)}
                          title={isSelected ? t("instances.form.modelRouting.clickToDeselect", "已勾选（点击取消）") : t("instances.form.modelRouting.clickToSelect", "未勾选（点击选择）")}
                        >
                          <span className="codex-model-routing-card__pill-checkbox">
                            {isSelected ? "✓" : ""}
                          </span>
                          {formatted}
                          {isExtra && (
                            <button
                              type="button"
                              className="codex-model-routing-card__pill-remove"
                              onClick={(e) => {
                                e.stopPropagation();
                                removeManualModel(route.id, model);
                              }}
                              title={t("common.remove", "删除")}
                            >
                              <X size={10} />
                            </button>
                          )}
                        </span>
                      );
                    })}
                  </div>
                )}
              </div>
            </div>
          );
        })}
      </div>

      <button
        type="button"
        className="btn btn-outline btn-sm codex-model-routing-modal__add-channel-btn"
        onClick={() => {
          const used = new Set(routes.map((route) => route.providerAccountId));
          const nextAccount =
            providerAccounts.find((account) => !used.has(account.id)) ??
            providerAccounts[0];
          const label = nextAccount
            ? shortCodexRouteAccountLabel(
                nextAccount,
                getAccountDisplayText?.(nextAccount) || nextAccount.email,
              )
            : "";
          onRoutesChange([
            ...routes,
            createCodexModelRoute(nextAccount ?? null, label),
          ]);
        }}
      >
        <Plus size={14} />
        {t("instances.form.modelRouting.addRoute", "添加 API 渠道")}
      </button>

      {routeActionError && (
        <p className="form-hint warning">{routeActionError}</p>
      )}
    </div>
  );
}


async function confirmRoutingToggle(
  nextEnabled: boolean,
  t: TFunction,
): Promise<boolean> {
  const title = nextEnabled
    ? t("instances.form.modelRouting.confirmEnableTitle", "开启第三方 API 路由？")
    : t("instances.form.modelRouting.confirmDisableTitle", "关闭第三方 API 路由？");
  const message = nextEnabled
    ? t(
        "instances.form.modelRouting.confirmEnableMessage",
        "开启后，Codex 的模型请求将通过 Cockpit Tools 本地服务分流。服务未运行时，官方订阅和第三方模型都可能无法使用。保存时将默认启用开机自启。\n\n确认开启吗？",
      )
    : t(
        "instances.form.modelRouting.confirmDisableMessage",
        "关闭后将停止本地网关并恢复为 100% 官方原生直连，所有第三方模型将不可用。\n\n确认关闭吗？",
      );
  const okLabel = nextEnabled
    ? t("instances.form.modelRouting.confirmEnableAction", "确认开启")
    : t("instances.form.modelRouting.confirmDisableAction", "确认关闭");
  const cancelLabel = t("common.cancel", "取消");

  let confirmed: boolean;
  try {
    confirmed = await confirmDialog(message, {
      title,
      okLabel,
      cancelLabel,
      kind: nextEnabled ? "info" : "warning",
    });
  } catch {
    confirmed = window.confirm(message);
  }
  return confirmed;
}

export function CodexModelRoutingModal({
  open,
  enabled = true,
  onEnabledChange,
  routes,
  accounts,
  onClose,
  onRoutesChange,
  onAccountsRefresh,
  getAccountDisplayText,
}: CodexModelRoutingModalProps) {
  const { t } = useTranslation();
  const [draftRoutes, setDraftRoutes] = useState<CodexInstanceApiRoute[]>([]);

  const handleModalToggle = async (nextEnabled: boolean) => {
    if (!onEnabledChange || nextEnabled === enabled) return;
    const confirmed = await confirmRoutingToggle(nextEnabled, t);
    if (!confirmed) return;
    onEnabledChange(nextEnabled);
  };

  useMemo(() => {
    if (open) {
      setDraftRoutes(
        routes.map((r) => ({
          ...r,
          extraModels: r.extraModels ? [...r.extraModels] : [],
        })),
      );
    }
  }, [open, routes]);

  useEscClose(open, onClose);

  const handleApply = useCallback(() => {
    onRoutesChange(draftRoutes);
    onClose();
  }, [draftRoutes, onClose, onRoutesChange]);

  if (!open) return null;

  return createPortal(
    <div className="modal-overlay codex-model-routing-overlay" onClick={onClose}>
      <div
        className="modal codex-model-routing-modal"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="modal-header">
          <div>
            <h2>{t("instances.form.modelRouting.modalTitle", "第三方 API 路由配置")}</h2>
            <p>
              {t(
                "instances.form.modelRouting.modalDescription",
                "配置第三方 API 账号与模型前缀。未配置前缀的模型继续走当前订阅。",
              )}
            </p>
          </div>
          <div className="codex-model-routing-modal__header-actions">
            {onEnabledChange && (
              <div className="codex-model-routing-modal__switch-wrap">
                <span className="codex-model-routing-modal__switch-label">
                  {enabled ? t("common.enabled", "已开启") : t("common.disabled", "已关闭")}
                </span>
                <label
                  className="codex-model-routing__switch"
                  title={enabled ? t("instances.form.modelRouting.clickToDisable", "点击关闭第三方路由") : t("instances.form.modelRouting.clickToEnable", "点击开启第三方路由")}
                  onClick={(e) => {
                    e.preventDefault();
                    void handleModalToggle(!enabled);
                  }}
                >
                  <input
                    type="checkbox"
                    checked={enabled}
                    onChange={() => {}}
                  />
                  <span className="codex-model-routing__switch-track" aria-hidden="true" />
                </label>
              </div>
            )}
            <button
              type="button"
              className="modal-close"
              onClick={onClose}
              aria-label={t("common.close", "关闭")}
            >
              <X size={16} />
            </button>
          </div>
        </div>

        <div className="modal-body">
          <div className="codex-model-routing-modal__tip">
            <Info size={16} className="codex-model-routing-modal__tip-icon" />
            <span>
              {t(
                "instances.form.modelRouting.modalTip",
                "开启后，官方订阅和第三方模型都会经过本地分流服务。请保持 Cockpit Tools 后台运行；服务异常时会自动恢复，连续失败则回退官方配置。",
              )}
            </span>
          </div>

          <CodexModelRoutingEditor
            routes={draftRoutes}
            accounts={accounts}
            onRoutesChange={setDraftRoutes}
            onAccountsRefresh={onAccountsRefresh}
            getAccountDisplayText={getAccountDisplayText}
          />
        </div>

        <div className="modal-footer">
          <button type="button" className="btn btn-secondary" onClick={onClose}>
            {t("common.cancel", "取消")}
          </button>
          <button type="button" className="btn btn-primary" onClick={handleApply}>
            {t("common.save", "保存")}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}

export function CodexModelRoutingSummary({
  routes,
  accounts,
  onRoutesChange,
  onAccountsRefresh,
  getAccountDisplayText,
}: {
  routes: CodexInstanceApiRoute[];
  accounts: CodexModelRoutingAccount[];
  onRoutesChange: (routes: CodexInstanceApiRoute[]) => void;
  onAccountsRefresh?: () => Promise<void> | void;
  getAccountDisplayText?: (account: CodexModelRoutingAccount) => string;
}) {
  const { t } = useTranslation();
  const [modalOpen, setModalOpen] = useState(false);
  const providerAccounts = useMemo(
    () => eligibleCodexModelRoutingAccounts(accounts),
    [accounts],
  );

  return (
    <>
      <div className="codex-model-routing-summary">
        <div className="codex-model-routing-summary__header">
          <span className="codex-model-routing-summary__title">
            {t("instances.form.modelRouting.channelsSummary", "第三方 API 渠道")}
            <span className="codex-model-routing-summary__count-badge">
              {routes.length}
            </span>
          </span>
          <button
            type="button"
            className="codex-model-routing-summary__manage-btn"
            onClick={() => setModalOpen(true)}
          >
            <SlidersHorizontal size={13} />
            {t("instances.form.modelRouting.manageChannels", "配置渠道")}
          </button>
        </div>

        {routes.length === 0 ? (
          <div className="codex-model-routing-summary__empty">
            <span>{t("instances.form.modelRouting.noChannels", "暂未添加第三方 API 渠道")}</span>
            <button
              type="button"
              className="btn btn-outline btn-xs"
              onClick={() => {
                if (providerAccounts.length > 0) {
                  const acc = providerAccounts[0];
                  const label = shortCodexRouteAccountLabel(
                    acc,
                    getAccountDisplayText?.(acc) || acc.email,
                  );
                  onRoutesChange([createCodexModelRoute(acc, label)]);
                }
                setModalOpen(true);
              }}
            >
              <Plus size={11} />
              {t("instances.form.modelRouting.addRoute", "添加渠道")}
            </button>
          </div>
        ) : (
          <div className="codex-model-routing-summary__list">
            {routes.map((route) => {
              const provider = providerAccounts.find(
                (account) => account.id === route.providerAccountId,
              );
              const accountName = shortCodexRouteAccountLabel(
                provider,
                getAccountDisplayText?.(provider!) || provider?.email,
              );
              const modelsCount = collectRouteUpstreamModels(route, provider).length;
              return (
                <div
                  className="codex-model-routing-summary__row"
                  key={route.id}
                  onClick={() => setModalOpen(true)}
                  title={t("instances.form.modelRouting.clickToEdit", "点击配置此渠道")}
                >
                  <div className="codex-model-routing-summary__row-left">
                    <code className="codex-model-routing-summary__namespace">
                      {route.namespace ? `${route.namespace}/` : "—"}
                    </code>
                    <span className="codex-model-routing-summary__account-name">
                      {accountName || t("instances.form.modelRouting.unnamedAccount", "未选择账号")}
                    </span>
                  </div>
                  <div className="codex-model-routing-summary__row-right">
                    <span className="codex-model-routing-summary__models-count">
                      {t("codex.api.modelCatalog.count", {
                        count: modelsCount,
                        defaultValue: "{{count}} 个模型",
                      })}
                    </span>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>

      <CodexModelRoutingModal
        open={modalOpen}
        enabled={true}
        routes={routes}
        accounts={accounts}
        onClose={() => setModalOpen(false)}
        onRoutesChange={onRoutesChange}
        onAccountsRefresh={onAccountsRefresh}
        getAccountDisplayText={getAccountDisplayText}
      />
    </>
  );
}

export function CodexModelRoutingFields({
  enabled,
  routes,
  accounts,
  hint,
  variant = "card",
  mode = "summary",
  onEnabledChange,
  onRoutesChange,
  onAccountsRefresh,
  getAccountDisplayText,
}: CodexModelRoutingFieldsProps) {
  const { t } = useTranslation();
  const [modalOpen, setModalOpen] = useState(false);

  const providerAccounts = useMemo(
    () => eligibleCodexModelRoutingAccounts(accounts),
    [accounts],
  );

  const totalModelsCount = useMemo(() => {
    let count = 0;
    for (const route of routes) {
      const provider = providerAccounts.find(
        (acc) => acc.id === route.providerAccountId,
      );
      count += collectRouteUpstreamModels(route, provider).length;
    }
    return count;
  }, [providerAccounts, routes]);

  const enableWithDefaultRoute = (nextEnabled: boolean) => {
    onEnabledChange(nextEnabled);
    if (nextEnabled && routes.length === 0) {
      onRoutesChange([createCodexModelRoute(providerAccounts[0] ?? null)]);
    }
  };

  const handleToggle = async (nextEnabled: boolean) => {
    if (nextEnabled === enabled) return;
    const confirmed = await confirmRoutingToggle(nextEnabled, t);
    if (!confirmed) return;
    enableWithDefaultRoute(nextEnabled);
  };

  const switchControl = (
    <label
      className="codex-model-routing__switch"
      title={enabled ? t("instances.form.modelRouting.clickToDisable", "点击关闭第三方路由") : t("instances.form.modelRouting.clickToEnable", "点击开启第三方路由")}
      onClick={(event) => {
        event.preventDefault();
        void handleToggle(!enabled);
      }}
    >
      <input
        id="codex-model-routing-enabled"
        type="checkbox"
        checked={enabled}
        onChange={() => {}}
        aria-checked={enabled}
      />
      <span className="codex-model-routing__switch-track" aria-hidden="true" />
    </label>
  );

  if (variant === "row") {
    return (
      <>
        <section className="codex-launch-preview-tool-row codex-model-routing-row">
          <div className="codex-launch-preview-tool-icon">
            <GitBranch size={16} />
          </div>
          <div className="codex-launch-preview-tool-copy">
            <div className="codex-model-routing-row__title-row">
              <h3>{t("instances.form.modelRouting.title", "第三方 API 路由")}</h3>
              {switchControl}
            </div>
            <p>
              {t(
                "instances.form.modelRouting.rowSummary",
                "普通模型使用当前订阅；带前缀模型（如 cpa/gpt-5.5）转发到第三方 API。两者都依赖本地分流服务。",
              )}
            </p>
            <div className="codex-launch-preview-tool-meta">
              <span className={enabled ? "is-enabled" : ""}>
                {enabled
                  ? t("codex.launchPreview.modelRoutingEnabled", "已启用")
                  : t("codex.launchPreview.modelRoutingDisabled", "未启用")}
              </span>
              {enabled && routes.length > 0 && (
                <>
                  <span>
                    {t("instances.form.modelRouting.routesCount", "{{count}} 个渠道", {
                      count: routes.length,
                    })}
                  </span>
                  {totalModelsCount > 0 && (
                    <span>
                      {t("instances.form.modelRouting.modelsCount", "{{count}} 个路由模型", {
                        count: totalModelsCount,
                      })}
                    </span>
                  )}
                </>
              )}
            </div>
          </div>
          <button
            type="button"
            className="btn btn-outline btn-sm codex-launch-preview-tool-action"
            onClick={() => {
              if (!enabled) {
                enableWithDefaultRoute(false);
              }
              setModalOpen(true);
            }}
          >
            {enabled
              ? t("instances.form.modelRouting.manageAction", "管理 API 渠道")
              : t("instances.form.modelRouting.enableAction", "配置 API 渠道")}
          </button>
        </section>

        <CodexModelRoutingModal
          open={modalOpen}
          enabled={enabled}
          routes={routes}
          accounts={accounts}
          onClose={() => setModalOpen(false)}
          onEnabledChange={onEnabledChange}
          onRoutesChange={onRoutesChange}
          onAccountsRefresh={onAccountsRefresh}
          getAccountDisplayText={getAccountDisplayText}
        />
      </>
    );
  }

  return (
    <div className="codex-model-routing">
      <div className="codex-model-routing__header">
        <div>
          <label htmlFor="codex-model-routing-enabled">
            {t("instances.form.modelRouting.title", "第三方 API 路由")}
          </label>
          <p className="form-hint">
            {t(
              "instances.form.modelRouting.summary",
              "普通模型使用当前订阅；带前缀模型（如 cpa/gpt-5.5）转发到第三方 API。两者都依赖本地分流服务。",
            )}
          </p>
        </div>
        {switchControl}
      </div>
      {hint && <p className="form-hint">{hint}</p>}
      {enabled && (
        mode === "inline" ? (
          <CodexModelRoutingEditor
            routes={routes}
            accounts={accounts}
            onRoutesChange={onRoutesChange}
            onAccountsRefresh={onAccountsRefresh}
            getAccountDisplayText={getAccountDisplayText}
          />
        ) : (
          <CodexModelRoutingSummary
            routes={routes}
            accounts={accounts}
            onRoutesChange={onRoutesChange}
            onAccountsRefresh={onAccountsRefresh}
            getAccountDisplayText={getAccountDisplayText}
          />
        )
      )}
    </div>
  );
}
