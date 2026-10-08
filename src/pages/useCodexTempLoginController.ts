import { useCallback, useEffect, useRef, useState } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";
import * as codexTempLoginService from "../services/codexTempLoginService";
import type { CodexTempLoginPhase } from "../services/codexTempLoginService";
import type { CodexAccount } from "../types/codex";
import { emitAccountsChanged } from "../utils/accountSyncEvents";
import { parseWindowsOperationError } from "../utils/windowsOperationError";
import type { useCodexAccountsBaseController } from "./useCodexAccountsBaseController";
import type { useCodexAccountsOAuthController } from "./useCodexAccountsOAuthController";

type CodexTempLoginControllerContext = Pick<
  ReturnType<typeof useCodexAccountsBaseController> &
    ReturnType<typeof useCodexAccountsOAuthController>,
  | "assignCodexAccountsToTargetGroup"
  | "closeAddModal"
  | "codexAccountsRef"
  | "fetchAccounts"
  | "maskAccountText"
  | "page"
  | "showAddModal"
  | "syncImportedAccountsToApiService"
  | "syncImportedToApiService"
  | "t"
>;

/**
 * 「官方登录」业务域：打开官方客户端在一个一次性空白 profile 里完成登录，
 * 后端先关闭客户端，再读取最终登录信息；导入成功后清理临时 profile。
 */
