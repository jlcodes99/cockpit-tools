import { useCallback, useEffect, useState } from "react";
import { Zap } from "lucide-react";
import * as codebuddyLocalAccessService from "../../services/codebuddyLocalAccessService";
import type {
  CodebuddyLocalAccessRoutingStrategy,
  CodebuddyLocalAccessState,
} from "../../types/codebuddyLocalAccess";
import {
  CodebuddyLocalAccessModal,
  type CodebuddyLocalAccessModalMode,
} from "../CodebuddyLocalAccessModal";

const RISK_NOTICE_ACCEPTED_KEY = "agtools.codebuddy.local_access.risk_notice_accepted";

export interface CodebuddyApiServiceLaunchButtonProps {
  /** 跳转完整 API 服务页（点击 Modal 中的"打开完整页"时调用） */
  onOpenFullPage?: () => void;
  /** 平台来源：intl / cn；用于决定 Modal 中优先显示的账号 region */
  platformRegion?: "intl" | "cn";
  /** 按钮位置变体（默认 toolbar 风格） */
  variant?: "toolbar" | "header";
  /** 按钮标题 */
  title?: string;
}

/**
 * 便捷启动 CodeBuddy API 服务的入口按钮 + Modal 容器。
 *
 * 该组件封装了：
 * - 拉取反代服务状态
 * - 打开 Modal 切换三态（panel/members/remove）
 * - 启停服务、保存账号选择
 * - 风险提示首次启用弹窗（持久化标志）
 *
 * 在 CodeBuddy CN 和 CodeBuddy 主账号页共用，避免重复实现。
 */
