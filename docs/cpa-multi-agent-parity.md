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

## Native host follow-up — 2026-11-07

Audited CPA v8.0.58 (`e5e6ad1e1b91fc20ab9964e6fb22c0c269b9d882`)
and sub2api `0eee80ba87aa4f3aff7282bf1695aa6dc0471c36` against the
previous local reference checkouts (`90c447d3` and `d851a3847`). These changes
run in the built-in Rust host. They do not restore the retired Go sidecar or
platform-package infrastructure, and do not change the historical Go baseline.

- Standalone native OAuth images use the outer Codex credential and `/images/*`;
  they do not require the separate SIWC grant or a Responses main model. Exact
  native GPT Image 2.6 IDs are eligible; arbitrary vendor suffixes remain API-key
  models. API-key source catalogs, permissions and selected full IDs still apply.
- Preserve `max_output_tokens`; only actual Claude ingress budgets from 1 to 15
  are raised to the native Codex minimum of 16. Other protocols retain their values.
- Sanitize input item ID characters on wire copies, using collision-safe hashes
  based on original IDs; tool arguments, content and short call IDs remain intact.
- Reject forced hosted image generation on native Lite routes, retaining the
  selected model. Optional declarations do not promote the route. API-key,
  independent SIWC and standalone Images contracts are separate. Remove unsupported
  hosted `complexity`, while standalone Images forward it and markup unchanged.
- Clean output-only `content_token_count` only from upstream output replay copies;
  preserve client-supplied history and immutable ownership evidence. Restore echoed
  instructions to the caller's original JSON value after internal normalization.
- Chat providers lower explicit client tool discovery and restore JSON/SSE search
  calls. Remote compaction returns reusable CPA capsules; recognize and expand only
  CPA-minted capsules, keep unknown official state opaque, and run capsule crypto
  in the existing bounded blocking pool.
- Long-history preparation and Chat translation avoid disposable full-body copies;
  ordinary stream content and tool arguments append in place. The host's pull-based
  HTTP/SSE streams already backpressure upstream, so no new arbitrary body cap or
  parallel queue was added. Existing large-history acceptance and cancellation
  regressions remain required.
- Local token counting makes upstream `count_tokens` capability cooldown changes
  inapplicable. Existing request-scoped terminal errors, 429 delivery and neutral
  transport failures remain in force. Permanent credential revocation stays terminal;
  the host's explicit `refresh_token_reused` policy remains distinct from CPA.
- Non-Codex provider catalogs keep image-input modalities while disabling the
  Codex-only original-image-detail flag.

The historical wire fixture retains its original cases, with explicit v8.0.58
expectations for preserved output budgets and rejected forced Lite images. JSON,
SSE, loopback images/refresh, tool discovery, compaction and ownership tests verify
actual behavior. No live account inference or GUI automation is part of this audit.

## Authorized live acceptance — 2026-11-07

At the user's explicit request, a newly compiled ignored acceptance test used
the saved API service's two OAuth accounts (Team and Plus). Each model was
selected in an in-memory collection on a private loopback listener; the saved
service model, membership, keys and listener were not replaced. Credential reads
and any refresh use the normal account store and cross-process transaction, not
an independently refreshed credential snapshot. Prompts and images were synthetic.

- Both accounts returned HTTP 200 and decodable PNGs for explicitly requested
  `gpt-image-2.6-flare` and `gpt-image-2.6-sunburst` generations.
- Both returned HTTP 200 for `gpt-6.1-sol` text, including preserved empty
  instructions. Standalone image routes used outer OAuth, no SIWC grant and no
  Responses main model.
- The live edit initially exposed an upstream `images` requirement. Native
  Codex edits now normalize JSON singular/plural inputs and multipart uploads
  to JSON reference arrays and normalize masks; independent API-key wire
  representations remain unchanged. Team JSON and Plus multipart edits then
  returned HTTP 200 with `complexity: medium`. An intervening TLS handshake EOF
  was recorded as a transport failure; a separate manual rerun succeeded.
- Image dimensions are a remaining upstream discrepancy: requests specified
  `1024x1024`, but generated PNG files were `1536x1024`. The gateway forwards the
  requested size and does not resample images, so this run does not establish
  exact output-size control. Size-based source selection is request-based.
- Final local regressions: 77 Images tests and 18 host image-routing tests passed,
  including the observed missing-`images` error and unchanged API-key JSON/binary
  multipart behavior. Live acceptance stays ignored unless explicitly invoked.

No account tokens, API keys or production conversation content are included in
this record. The private live artifacts contain only synthetic outputs and
sanitized protocol evidence; the initial audit above remains a separate run.

### Output-size investigation — 2026-11-07

The authorized Team and Plus follow-up records the final serialized request at
the HTTP boundary (after installation metadata and model selection), retaining
only geometry fields and the URL path. The capture is test-only and includes no
headers, credentials, prompts, inputs or generated-image data.

Both accounts forwarded `size: 1024x1536` and `size: 2048x2048` unchanged to
`/backend-api/codex/images/generations`. The upstream response itself reported
`size: 1536x1024`, and independently decoded PNG headers agreed. Sending explicit
`aspect_ratio: 1:1` with `resolution: 1k`, or numeric `width` / `height`, did not
change that result. Even an invalid size string was accepted and returned the
same dimensions. A Team `gpt-image-2.5-flare` control behaved identically, so the
observed behavior is not confined to the 2.6 model IDs.
The final Sunburst portrait and 2K-square controls reproduced it on both accounts
as well. All 16 new 2.6 geometry requests returned HTTP 200 and valid PNGs, but
none honored the requested geometry. The 77 Images and 18 host image-routing
regressions passed after the evidence/assertion changes.

This establishes that the current OAuth Codex standalone image endpoint does
not honor the tested size controls; it is not a dropped or rewritten gateway
parameter. It does not establish a universal restriction on the public OpenAI
Images API or on all models. The public Image API documentation continues to
describe the `size` option. A separate, explicitly selected non-Lite GPT-5.4
Responses control was rejected upstream as unsupported for a ChatGPT Codex
account. The GPT-5.5 control selected the existing migrated Lite main model and
was rejected locally; it is not evidence about non-Lite image sizing.

During the investigation, the gateway preserved all requested geometry fields,
the upstream's reported actual size and the original generated bytes. The live
driver records requested/returned geometry and PNG dimensions so the evidence
can be independently checked.

Following the user's explicit request to stop sending ineffective parameters,
standalone Codex OAuth generation and edit payloads now omit `size`, `width`,
`height`, `aspect_ratio` and `resolution`. The immutable original request still
supplies local size-based source selection, including credential-refresh attempts.
Independent API-key JSON/multipart requests and Responses image tools retain
their existing parameter behavior. The gateway continues to return the upstream's
actual size and original image bytes. Regressions cover omission on generation,
JSON edits, multipart edits and refreshed requests, preservation on independent
API-key paths, and unchanged local source-selection sizes. No model substitution,
alternate-protocol fallback, resampling or additional rejection rule is introduced.

Reference: [official image-generation output options](https://developers.openai.com/api/docs/guides/image-generation#customize-image-output).
