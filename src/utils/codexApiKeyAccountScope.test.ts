import assert from "node:assert/strict";
import test from "node:test";

import {
  isCodexApiKeyScopeAccountActive,
  reconcileCodexApiKeyScopeAccountIds,
  selectCodexApiKeyScopeAccounts,
} from "./codexApiKeyAccountScope.ts";
import type { CodexAccount } from "../types/codex.ts";

const oauthAccount = (
  id: string,
  planType = "plus",
): CodexAccount => ({
  id,
  auth_mode: "oauth",
  plan_type: planType,
} as CodexAccount);

test("includes a valid OAuth account that is outside the default service pool", () => {
  const selected = selectCodexApiKeyScopeAccounts({
    restrictFreeAccounts: true,
    scopedAccountIds: [],
    accounts: [
      oauthAccount("default-pool-account"),
      oauthAccount("pro-account", "pro"),
    ],
  });

  assert.deepEqual(
    selected.map((account) => account.id),
    ["default-pool-account", "pro-account"],
  );
});

test("excludes provider-gateway accounts but keeps a persisted fixed scope visible", () => {
  const selected = selectCodexApiKeyScopeAccounts({
    restrictFreeAccounts: true,
    scopedAccountIds: ["provider-gateway-account"],
    accounts: [
      oauthAccount("pro-account", "pro"),
      {
        id: "provider-gateway-account",
        auth_mode: "apikey",
        api_wire_api: "chat_completions",
        plan_type: "API_KEY",
      } as CodexAccount,
    ],
  });

  assert.deepEqual(
    selected.map((account) => account.id),
    ["pro-account", "provider-gateway-account"],
  );
});

test("excludes a provider-gateway account inferred from its upstream URL", () => {
  const selected = selectCodexApiKeyScopeAccounts({
    restrictFreeAccounts: true,
    scopedAccountIds: [],
    accounts: [
      oauthAccount("pro-account", "pro"),
      {
        id: "provider-gateway-account",
        auth_mode: "apikey",
        api_base_url: "https://api.deepseek.com/v1",
        plan_type: "API_KEY",
      } as CodexAccount,
    ],
  });

  assert.deepEqual(
    selected.map((account) => account.id),
    ["pro-account"],
  );
});

test("preserves an existing scoped account that is temporarily absent while filtering invalid new selections", () => {
  const accountIds = reconcileCodexApiKeyScopeAccountIds({
    accounts: [oauthAccount("pro-account", "pro")],
    restrictFreeAccounts: true,
    persistedAccountIds: ["temporarily-unavailable-account"],
    draftAccountIds: [
      "temporarily-unavailable-account",
      "pro-account",
      "new-provider-gateway-account",
    ],
  });

  assert.deepEqual(accountIds, [
    "temporarily-unavailable-account",
    "pro-account",
  ]);
});

test("marks only service-pool accounts as active when the key inherits its account pool", () => {
  assert.equal(
    isCodexApiKeyScopeAccountActive({
      accountId: "default-pool-account",
      inheritAccountPool: true,
      accountIds: ["pro-account"],
      inheritedAccountIds: ["default-pool-account"],
    }),
    true,
  );
  assert.equal(
    isCodexApiKeyScopeAccountActive({
      accountId: "pro-account",
      inheritAccountPool: true,
      accountIds: ["pro-account"],
      inheritedAccountIds: ["default-pool-account"],
    }),
    false,
  );
});
