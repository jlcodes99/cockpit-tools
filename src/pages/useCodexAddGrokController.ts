import { useCallback } from "react";
import * as codexService from "../services/codexService";
import type { CodexAccount } from "../types/codex";
import { emitAccountsChanged } from "../utils/accountSyncEvents";
import type { useCodexAccountsBaseController } from "./useCodexAccountsBaseController";
import type { useCodexAccountsOAuthController } from "./useCodexAccountsOAuthController";

type CodexAddGrokControllerContext = Pick<
  ReturnType<typeof useCodexAccountsBaseController> &
    ReturnType<typeof useCodexAccountsOAuthController>,
  "fetchAccounts"
>;

/**
 * Grok 账号业务域：把 Grok 平台已登录账号接入 Codex。
 *
 * 只在 API 服务成员弹框里使用：账号不带上游 API Key，模型目录由后端带入，
 * 凭据由 Grok 账号统一维护。
 */
export function useCodexAddGrokController(
  context: CodexAddGrokControllerContext,
) {
  const { fetchAccounts } = context;

  /** 创建 Grok 供应商账号并刷新列表，供 API 服务成员弹框勾选。 */
  const handleCreateGrokUpstreamAccount = useCallback(
    async (
      grokAccountId: string,
      modelCatalog?: string[],
    ): Promise<CodexAccount> => {
      const account = await codexService.addCodexAccountFromGrok(
        grokAccountId,
        {
          // 不传模型目录时由后端带入默认 Grok 目录（成员弹框的快捷入口）。
          apiModelCatalog:
            modelCatalog && modelCatalog.length > 0 ? modelCatalog : null,
        },
      );
      await fetchAccounts();
      await emitAccountsChanged({ platformId: "codex", reason: "import" });
      return account;
    },
    [fetchAccounts],
  );

  return {
    handleCreateGrokUpstreamAccount,
  };
}
