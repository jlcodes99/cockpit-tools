import assert from "node:assert/strict";
import test from "node:test";

import {
  APIKEY_FUN_DIRECT_ENDPOINT,
  APIKEY_FUN_GLOBAL_ENDPOINT,
  APIKEY_FUN_PROVIDER_BASE_URL,
  APIKEY_FUN_REGISTER_URL,
  isApiKeyFunProviderBaseUrl,
  normalizeApiKeyFunOfficialUrl,
  normalizeApiKeyFunProviderBaseUrl,
} from "./apikeyFunLinks.ts";

test("apikey.fan constants use the new domain", () => {
  assert.equal(APIKEY_FUN_REGISTER_URL, "https://apikey.fan/register?aff=cockpit");
  assert.equal(APIKEY_FUN_GLOBAL_ENDPOINT, "https://api.apikey.fan");
  assert.equal(APIKEY_FUN_DIRECT_ENDPOINT, "https://slb.apikey.fan");
  assert.equal(APIKEY_FUN_PROVIDER_BASE_URL, "https://api.apikey.fan/v1");
});

test("official URLs migrate from the legacy domain to apikey.fan", () => {
  assert.equal(
    normalizeApiKeyFunOfficialUrl("https://apikey.fun/"),
    APIKEY_FUN_REGISTER_URL,
  );
  assert.equal(
    normalizeApiKeyFunOfficialUrl("https://apikey.fun/register?aff=other"),
    APIKEY_FUN_REGISTER_URL,
  );
  assert.equal(
    normalizeApiKeyFunOfficialUrl("https://apikey.fan/register"),
    APIKEY_FUN_REGISTER_URL,
  );
  assert.equal(
    normalizeApiKeyFunOfficialUrl("https://relay.example.com/register"),
    "https://relay.example.com/register",
  );
});

test("provider base URL detection accepts current and legacy APIKEY hosts", () => {
  assert.equal(isApiKeyFunProviderBaseUrl("https://api.apikey.fan/v1"), true);
  assert.equal(isApiKeyFunProviderBaseUrl("https://api.apikey.fun/v1/"), true);
  assert.equal(isApiKeyFunProviderBaseUrl("https://api.apikey.fan/v2"), false);
});

test("provider base URL normalization rewrites only the legacy APIKEY.FUN host", () => {
  assert.equal(
    normalizeApiKeyFunProviderBaseUrl("https://api.apikey.fun/v1/"),
    "https://api.apikey.fan/v1",
  );
  assert.equal(
    normalizeApiKeyFunProviderBaseUrl("https://api.apikey.fan/v1"),
    "https://api.apikey.fan/v1",
  );
  assert.equal(
    normalizeApiKeyFunProviderBaseUrl("https://relay.example.com/v1"),
    "https://relay.example.com/v1",
  );
});