export function CodebuddyApiServiceLaunchButton(
  props: CodebuddyApiServiceLaunchButtonProps,
) {
  const { onOpenFullPage, variant = "toolbar", title } = props;

  const [isOpen, setIsOpen] = useState(false);
  const [mode, setMode] = useState<CodebuddyLocalAccessModalMode>("panel");
  const [state, setState] = useState<CodebuddyLocalAccessState | null>(null);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [riskNoticeAccepted, setRiskNoticeAccepted] = useState(false);

  useEffect(() => {
    try {
      setRiskNoticeAccepted(localStorage.getItem(RISK_NOTICE_ACCEPTED_KEY) === "true");
    } catch {
      // ignore
    }
  }, []);

  const refreshState = useCallback(async () => {
    setLoading(true);
    try {
      const next = await codebuddyLocalAccessService.getCodebuddyLocalAccessState();
      setState(next);
    } catch (err) {
      console.error("拉取 CodeBuddy 本地访问状态失败:", err);
    } finally {
      setLoading(false);
    }
  }, []);

  const openPanel = useCallback(async () => {
    setMode("panel");
    setIsOpen(true);
    await refreshState();
  }, [refreshState]);

  const toggleEnabled = useCallback(async () => {
    setSaving(true);
    try {
      const next = await codebuddyLocalAccessService.setCodebuddyLocalAccessEnabled(
        !state?.collection?.enabled,
      );
      setState(next);
      if (next.collection.enabled) {
        try {
          localStorage.setItem(RISK_NOTICE_ACCEPTED_KEY, "true");
          setRiskNoticeAccepted(true);
        } catch {
          // ignore
        }
      }
    } catch (err) {
      console.error("切换 CodeBuddy 本地访问启用状态失败:", err);
    } finally {
      setSaving(false);
    }
  }, [state?.collection?.enabled]);

  const saveAccounts = useCallback(
    async (intlAccountIds: string[], cnAccountIds: string[]) => {
      if (!state) return;
      setSaving(true);
      try {
        const next = await codebuddyLocalAccessService.saveCodebuddyLocalAccessCollection({
          ...state.collection,
          intlAccountIds,
          cnAccountIds,
        });
        setState(next);
      } catch (err) {
        console.error("保存 CodeBuddy 本地访问账号选择失败:", err);
        throw err;
      } finally {
        setSaving(false);
      }
    },
    [state],
  );

  const removeApiService = useCallback(async () => {
    if (!state) return;
    setSaving(true);
    try {
      const next = await codebuddyLocalAccessService.setCodebuddyLocalAccessEnabled(false);
      const cleared = await codebuddyLocalAccessService.saveCodebuddyLocalAccessCollection({
        ...next.collection,
        enabled: false,
        intlAccountIds: [],
        cnAccountIds: [],
        apiKeys: [],
      });
      setState(cleared);
    } catch (err) {
      console.error("移除 CodeBuddy API 服务失败:", err);
      throw err;
    } finally {
      setSaving(false);
    }
  }, [state]);

  const rotateApiKey = useCallback(async () => {
    if (!state?.collection?.apiKeys?.[0]?.id) return;
    setSaving(true);
    try {
      const next = await codebuddyLocalAccessService.rotateCodebuddyLocalAccessApiKey(
        state.collection.apiKeys[0].id,
      );
      setState(next);
    } catch (err) {
      console.error("轮换 CodeBuddy 本地访问 Key 失败:", err);
    } finally {
      setSaving(false);
    }
  }, [state?.collection?.apiKeys]);

  const refreshStats = useCallback(async () => {
    await refreshState();
  }, [refreshState]);

  const updateRoutingStrategy = useCallback(
    async (strategy: CodebuddyLocalAccessRoutingStrategy) => {
      if (!state) return;
      setSaving(true);
      try {
        const next = await codebuddyLocalAccessService.saveCodebuddyLocalAccessCollection({
          ...state.collection,
          routingStrategy: strategy,
        });
        setState(next);
      } catch (err) {
        console.error("更新 CodeBuddy 调度策略失败:", err);
        throw err;
      } finally {
        setSaving(false);
      }
    },
    [state],
  );

  const updateSessionAffinity = useCallback(
    async (enabled: boolean) => {
      if (!state) return;
      setSaving(true);
      try {
        const next = await codebuddyLocalAccessService.saveCodebuddyLocalAccessCollection({
          ...state.collection,
          sessionAffinity: enabled,
        });
        setState(next);
      } catch (err) {
        console.error("更新 CodeBuddy 会话亲和失败:", err);
        throw err;
      } finally {
        setSaving(false);
      }
    },
    [state],
  );

  const killPort = useCallback(async () => {
    const port = state?.actualPort ?? state?.collection?.port;
    if (!port) return;
    setSaving(true);
    try {
      await codebuddyLocalAccessService.killCodebuddyLocalAccessPort(port);
      await refreshState();
    } catch (err) {
      console.error("回收 CodeBuddy 反代服务端口失败:", err);
    } finally {
      setSaving(false);
    }
  }, [state?.actualPort, state?.collection?.port, refreshState]);

  const acceptRiskNotice = useCallback(() => {
    try {
      localStorage.setItem(RISK_NOTICE_ACCEPTED_KEY, "true");
    } catch {
      // ignore
    }
    setRiskNoticeAccepted(true);
  }, []);

  const buttonClass =
    variant === "header"
      ? "btn btn-secondary codebuddy-api-launch-btn header"
      : "btn btn-secondary icon-only codebuddy-api-launch-btn";

  const buttonTitle = title ?? "启动 API 服务";

  return (
    <>
      <button
        type="button"
        className={buttonClass}
        onClick={openPanel}
        disabled={loading && !state}
        title={buttonTitle}
        aria-label={buttonTitle}
      >
        <Zap size={variant === "header" ? 14 : 16} />
        {variant === "header" && <span>API 服务</span>}
      </button>

      <CodebuddyLocalAccessModal
        isOpen={isOpen}
        mode={mode}
        state={state}
        onClose={() => setIsOpen(false)}
        onOpenFullPage={onOpenFullPage}
        onToggleEnabled={toggleEnabled}
        onSaveAccounts={saveAccounts}
        onRemoveApiService={removeApiService}
        onKillPort={killPort}
        onRotateApiKey={rotateApiKey}
        onRefreshStats={refreshStats}
        onUpdateRoutingStrategy={updateRoutingStrategy}
        onUpdateSessionAffinity={updateSessionAffinity}
        saving={saving}
        riskNoticeAccepted={riskNoticeAccepted}
        onAcceptRiskNotice={acceptRiskNotice}
      />
    </>
  );
}
