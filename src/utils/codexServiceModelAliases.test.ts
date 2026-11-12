import assert from "node:assert/strict";
import test from "node:test";
import { appendClaudeCodeModelAliases, parseModelAliasText } from "./codexServiceModelAliases";

test("Claude Code adds both Luna aliases and leaves Spark available under its own name", () => {
  const models = ["gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna", "gpt-5.3-codex-spark"];
  const draft = appendClaudeCodeModelAliases("", models);
  assert.deepEqual(parseModelAliasText(draft), [
    { sourceModel: "gpt-5.6-sol", alias: "claude-fable-5", fork: true },
    { sourceModel: "gpt-5.6-terra", alias: "claude-opus-4-8", fork: true },
    { sourceModel: "gpt-5.6-luna", alias: "claude-sonnet-5", fork: true },
    { sourceModel: "gpt-5.6-luna", alias: "claude-haiku-4-5-20251001", fork: true },
  ]);
  assert.equal(appendClaudeCodeModelAliases(draft, models), draft);
});

test("the preset appends supported models without replacing existing aliases or model IDs", () => {
  const draft = "custom-model as CLAUDE-SONNET-5\nunfinished edit\n";
  const result = appendClaudeCodeModelAliases(draft, ["gpt-5.6-luna", "claude-fable-5"]);
  assert.equal(result, draft + "gpt-5.6-luna => claude-haiku-4-5-20251001 +");
  assert.equal(appendClaudeCodeModelAliases(draft, []), draft);
  assert.equal(appendClaudeCodeModelAliases("", ["gpt-5.6-luna", "claude-haiku-4-5-20251001"]),
    "gpt-5.6-luna => claude-sonnet-5 +");
});

test("the preset respects a draft that replaces the original model name", () => {
  const draft = "gpt-5.6-luna => luna";
  const result = appendClaudeCodeModelAliases(draft, ["gpt-5.6-luna"]);
  assert.deepEqual(parseModelAliasText(result).map(({ alias, fork }) => [alias, fork]), [
    ["luna", false],
    ["claude-sonnet-5", false],
    ["claude-haiku-4-5-20251001", false],
  ]);
});
