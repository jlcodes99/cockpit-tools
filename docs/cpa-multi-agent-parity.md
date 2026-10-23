# Codex multi-agent call-chain parity

Reference: CLIProxyAPI **v8.0.32**, commit
[`911f49cbd49f4fe2516979754a7488a3fd14fe64`](https://github.com/router-for-me/CLIProxyAPI/tree/911f49cbd49f4fe2516979754a7488a3fd14fe64).
This audit concerns Codex multi-agent request preparation, executor dispatch,
response restoration and conversation continuation. It does not claim that the
entire embedded CPA dependency has been upgraded to v8.

## Request and response order

1. Authenticate the downstream API key and retain the original client headers.
2. At the Responses boundary, prepare tool definitions and apply the independently
   gated orphan-delegation adapter. WebSocket create/append requests use the same
   boundary after envelope normalization. Keep originals for client history.
3. Resolve the actual account/model, including aliases, and its compatibility flag.
4. Translate the requested protocol; normalize reasoning, tool schemas and native
   request options for the selected transport.
5. Apply the gated multi-agent input and namespace transforms. SIWC public API
   requests additionally require portable message input, regardless of optional
   optimization. This extra route requirement is explicit, not a claim that CPA
   turns `isCompat` on for every SIWC credential.
6. Send HTTP/SSE or WebSocket, retaining request-local restoration state. A native
   WebSocket connection also retains the optimized namespace state across deltas.
7. Restore tool names before client protocol translation, identity observation and
   history publication. Only completed, validated output becomes a replay anchor.
8. The client executes spawn/send/followup/wait tools. Tool outputs, child messages
   and subsequent parent input traverse the same pipeline again.

## Trigger and behavior matrix

| Stage | CPA trigger and behavior | Cockpit implementation / integration |
| --- | --- | --- |
| Tool preparation | `optimize-multi-agent-v2` AND official Codex UA; before provider selection | Always enabled by the host; shared boundary preparation retains the original client UA |
| Client capability catalog | Optimization enables MultiAgentV2 for known and synthesized models; tool search requires a viable route | Native `/models` and written client catalogs enable optimization automatically; mixed Chat routes do not advertise unsupported discovery |
| Recognized UA | `Codex Desktop/`, `codex-tui/`, exact `codex_cli_rs`, `codex_cli_rs/`, `codex_exec/` | Same case-sensitive prefixes and exact match |
| Tool discovery scope | `tools` and `input[].additional_tools`, recursive namespace definitions | Both containers; public client tool search retains its separate discovery adapter |
| Message schema | Remove only `parameters.properties.message.encrypted` on function tools named spawn_agent, send_message, followup_task | Same narrow field; no rewrite inside argument values, enum/default data or unrelated tools |
| Spawn model description | Available models intersect catalog; known templates sort by priority then ID; synthesized models by display name then ID; preserve other instructions | Caller-visible host catalog, same ordering; synthesized entries do not inherit service tiers |
| Namespace optimization | Optimization gate, a spawn tool under collaboration, no reserved-name conflict | Rename declaration to collaboration-optimize; conflict leaves names unchanged |
| Namespace restoration | Only when this request/connection actually optimized | Function/custom calls; namespace, dotted and double-underscore forms; never alter argument strings or tool output content |
| Agent content only | Optimization gate on a native Codex target | Agent encrypted_content string becomes input_text containing exactly the same string; this is not decryption |
| Portable agent messages | `isCompat`, or gated optimization during non-Codex/non-Responses translation | Convert agent_message to message/user; compatibility removes author, recipient and internal passthrough metadata |
| Compatibility precedence | Resolved model metadata first; account model name/alias next; API-key model compatibility fallback; default false | Public API-key targets use protocol compatibility automatically; backend OAuth and Agent Identity retain their private protocol without a model list |
| SIWC | Separate credential/base URL/header rules; SIWC alone does not set CPA isCompat | Public route always adapts unsupported native agent envelopes; required public adaptation is automatic and does not expose model-list or optimization switches |
| Orphan delegation | Independent option AND `X-Openai-Subagent: collab_spawn` (case-insensitive value); exact codex_app namespace and create_thread/send_message_to_thread function output | Count and consume matching function calls across the entire request; only unmatched targeted outputs become user messages preserving the output text |
| Reasoning items | Codex/SIWC sanitizes before optimization; compatibility preserves third-party content and unknown signature formats | Same wire-only summary/content, store/orphan ID and signature-shape rules; ownership checks remain independent |
| Input item IDs | Supported prefixes, 64 Unicode characters, deterministic collision-safe shortening | Same final-wire adaptation; only overlong encrypted reasoning items are omitted as CPA specifies, while the source transcript remains intact |
| Final routing hint | Backend OAuth uses final model and tier, explicit account/model headers take precedence | HTTP/WS derive after mutations; API keys and SIWC do not inherit backend hints |
| Tool schema compatibility | Reserved numeric fields normalize only for non-Codex targets; schema patterns/bounds follow Codex executor rules | Independent UA gate, exact decimal representation for int64 boundaries, recursive namespaces, and SIWC schema stage |
| Paired outputs | Call IDs pair function/custom tools with results | Preserve call ID, exact arguments, output data and order; repair only from scoped observed calls |
| HTTP streaming/nonstream | Both executor paths invoke the same preparation/restoration; nonstream collects completed SSE | Common native adapter and embedded CPA invocation paths |
| WebSocket | Normalize create/append; track optimized namespace per upstream connection; conflict clears it | Native connection state and common adapter; public SIWC uses the existing HTTP replay bridge |
| Continuation | Full transcript replaces stale local state; incremental previous_response_id continues the current upstream connection | Existing owner-scoped replay, checkpoints and full-history reconstruction; no cross-account opaque replay |
| Reasoning replay cache | CPA replay cache enabled for Claude source only | Do not add Claude cache replay to native Codex requests; preserve existing source-aware translation and owner validation |
| Client dispatch | CPA does not run subagents itself | Client executes tools after restored output; gateway tests exercise the request/response protocol, not desktop orchestration |

## Automatic host policy

Cockpit always enables multi-agent optimization and targeted orphan-delegation
recovery. Retired `collection.multiAgent` flags and model lists cannot disable
these behaviors. Public API-key targets receive portable input compatibility;
backend OAuth and Agent Identity keep their official private protocol. SIWC
continues to use its required public Responses adapter independently.

The original client UA, exact orphan envelope, namespace conflicts, account
ownership and encrypted-history boundaries remain meaningful protocol conditions.
They are not user-configurable feature switches. Caller-visible model catalogs
retain the original key scope. No manually maintained compatibility-model list or
`cockpit-model-compat` map is generated by the host.

Automatic capability replacement stays inside the current key/model account
scope. Unknown capabilities first reach the selected upstream. Only definite
pre-execution rejection and complete portable history allow another account;
pinned requests, opaque history and uncertain execution are never replayed.

## Required regression coverage

- Each UA and independent optimization/compatibility/orphan gate, including disabled
  options, non-Codex clients, reserved namespace conflicts, and namespace-free deltas.
- Tool discovery then spawn, child message, send, followup, wait result, and resumed
  parent/child history through real loopback HTTP and WebSocket gateway entrypoints.
- Additional-tool declarations, empty function arguments, both tool-call formats,
  nonempty encryption evidence, malformed responses, and caller/account isolation.
- Settings save/read, concurrent or stale publication, invalid model/account input,
  credential ID replacement, caller-visible model ordering and routing metadata.

Local protocol tests can validate the gateway contract. They do not establish that
an external ChatGPT account accepts every undocumented native item, nor replace a
user-authorized run in the actual desktop client.

## Existing host extensions retained

The local legacy missing-call-ID repair remains available for previously supported
transcripts; it is not advertised as CPA v8's Responses translator behavior.
Client tool-search lowering on SIWC is a host adapter, not native hosted search
support. The host does not import arbitrary CPA YAML payload override rules; its
existing explicit request policies continue to run at the host boundary.

## Earlier validation on 2026-10-18, before automatic-policy changes

- Rust library: 3,497 passed, zero failures, 27 explicitly ignored tests.
- The two isolated official CLI tests were also run explicitly: actual child creation,
  messaging, waiting, followup and results through the native SIWC gateway, plus
  reading/editing/checking a temporary file. Both passed. The upstream was scripted
  loopback; no live OpenAI account inference was used.
- Frontend: 1,352 passed; type checking, all 18 locale catalogs, strict modal sizing
  and whitespace checks passed.
- Embedded CPA: complete executor, executor helpers, multi-agent, Responses handler,
  configuration, auth and utility test packages passed, as did the outer sidecar
  suite and standalone server build.

The default thread-stack limit is unchanged. The cooldown retry path now holds its
large route-selection future on the heap (14,240 to 1,384 bytes for the outer future),
with a size regression and the full host suite guarding the original stack failure.
