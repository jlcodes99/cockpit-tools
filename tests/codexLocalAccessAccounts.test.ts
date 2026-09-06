import assert from "node:assert/strict";
import { describe, it } from "node:test";
import type { CodexAccount } from "../src/types/codex.ts";
import { resolveCodexLocalAccessInitialAccountIds } from "../src/utils/codexLocalAccessAccounts.ts";

const account = (id: string): CodexAccount =>
  ({
    id,
    email: `${id}@example.com`,
    plan_type: "plus",
    tokens: {
      id_token: "",
      access_token: "",
    },
    created_at: 0,
    last_used: 0,
  }) as CodexAccount;

describe("codex local access account initialization", () => {
  it("preserves persisted member ids before the account list is loaded", () => {
    assert.deepEqual(
      resolveCodexLocalAccessInitialAccountIds(
        ["persisted", "persisted"],
        [],
        true,
        false,
      ),
      ["persisted"],
    );
  });

  it("filters missing member ids only after the account list is loaded", () => {
    assert.deepEqual(
      resolveCodexLocalAccessInitialAccountIds(
        ["available", "missing"],
        [account("available")],
        true,
        true,
      ),
      ["available"],
    );
  });
});
