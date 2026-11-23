import assert from "node:assert/strict";
import test from "node:test";
import {
  canAddCodexAccountToLocalAccess,
  filterCodexLocalAccessAccountIds,
  getCodexLocalAccessAccountIneligibleReason,
  isCodexLocalAccessEligibleAccount,
  isCodexOAuthBindingEligibleAccount,
  resolveImportedCodexAccountIdsForLocalAccess,
} from "./codexLocalAccessAccounts.ts";
import type { CodexAccount } from "../types/codex.ts";

function account(partial: Partial<CodexAccount>): CodexAccount {
  return {
    id: partial.id || "acc-1",
    email: partial.email || "a@example.com",
    tokens: partial.tokens || {
      id_token: "",
      access_token: "",
      refresh_token: "",
    },
    created_at: partial.created_at || Date.now(),
    ...partial,
  } as CodexAccount;
}

test('a paused member survives a settings save without admitting other incomplete accounts', () => {
  const paused = account({ id: 'paused', authorization_status: 'pending' });
  const candidate = account({ id: 'candidate', authorization_status: 'pending' });
  assert.deepEqual(filterCodexLocalAccessAccountIds(['paused', 'candidate', 'paused'], [paused, candidate], ['paused']), ['paused']);
  assert.deepEqual(filterCodexLocalAccessAccountIds(['paused', 'candidate'], [paused, candidate]), []);
});

test("pending oauth accounts are ineligible for API service", () => {
  const pending = account({
    authorization_status: "pending",
    account_note: "import later",
  });
  assert.equal(
    getCodexLocalAccessAccountIneligibleReason(pending),
    "pending_oauth",
  );
  assert.equal(isCodexLocalAccessEligibleAccount(pending), false);
});

test("authorized oauth accounts remain eligible", () => {
  const ok = account({
    tokens: {
      id_token: "id",
      access_token: "access",
      refresh_token: "refresh",
    },
    plan_type: "plus",
  });
  assert.equal(getCodexLocalAccessAccountIneligibleReason(ok), null);
  assert.equal(isCodexLocalAccessEligibleAccount(ok), true);
});

test("eligible accounts can be added when they are not API service members", () => {
  const eligible = account({ plan_type: "plus" });

  assert.equal(
    canAddCodexAccountToLocalAccess(eligible, new Set()),
    true,
  );
  assert.equal(
    canAddCodexAccountToLocalAccess(
      eligible,
      new Set([eligible.id]),
    ),
    false,
  );
});

test("Free and expired OAuth accounts can be added and kept in the API service pool", () => {
  const free = account({ id: "free", plan_type: "free" });
  const expiredPlus = account({
    id: "expired",
    plan_type: "plus",
    subscription_active_until: "2020-01-01T00:00:00Z",
  });

  assert.equal(canAddCodexAccountToLocalAccess(free, new Set()), true);
  assert.deepEqual(
    filterCodexLocalAccessAccountIds([free.id, expiredPlus.id], [free, expiredPlus]),
    [free.id, expiredPlus.id],
  );
});

test("Agent Identity imports are forced into API service without enabling global sync", () => {
  const regular = account({ id: "regular" });
  const agentIdentity = account({
    id: "agent-identity",
    agent_identity: {
      agent_runtime_id: "runtime",
      agent_private_key: "private-key",
      account_id: "account",
      chatgpt_user_id: "user",
    },
  });

  assert.deepEqual(
    resolveImportedCodexAccountIdsForLocalAccess(
      [regular, agentIdentity],
      false,
      true,
    ),
    ["agent-identity"],
  );
  assert.deepEqual(
    resolveImportedCodexAccountIdsForLocalAccess(
      [regular, agentIdentity],
      false,
      false,
    ),
    [],
  );
});

test("DeepSeek Responses accounts can join API service", () => {
  const deepseek = account({
    id: "deepseek",
    auth_mode: "apikey",
    api_provider_id: "deepseek",
    api_base_url: "https://api.deepseek.com",
    api_wire_api: "responses",
  });
  assert.equal(
    getCodexLocalAccessAccountIneligibleReason(deepseek),
    null,
  );
  assert.equal(isCodexLocalAccessEligibleAccount(deepseek), true);
  assert.equal(canAddCodexAccountToLocalAccess(deepseek, new Set()), true);
  assert.deepEqual(
    resolveImportedCodexAccountIdsForLocalAccess([deepseek], true, false),
    ["deepseek"],
  );
});

test("Web Session imports never join API service even when sync-all is enabled", () => {
  const regular = account({ id: "regular" });
  const webSession = account({
    id: "web-session",
    token_source_mode: "chatgpt_web_session",
    tokens: {
      id_token: "id",
      access_token: "access",
    },
  });

  assert.equal(
    getCodexLocalAccessAccountIneligibleReason(webSession),
    "web_session_quota_only",
  );
  assert.equal(isCodexOAuthBindingEligibleAccount(webSession), false);
  assert.deepEqual(
    resolveImportedCodexAccountIdsForLocalAccess(
      [regular, webSession],
      true,
      false,
    ),
    ["regular"],
  );
});

test("Agent Identity remains eligible for the API service", () => {
  const agentIdentity = account({
    plan_type: "free",
    agent_identity: {
      agent_runtime_id: "runtime",
      agent_private_key: "private-key",
      account_id: "account",
      chatgpt_user_id: "user",
    },
  });

  assert.equal(getCodexLocalAccessAccountIneligibleReason(agentIdentity), null);
  assert.equal(isCodexOAuthBindingEligibleAccount(agentIdentity), false);
});