export function useCodexTempLoginController(
  context: CodexTempLoginControllerContext,
) {
  const {
    assignCodexAccountsToTargetGroup,
    closeAddModal,
    codexAccountsRef,
    fetchAccounts,
    maskAccountText,
    page,
    showAddModal,
    syncImportedAccountsToApiService,
    syncImportedToApiService,
    t,
  } = context;

  const [tempLoginPhase, setTempLoginPhase] =
    useState<CodexTempLoginPhase | null>(null);
  const [tempLoginRunning, setTempLoginRunning] = useState(false);
  const [tempLoginCancelling, setTempLoginCancelling] = useState(false);
  const [tempLoginError, setTempLoginError] = useState<string | null>(null);
  const [tempLoginNotice, setTempLoginNotice] = useState<string | null>(null);
  const [tempLoginRecoverable, setTempLoginRecoverable] = useState(false);
  const sessionIdRef = useRef<string | null>(null);
  const recoverySessionIdRef = useRef<string | null>(null);
  const syncToApiServiceRef = useRef(syncImportedToApiService);
  syncToApiServiceRef.current = syncImportedToApiService;

  const phaseMessage = useCallback(
    (phase: CodexTempLoginPhase): string => {
      switch (phase) {
        case "preparing":
          return t("codex.tempLogin.phase.prepare", "正在准备临时配置…");
        case "launching":
          return t("codex.tempLogin.phase.launch", "正在打开官方客户端…");
        case "waiting-login":
          return t(
            "codex.tempLogin.phase.wait",
            "请在官方客户端完成登录…",
          );
        case "importing":
          return t("codex.tempLogin.phase.import", "正在读取登录信息…");
        case "closing":
          return t("codex.tempLogin.phase.close", "正在关闭官方客户端…");
        case "cleaning":
          return t("codex.tempLogin.phase.clean", "正在清理临时配置…");
        default:
          return "";
      }
    },
    [t],
  );

  const resetTempLoginState = useCallback(() => {
    sessionIdRef.current = null;
    setTempLoginRunning(false);
    setTempLoginCancelling(false);
    setTempLoginPhase(null);
  }, []);

  /**
   * 商店版 Codex 无法启动（直启 / PowerShell / 包身份都被系统拒绝）时，后端回传的是
   * 内部错误串，直接展示对用户不可读，这里换成可操作的说明并保留原始原因。
   */
  const describeTempLoginFailure = useCallback(
    (message: string): string => {
      if (message.startsWith("CODEX_TEMP_LOGIN_RECOVERY:")) {
        const path = message.slice("CODEX_TEMP_LOGIN_RECOVERY:".length).split("|")[0];
        return t("codex.tempLogin.recovery", { path });
      }
      if (message === "CODEX_TEMP_LOGIN_CLOSED") return t("codex.tempLogin.closedWithoutLogin");
      const parsed = parseWindowsOperationError(message, {
        operation: "launch_app",
      });
      if (!parsed || parsed.code !== "codex_store_launch_blocked") {
        return message;
      }
      return `${t(
        "common.windowsOperation.storeLaunchBlockedDescription",
        "Windows 拒绝了商店版 Codex 客户端的全部启动方式（直接启动、PowerShell 启动、包身份启动），已阻止启动以免打开错误的账号。多数情况下需要修复或重新安装商店版 Codex（可在 Microsoft Store 检查更新并重新安装）；也可「重新检测路径并重试」；仍失败时请把该实例的启动方式改为 CLI。",
      )}\n${parsed.originalReason}`;
    },
    [t],
  );

  const reportTempLoginFailure = useCallback(
    (message: string) => {
      setTempLoginError(describeTempLoginFailure(message));
      page.setAddStatus("error");
      page.setAddMessage(
        t("codex.tempLogin.failed", "官方登录失败：{{error}}").replace(
          "{{error}}",
          describeTempLoginFailure(message),
        ),
      );
    },
    [describeTempLoginFailure, page, t],
  );

  const handleTempLoginCompleted = useCallback(
    async (account: CodexAccount | null, accountId: string | null) => {
      const email = account?.email ?? null;
      page.setAddStatus("success");
      page.setAddMessage(
        t("codex.tempLogin.success", "已获取账号：{{email}}").replace(
          "{{email}}",
          maskAccountText(email ?? ""),
        ),
      );
      try {
        await fetchAccounts();
        // 事件自带账号快照；缺失时（旧事件格式）再回退到账号列表。
        const imported =
          account ??
          (accountId
            ? codexAccountsRef.current.find((item) => item.id === accountId) ?? null
            : null);
        if (imported) {
          await assignCodexAccountsToTargetGroup([imported]);
        }
        await emitAccountsChanged({ platformId: "codex", reason: "import" });
        if (imported && syncToApiServiceRef.current) {
          try {
            await syncImportedAccountsToApiService([imported.id]);
          } catch (error) {
            page.setAddStatus("error");
            page.setAddMessage(
              t(
                "codex.importApiService.syncFailed",
                "账号已导入，但加入 API 服务失败：{{error}}",
              ).replace("{{error}}", String(error).replace(/^Error:\s*/, "")),
            );
            return;
          }
        }
      } catch (error) {
        // 账号已经在后端落盘，读取列表失败只影响当前界面刷新。
        console.warn("[Codex Temp Login] 刷新账号列表失败:", error);
      }
      setTimeout(() => {
        closeAddModal();
      }, 1200);
    },
    [
      assignCodexAccountsToTargetGroup,
      closeAddModal,
      codexAccountsRef,
      fetchAccounts,
      maskAccountText,
      page,
      syncImportedAccountsToApiService,
      t,
    ],
  );

  useEffect(() => {
    let disposed = false;
    let unlisten: UnlistenFn | null = null;

    void codexTempLoginService
      .listenCodexTempLoginProgress((payload) => {
        if (disposed) return;
        if (sessionIdRef.current && payload.sessionId !== sessionIdRef.current) {
          return;
        }

        if (payload.phase === "completed") {
          recoverySessionIdRef.current = null;
          setTempLoginRecoverable(false);
          resetTempLoginState();
          setTempLoginError(null);
          setTempLoginNotice(null);
          void handleTempLoginCompleted(
            payload.account ?? null,
            payload.accountId ?? null,
          );
          return;
        }

        if (payload.phase === "failed") {
          if (payload.error?.startsWith("CODEX_TEMP_LOGIN_RECOVERY:")) { recoverySessionIdRef.current = payload.sessionId; setTempLoginRecoverable(true); }
          resetTempLoginState();
          reportTempLoginFailure(
            payload.error?.trim() ||
              t("codex.tempLogin.failed", "官方登录失败：{{error}}").replace(
                "{{error}}",
                "-",
              ),
          );
          return;
        }

        if (payload.phase === "cancelled") {
          resetTempLoginState();
          if (payload.error) {
            reportTempLoginFailure(payload.error);
            return;
          }
          setTempLoginError(null);
          setTempLoginNotice(
            t("codex.tempLogin.cancelled", "已取消官方登录，临时配置已清理。"),
          );
          page.setAddStatus("idle");
          page.setAddMessage("");
          return;
        }

        setTempLoginPhase(payload.phase);
        page.setAddStatus("loading");
        page.setAddMessage(phaseMessage(payload.phase));
      })
      .then((dispose) => {
        if (disposed) {
          dispose();
          return;
        }
        unlisten = dispose;
      })
      .catch((error) => {
        console.warn("[Codex Temp Login] 订阅登录进度失败:", error);
      });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [handleTempLoginCompleted, page, phaseMessage, reportTempLoginFailure, resetTempLoginState, t]);

  /** 添加账号弹框关闭时，终止仍在进行的官方登录，避免留下无人接管的临时 profile。 */
  useEffect(() => {
    if (showAddModal || !sessionIdRef.current) return;
    const sessionId = sessionIdRef.current;
    resetTempLoginState();
    void codexTempLoginService.cancelCodexTempLogin(sessionId).catch(() => {});
  }, [resetTempLoginState, showAddModal]);

  useEffect(
    () => () => {
      const sessionId = sessionIdRef.current;
      if (!sessionId) return;
      void codexTempLoginService.cancelCodexTempLogin(sessionId).catch(() => {});
    },
    [],
  );

  const handleStartCodexTempLogin = useCallback(async (fresh = false) => {
    if (sessionIdRef.current) return;
    setTempLoginError(null);
    setTempLoginNotice(null);
    setTempLoginPhase("preparing");
    setTempLoginRunning(true);
    page.setAddStatus("loading");
    page.setAddMessage(phaseMessage("preparing"));
    try {
      if (fresh) { recoverySessionIdRef.current = null; setTempLoginRecoverable(false); }
      const session = recoverySessionIdRef.current
        ? await codexTempLoginService.retryCodexTempLoginImport(recoverySessionIdRef.current)
        : await codexTempLoginService.startCodexTempLogin();
      sessionIdRef.current = session.sessionId;
    } catch (error) {
      resetTempLoginState();
      reportTempLoginFailure(String(error).replace(/^Error:\s*/, ""));
    }
  }, [
    page,
    phaseMessage,
    reportTempLoginFailure,
    resetTempLoginState,
  ]);

  const handleCancelCodexTempLogin = useCallback(async () => {
    const sessionId = sessionIdRef.current;
    if (!sessionId || tempLoginCancelling) return;
    setTempLoginCancelling(true);
    page.setAddStatus("loading");
    page.setAddMessage(
      t("codex.tempLogin.cancelling", "正在取消官方登录…"),
    );
    try {
      await codexTempLoginService.cancelCodexTempLogin(sessionId);
    } catch (error) {
      setTempLoginCancelling(false);
      reportTempLoginFailure(String(error).replace(/^Error:\s*/, ""));
    }
  }, [page, reportTempLoginFailure, t, tempLoginCancelling]);

  return {
    handleCancelCodexTempLogin,
    handleStartCodexTempLogin,
    tempLoginCancelling,
    tempLoginError,
    tempLoginRecoverable,
    tempLoginNotice,
    tempLoginPhase,
    tempLoginPhaseMessage: phaseMessage,
    tempLoginRunning,
  };
}
