import assert from "node:assert/strict";
import test from "node:test";
import {
  buildUsageBaseUrlCandidates,
  formatModelProviderUsageMoney,
  queryModelProviderUsage,
  resolveNewApiQuotaSnapshot,
  type ModelProviderUsageSummary,
} from "./modelProviderUsageService.ts";

function summary(
  partial: Partial<ModelProviderUsageSummary>,
): ModelProviderUsageSummary {
  return {
    modelStatsCount: 0,
    latencyMs: 0,
    ...partial,
  };
}

test("usage lookup tries a root provider URL before its /v1 fallback", () => {
  assert.deepEqual(
    buildUsageBaseUrlCandidates("https://sub2api.example.com/"),
    ["https://sub2api.example.com/", "https://sub2api.example.com/v1"],
  );
});

test("usage lookup does not rewrite providers with an explicit path", () => {
  assert.deepEqual(
    buildUsageBaseUrlCandidates("https://sub2api.example.com/api"),
    ["https://sub2api.example.com/api"],
  );
});

test("new_api quota uses token allocation details when available", () => {
  const snapshot = resolveNewApiQuotaSnapshot(
    summary({
      mode: "new_api",
      quotaLimit: 100,
      quotaRemaining: 80,
      details: [
        { key: "totalGranted", label: "Granted", value: "250" },
        { key: "totalAvailable", label: "Available", value: "175.5" },
        { key: "expiresAt", label: "Expires", value: "1800000000" },
      ],
    }),
  );

  assert.deepEqual(snapshot, {
    granted: 250,
    available: 175.5,
    expiresAt: 1800000000,
  });
});

test("new_api quota falls back to billing limits when token allocation is absent", () => {
  const snapshot = resolveNewApiQuotaSnapshot(
    summary({
      mode: "new_api",
      quotaLimit: 1849,
      quotaRemaining: 1610,
      details: [
        { key: "hardLimitUsd", label: "Hard Limit", value: "1849" },
        { key: "accessUntil", label: "Access Until", value: "1815609561" },
        { key: "totalUsage", label: "Total Usage", value: "23900" },
      ],
    }),
  );

  assert.deepEqual(snapshot, {
    granted: 1849,
    available: 1610,
    expiresAt: 1815609561,
  });
});

test("new_api quota ignores malformed numeric details", () => {
  const snapshot = resolveNewApiQuotaSnapshot(
    summary({
      mode: "new_api",
      quotaLimit: 75,
      quotaRemaining: 25,
      details: [
        { key: "totalGranted", label: "Granted", value: "unlimited" },
        { key: "totalAvailable", label: "Available", value: "" },
        { key: "expiresAt", label: "Expires", value: "never" },
      ],
    }),
  );

  assert.deepEqual(snapshot, {
    granted: 75,
    available: 25,
    expiresAt: null,
  });
});

test("token plan percentages render without currency decimals", () => {
  assert.equal(formatModelProviderUsageMoney(72, "%"), "72%");
});

test("queryModelProviderUsage retries next candidate when encountering parse error on root URL", async () => {
  const attemptedUrls: string[] = [];
  const originalWindow = globalThis.window;
  Object.defineProperty(globalThis, "window", {
    configurable: true,
    value: {
      __TAURI_INTERNALS__: {
        invoke: async (command: string, args: Record<string, unknown>) => {
          if (command === "codex_query_model_provider_usage") {
            const url = String(args.baseUrl);
            attemptedUrls.push(url);
            if (url === "https://api.apikey.fan") {
              throw new Error("PROVIDER_USAGE_PARSE_FAILED: expected value at line 1 column 1");
            }
            if (url === "https://api.apikey.fan/v1") {
              return summary({
                mode: "sub2api",
                remaining: 50,
                unit: "USD",
              });
            }
          }
          throw new Error(`Unexpected command: ${command}`);
        },
      },
    },
  });

  try {
    const result = await queryModelProviderUsage({
      baseUrl: "https://api.apikey.fan",
      apiKey: "sk-test",
      integrationType: "sub2api",
    });
    assert.deepEqual(attemptedUrls, [
      "https://api.apikey.fan",
      "https://api.apikey.fan/v1",
    ]);
    assert.equal(result.remaining, 50);
  } finally {
    if (originalWindow) {
      Object.defineProperty(globalThis, "window", {
        configurable: true,
        value: originalWindow,
      });
    } else {
      delete (globalThis as Record<string, unknown>).window;
    }
  }
});
