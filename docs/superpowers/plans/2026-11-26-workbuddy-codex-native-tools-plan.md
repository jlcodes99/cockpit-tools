# WorkBuddy Codex native-tools implementation plan

Approved design: [native tools design](../specs/2026-11-26-workbuddy-codex-native-tools-design.md).

The requested writing-plans skill is unavailable in this session; this explicit plan supplies the implementation/test gates instead. No production work preceded user approval.

## Ownership and integration

- Root integrates and owns API routing/config/profile changes, regression fences, final diff/test review, real Codex E2E, and PR.
- Protocol worker owns only new `workbuddy_delegate_protocol.rs` / related new protocol fixtures: strict tool-catalog conversion, native delegate request validation, Responses output items/events, input-history/result reconciliation. No existing shared module edits without root coordination.
- Runtime worker owns only new `workbuddy_delegate_runtime.rs` and new temporary-launcher module: safe installed-CLI discovery/spawn, bidirectional ACP RPC reader, pending callbacks/permissions, actor registry/continuation/TTL/cancel. Does not modify shared AppState/mod.rs/http.rs; root wires these.
- No worker reads actual credentials or invokes real models; root alone owns the approved 20-request model budget, which starts at zero for implementation E2E and counts retries.

## Steps

1. Finalize internal Rust interfaces and add standalone protocol/runtime modules with deterministic tests. Preserve current cloud text mode. Expose explicit native_delegate configuration; do not infer mode from a tools field.
2. Add secure per-conversation runtime: private profile, token via anonymous stdin bootstrap, only DelegateTool, deny builtin file/terminal/unknown callbacks, loopback authenticated server; no secret persistence/logging.
3. Wire per-AppState actor registry and HTTP Responses handler. Maintain actor across tool handoffs and Ready between user turns. Handle full-history and previous_response_id continuation; verify owner/thread/account/source generation/model/catalog.
4. Pin strict WorkBuddy source before generic mapping rebuild/reselection/fallback. Existing direct-injection guard remains. Never replay lost pending calls or transparently rebuild actor after source/process failure.
5. Implement Native API Service explicit route configuration and dedicated scoped-key Codex profile. Publish exact WorkBuddy model IDs/effort capabilities; force HTTP Responses; reject unsupported hosted features/compact/WS. No collection-wide account-pool mutation.
6. Deterministic regression matrix: tool/custom/namespace fidelity, malformed/unknown/schema limits, ownership, pending/results exactly-once, races, stale history, permission deny, timeout/cancel/shutdown, source loss, transport failure/no fallback, exact SSE ordering and accounting, profile restore and secret non-persistence.
7. Build/test focused modules and run relevant existing WorkBuddy/native route tests, typecheck and changed UI tests. Root reviews final diff and test coverage; fix failures before live calls.
8. Real Codex E2E within 20 new model HTTP requests (retries count), synthetic-only temporary project and isolated HOME/CODEX_HOME. Verify actual file read, diff, failing→passing command exit codes, result-fed follow-up and second user turn; record selected WorkBuddy source/model, no fallback. Stop and ask if budget exhausted.
9. Review source-only git diff, format checks, artifact/credential audit; push branch and create separate feature PR only after actual success. Existing repair PR #3301 remains independent; document dependency if still unmerged.

## Completion gates

Mocks/nonce feasibility alone are not acceptance. Do not submit a feature PR with unproven Codex tool execution, hidden account fallback, direct WorkBuddy builtin execution, or fabricated quota/usage. Gateway/process restart invalidates live pending actors explicitly; cross-restart replay is out of scope.
