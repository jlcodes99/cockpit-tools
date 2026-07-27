import assert from "node:assert/strict";
import test from "node:test";

import {
  findCodexWebSessionImports,
  splitCodexImportPayloads,
} from "./codexJsonImportProgress.ts";

test("splits a JSON account array into individual payloads", () => {
  const payloads = splitCodexImportPayloads(
    JSON.stringify([{ refresh_token: "first" }, { refresh_token: "second" }]),
  );

  assert.deepEqual(payloads.map((item) => JSON.parse(item)), [
    { refresh_token: "first" },
    { refresh_token: "second" },
  ]);
});

test("keeps a single auth.json object intact", () => {
  const auth = {
    auth_mode: "chatgpt",
    tokens: { access_token: "access", id_token: "id" },
  };

  assert.deepEqual(splitCodexImportPayloads(JSON.stringify(auth)), [
    JSON.stringify(auth),
  ]);
});

test("extracts importable OpenAI OAuth accounts from a Sub2API export", () => {
  const payloads = splitCodexImportPayloads(
    JSON.stringify({
      exported_at: "2026-07-19T00:00:00Z",
      accounts: [
        { platform: "openai", type: "oauth", credentials: { access_token: "a" } },
        { platform: "anthropic", type: "oauth", credentials: { access_token: "b" } },
        { platform: "openai", type: "oauth", credentials: { access_token: "c" } },
      ],
    }),
  );

  assert.equal(payloads.length, 2);
  assert.deepEqual(
    payloads.map((item) => JSON.parse(item).credentials.access_token),
    ["a", "c"],
  );
});

test("splits newline-delimited JSON and raw token lines", () => {
  assert.equal(
    splitCodexImportPayloads('{"refresh_token":"a"}\n{"refresh_token":"b"}').length,
    2,
  );
  assert.deepEqual(splitCodexImportPayloads("token-a\ntoken-b"), [
    "token-a",
    "token-b",
  ]);
});

test("keeps malformed multiline JSON together for backend validation", () => {
  assert.deepEqual(splitCodexImportPayloads('{\n"refresh_token":\n}'), [
    '{\n"refresh_token":\n}',
  ]);
});

test("detects direct and wrapped Web Session imports", () => {
  const direct = {
    user: { email: "session@example.com" },
    account: { id: "account-session" },
    accessToken: "header.payload.signature",
    authProvider: "openai",
  };
  const wrapped = {
    session_json: JSON.stringify({
      user: { name: "Wrapped User" },
      account: { id: "account-wrapped" },
      accessToken: "header.payload.signature",
    }),
  };

  assert.deepEqual(
    findCodexWebSessionImports(JSON.stringify([direct, wrapped])),
    [{ label: "session@example.com" }, { label: "Wrapped User" }],
  );
});

test("does not classify regular token JSON as a Web Session", () => {
  assert.deepEqual(
    findCodexWebSessionImports(
      JSON.stringify({
        access_token: "header.payload.signature",
        refresh_token: "refresh-token",
      }),
    ),
    [],
  );
});
