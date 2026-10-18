import assert from "node:assert/strict";
import test from "node:test";
import {
  canChooseCodexProviderAccessMode,
  isCodexProviderAccessAccount,
  resolveCodexProviderAccessMode,
  type CodexProviderAccessAccount,
} from "./codexProviderAccess.ts";

const apiKey: CodexProviderAccessAccount = { auth_mode: "apikey", api_wire_api: "responses" };

test("Responses API keys retain their persisted modes independently of provider identity", () => {
  for (const mode of ["gateway", "direct", "cdp"] as const) {
    const account = { ...apiKey, api_instance_access_mode: mode };
    assert.equal(canChooseCodexProviderAccessMode(account), true);
    assert.equal(resolveCodexProviderAccessMode(account), mode);
  }
  assert.equal(resolveCodexProviderAccessMode({ ...apiKey, api_instance_access_mode: " DIRECT " }), "direct");
});

test("missing protocol uses the backend's Responses default, with gateway as the default mode", () => {
  for (const api_wire_api of [undefined, null, "", "  "]) {
    const account = { auth_mode: "apikey", api_wire_api } as CodexProviderAccessAccount;
    assert.equal(canChooseCodexProviderAccessMode(account), true);
    assert.equal(resolveCodexProviderAccessMode(account), "gateway");
    assert.equal(resolveCodexProviderAccessMode({ ...account, api_instance_access_mode: "direct" }), "direct");
  }
  assert.equal(resolveCodexProviderAccessMode({ ...apiKey, api_instance_access_mode: "unknown" }), "gateway");
  assert.equal(resolveCodexProviderAccessMode(null), "gateway");
});

test("Chat Completions stays on the gateway and cannot select direct access", () => {
  const account: CodexProviderAccessAccount = { ...apiKey, api_wire_api: "chat_completions", api_instance_access_mode: "direct" };
  assert.equal(isCodexProviderAccessAccount(account), true);
  assert.equal(canChooseCodexProviderAccessMode(account), false);
  assert.equal(resolveCodexProviderAccessMode(account), "gateway");
});

test("OAuth, unknown protocols and Grok-linked gateway credentials cannot opt into direct access", () => {
  for (const auth_mode of [undefined, "oauth", "agent_identity", "web_session"]) {
    const account = { ...apiKey, auth_mode, api_instance_access_mode: "direct" };
    assert.equal(isCodexProviderAccessAccount(account), false);
    assert.equal(canChooseCodexProviderAccessMode(account), false);
    assert.equal(resolveCodexProviderAccessMode(account), "gateway");
  }
  const grok = { ...apiKey, upstream_grok_account_id: "grok-fixture", api_instance_access_mode: "direct" };
  assert.equal(isCodexProviderAccessAccount(grok), false);
  assert.equal(resolveCodexProviderAccessMode(grok), "gateway");
  assert.equal(canChooseCodexProviderAccessMode({ ...apiKey, api_wire_api: "unknown" } as unknown as CodexProviderAccessAccount), false);
});
