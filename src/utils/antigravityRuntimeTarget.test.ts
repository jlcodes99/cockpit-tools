import assert from "node:assert/strict";
import test from "node:test";

import {
  ANTIGRAVITY_RUNTIME_TARGETS,
  isAntigravityRuntimeTarget,
  normalizeAntigravityRuntimeTarget,
  buildEmptyAntigravityCurrentAccounts,
} from "./antigravityRuntimeTarget.ts";
import { ALL_PLATFORM_IDS, PLATFORM_PAGE_MAP } from "../types/platform.ts";

test("ANTIGRAVITY_RUNTIME_TARGETS contains exactly three targets", () => {
  assert.deepEqual(ANTIGRAVITY_RUNTIME_TARGETS, [
    "antigravity",
    "antigravity_ide",
    "antigravity_cli",
  ]);
});

test("isAntigravityRuntimeTarget recognizes valid targets and rejects invalid ones", () => {
  assert.equal(isAntigravityRuntimeTarget("antigravity"), true);
  assert.equal(isAntigravityRuntimeTarget("antigravity_ide"), true);
  assert.equal(isAntigravityRuntimeTarget("antigravity_cli"), true);

  assert.equal(isAntigravityRuntimeTarget("cursor"), false);
  assert.equal(isAntigravityRuntimeTarget("codex"), false);
  assert.equal(isAntigravityRuntimeTarget(""), false);
  assert.equal(isAntigravityRuntimeTarget(null), false);
  assert.equal(isAntigravityRuntimeTarget(undefined), false);
});

test("normalizeAntigravityRuntimeTarget handles targets, aliases, and fallbacks", () => {
  // Direct matches
  assert.equal(normalizeAntigravityRuntimeTarget("antigravity"), "antigravity");
  assert.equal(normalizeAntigravityRuntimeTarget("antigravity_ide"), "antigravity_ide");
  assert.equal(normalizeAntigravityRuntimeTarget("antigravity_cli"), "antigravity_cli");

  // Aliases for CLI
  assert.equal(normalizeAntigravityRuntimeTarget("antigravity-cli"), "antigravity_cli");
  assert.equal(normalizeAntigravityRuntimeTarget("cli"), "antigravity_cli");
  assert.equal(normalizeAntigravityRuntimeTarget("agy"), "antigravity_cli");
  assert.equal(normalizeAntigravityRuntimeTarget("  AGY  "), "antigravity_cli");

  // Fallbacks to antigravity_ide
  assert.equal(normalizeAntigravityRuntimeTarget(null), "antigravity_ide");
  assert.equal(normalizeAntigravityRuntimeTarget(undefined), "antigravity_ide");
  assert.equal(normalizeAntigravityRuntimeTarget(""), "antigravity_ide");
  assert.equal(normalizeAntigravityRuntimeTarget("unknown_runtime"), "antigravity_ide");
});

test("ALL_PLATFORM_IDS and PLATFORM_PAGE_MAP include antigravity_cli", () => {
  assert.equal(ALL_PLATFORM_IDS.includes("antigravity_cli"), true);
  assert.equal(PLATFORM_PAGE_MAP["antigravity_cli"], "overview");
});

test("buildEmptyAntigravityCurrentAccounts initializes all three targets to defaultValue", () => {
  const empty = buildEmptyAntigravityCurrentAccounts(null);
  assert.deepEqual(empty, {
    antigravity: null,
    antigravity_ide: null,
    antigravity_cli: null,
  });
});
