import type { TFunction } from "i18next";
import type { CodexLocalAccessProxyRoute } from "../../types/codexLocalAccess";

interface CodexRequestProxyLabelProps {
  route?: CodexLocalAccessProxyRoute | null;
  t: TFunction;
}

/** 只展示请求快照，避免节点切换或改名后误标历史请求。 */
export function CodexRequestProxyLabel({ route, t }: CodexRequestProxyLabelProps) {
  const name = route?.name?.trim();
  const namedRoute = (route?.kind === "node" || route?.kind === "proxy") && name;
  const value = route?.kind === "direct"
    ? t("common.requestProxy.direct")
    : namedRoute || t("common.requestProxy.unknown");
  const label = t("common.requestProxy.value", { name: value });
  const title = route?.kind === "proxy" && namedRoute
    ? `${label}\n${t("common.requestProxy.entryHint")}`
    : label;

  return <span className="codex-request-proxy-label" title={title}>{label}</span>;
}
