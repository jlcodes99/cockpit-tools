import assert from "node:assert/strict";
import test from "node:test";
import {
  canAddCodexAccountToLocalAccess,
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

test("pending oauth accounts are ineligible for API service", () => {
  const pending = account({
    authorization_status: "pending",
    account_note: "import later",
  });
  assert.equal(
    getCodexLocalAccessAccountIneligibleReason(pending, true),
    "pending_oauth",
  );
  assert.equal(isCodexLocalAccessEligibleAccount(pending, true), false);
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
  assert.equal(getCodexLocalAccessAccountIneligibleReason(ok, true), null);
  assert.equal(isCodexLocalAccessEligibleAccount(ok, true), true);
});

test("eligible accounts can be added when they are not API service members", () => {
  const eligible = account({ plan_type: "plus" });

  assert.equal(
    canAddCodexAccountToLocalAccess(eligible, new Set(), true),
    true,
  );
  assert.equal(
    canAddCodexAccountToLocalAccess(
      eligible,
      new Set([eligible.id]),
      true,
    ),
    false,
  );
});

test("direct add follows the API service free-account restriction", () => {
  const free = account({ plan_type: "free" });

  assert.equal(canAddCodexAccountToLocalAccess(free, new Set(), true), false);
  assert.equal(canAddCodexAccountToLocalAccess(free, new Set(), false), true);
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

test("Agent Identity remains eligible when regular free accounts are restricted", () => {
  const agentIdentity = account({
    plan_type: "free",
    agent_identity: {
      agent_runtime_id: "runtime",
      agent_private_key: "private-key",
      account_id: "account",
      chatgpt_user_id: "user",
    },
  });

  assert.equal(getCodexLocalAccessAccountIneligibleReason(agentIdentity, true), null);
  assert.equal(isCodexOAuthBindingEligibleAccount(agentIdentity), false);
});
