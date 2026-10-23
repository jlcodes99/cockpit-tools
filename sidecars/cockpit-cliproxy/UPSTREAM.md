# CLIProxyAPI upstream sync

The bundled source under `third_party/CLIProxyAPI` is synchronized to the
`CLIProxyAPI` `v7.2.155` source tree with Cockpit compatibility shims. The outer
sidecar module declares the same upstream version and continues to use the local
`replace` target.

## Codex host-result continuation — 2026-10-19

Keep `NormalizeResponsesCallIDs` aligned with the native
`responses_host_results` envelope matcher. Exact `codex_app.create_thread` and
`codex_app.send_message_to_thread` result envelopes without call IDs must not
consume pending ordinary calls. The shared Rust validator admits missing/empty
IDs only for those host records and still validates payloads, opaque provenance,
explicit IDs and ordinary tool pairings. Preserve the host-result unit and
real-worker continuation regressions when synchronizing upstream code.

## Service member pause — 2026-10-19

The wrapper keeps `pausedAccountIds` separate from membership and explicit API-key
scopes. It excludes paused credentials before affinity, backup, quota and concurrency
selection, checks direct provider admission, and preserves the final upstream-send
guard. Internal background keys and other profile manifests remain independent;
pausing never mutates shared credential `Disabled` state or empties a key scope.

Only when every remaining candidate in an explicit nonempty key scope is paused,
the wrapper marks the local `503/account_paused` rejection as terminal so Responses
WebSocket clients receive an error frame instead of silent reconnects. The small SDK
`BuildErrorResponseBodyWithError` patch preserves that exact code with
`invalid_request_error` and `retryable: false`; all other terminal authentication
errors retain upstream `upstream_authentication_required` behavior. Keep the wrapper
HTTP/SSE/WS loopback, fallback/scope and SDK serialization regressions on upstream sync.

## Codex multi-agent call-chain alignment — 2026-10-18

Reference: upstream **v8.0.32**, commit
[`911f49cbd49f4fe2516979754a7488a3fd14fe64`](https://github.com/router-for-me/CLIProxyAPI/tree/911f49cbd49f4fe2516979754a7488a3fd14fe64).
See [the complete trigger and execution matrix](../../docs/cpa-multi-agent-parity.md)
for request-boundary preparation, per-account/model compatibility, every Codex
HTTP/stream/compact/WebSocket executor, namespace restoration and continuation.
The vendored module remains v7 with the complete relevant behavior closure ported;
this is not a claim of upgrading unrelated providers or the entire v8 runtime.

The previous selective audit is superseded for this chain. The host always enables
optimization and targeted orphan-delegation recovery, and chooses input compatibility
from the actual target protocol. Retired user flags and model lists are ignored;
the vendored upstream's internal configuration interfaces remain separate. Both engines use original client headers, the actual selected model,
recursive tool declarations and per-attempt restoration state. Preserve existing
Cockpit caller/account ownership and final-declaration plaintext proofs.

SIWC public Responses needs portable agent-message input independently of automatic
optimization; this explicit route adapter does not mean CPA itself defaults every
SIWC model to `isCompat`. Child tools still execute in the client. The gateway
adapts their declarations, messages and results and restores complete validated
history for stateless public requests, including downstream WebSocket reconnects.
It does not decrypt historical ciphertext or enable unsupported hosted tools.

Other already-integrated invocation compatibility remains: reasoning update
collapse with explicit effort precedence, public SIWC identity headers, empty
function arguments, reversible tool aliases, client discovery, bounded retry,
SSE framing and completed-output reconstruction. The full regression suites,
loopback lifecycle fixtures and transport matrices must pass before packaging.

## Collaboration plaintext markers

The SDK continuation observer also retains complete `response.output_item.done`
items per upstream attempt. A genuine successful terminal with missing or empty
output is normalized from those contiguous, nonconflicting items before the
shared store validates and persists it. Nonempty terminal output remains
authoritative; EOF, cancellation, partial items and failed terminals do not
authorize reconstruction. Original wire logs, ciphertext and public output
handling remain unchanged. Keep the SDK, HTTP/WS and real-worker regressions
when updating upstream response handling.

New plaintext collaboration calls carry `encrypted_function_args: []` for the
official client's plaintext delivery path. Codex response marking is derived
from the final transmitted tool declarations, independently of namespace
optimization. Existing encryption and signatures are preserved; unknown or
conflicting declarations do not authorize plaintext. New continuity events
receive the same markers before persistence while raw wire logs stay intact.
Chat-to-Responses conversion marks its known plaintext calls at the conversion
boundary. This patch does not repair or rewrite legacy history.

Official Codex request optimization follows the CPA gate and changes the content
label without changing its string bytes. Compatibility mode additionally projects
agent-message envelopes to standard messages. Original identity history and signed
reasoning remain intact; this adaptation is not decryption.
Shared continuation validation pairs legacy `local_shell_call` with
`function_call_output` and retains raw historical function arguments when a
matching result exists, including a client-reported argument parsing error.
Missing, duplicate, out-of-order or mismatched tool results still prevent replay.

## Conversation request sizes

The local continuity client does not cap prepare/bind envelopes or their returned
request bodies at 64 MiB. The Rust control listener follows the same policy, so
the bridge does not impose a smaller limit than the public conversation endpoint.
Authorization, operation deadlines and concurrent state/cache protections remain
in force. The shared 256 MiB copy-accounting budget does not reject a lone active
large conversation; additional requests still respect the aggregate budget.
Completed or cancelled tickets release their reservations, and idempotent retries
retain the same scope checks.

Accepted complete histories are stored in the shared identity database as
caller-scoped, losslessly compressed wire items and ordered response manifests.
The 8 MiB / 64 MiB / 128-entry hot cache is optional; its size and eviction no
longer determine whether an accepted response can be continued after restart or
an engine change. Shared history items are deduplicated, including opaque tool
arguments and ciphertext without modification. Only verified complete responses
publish durable history; incomplete WebSocket anchors remain connection-local
and pinned to their original caller/account/owner/mapped identities. The current
WebSocket replay anchor is not discarded at 16 MiB. This storage grows with
accepted history and does not reconstruct checkpoints lost by older versions.

## Codex model context templates

Codex context defaults come from the bundled client catalog. The registry builds
the context-template index while loading a catalog and publishes it atomically
with that catalog's revision. Model lookup must reuse this index instead of
copying and decoding the full prompt-bearing JSON for every model. Returned
optional compaction limits remain caller-owned; catalog refreshes replace the
index without exposing partial state. Keep the refresh, concurrency and lookup
benchmarks when updating the registry.

## Codex sync scope

- Upstream repository: `https://github.com/router-for-me/CLIProxyAPI`
- Synchronized module baseline: `v7.2.155` (`7fac6b15`)
- Codex Live baseline: the upstream Live implementation plus later capability,
  client-secret, WebSocket, media-relay, and TCP-proxy commits
- Cockpit integration routes: `POST /v1/live`, `GET /v1/live/:call_id`,
  `POST /v1/realtime/calls`, and `GET /v1/realtime/calls/:call_id`

Cockpit uses the upstream Codex executor, Responses WebSocket, model catalog,
auth scheduler, Live request/sideband/realtime implementation, and supporting
config/proxy utilities. The outer relay keeps API-key account scoping and rejects
provider-gateway profiles because Codex Live requires a ChatGPT OAuth credential.

## Codex/API policy convergence

The local `CLIProxyAPI` v7.2.157 tree (`09a29bd3`) is the reference for Codex and
Responses API behavior. This is a targeted policy alignment, not a full upgrade
of the bundled v7.2.155 dependency tree.

- Removed the local device/session/full fingerprint rewrite and automatic host
  projection; old account fields are retained only for import/export compatibility.
- Removed the local official-client restriction and third-party-client exceptions,
  including global/per-account controls and runtime metadata projection. Clients
  still require the normal API key and remain subject to account-scoping rules.
- Removed the API-Service-only capacity wrapper, error-code rewriting and
  transient-request cooldown bypass. The v7.2.157 bootstrap capacity detection,
  `model_not_found` account rotation, optional model-level cooling, and permanent
  OAuth failure handling are used instead.
- Removed extra host/WebSocket input namespace deletion and the retired Rust
  gateway dispatch/rejected-field retry path. Removed the uncalled handler-level
  encrypted-content retry helper; request history is not stripped and replayed.
  Protocol translators,
  signature validation, token accounting and upstream session safety remain intact.
- Retained Cockpit token authority, scoped keys, instance-specific gateways,
  Responses Lite integration, and Agent Identity as local extensions.
  Agent Identity has no equivalent in the reference tree and is not removed
  without an explicit decision about existing accounts.

## Targeted compatibility sync (2026-09)

The current local CLIProxyAPI and sub2api sources were reviewed for failure
boundary and proxy behavior. The synchronized fixes are intentionally limited
to compatibility seams: cancellation-aware HTTP/SOCKS proxy dialing, HTTPS
proxy CONNECT support for Codex WebSockets, route-aware WebSocket reuse and
ping writes, service-tier normalization, structured Responses stream errors,
and image-stream failure/truncation propagation. This is not a dependency
version bump; the v7.2.155 baseline and Cockpit-specific account scoping remain
unchanged.

The current Codex executor also derives the native OAuth routing hint after
translation, so a normalized `service_tier` cannot disagree with the header.
The larger upstream request-capability refactor is intentionally not copied
until the corresponding translator and thinking APIs are synchronized together.
- Synchronized the v7.2.157 Responses transport fixes: official nested SSE error
  payloads with preserved sequence numbers, split-CRLF framing, WebSocket prewarm
  follow-up merging, and named `function_call_output` passthrough.
- Synchronized Codex schema/header compatibility and model additions: unsupported
  Unicode property regexes are removed from tool schemas, `$CPA-SESSION-ID`
  custom headers resolve from the canonical request session, and the
  `gpt-image-2.5-flare` / `gpt-image-2.5-sunburst` image variants are accepted.
  Cockpit keeps `gpt-5.6-luna` and `gpt-image-2.5` as its local image defaults.
- Synchronized the xAI (Grok) chat path with the reference tree: the HTTP chat
  executor, auth helpers and the exported `util.InlineLocalRefs` now match
  `CLIProxyAPI` HEAD (`xai_executor.go`, `xai_executor_request.go`,
  `xai_executor_response.go`, `xai_executor_execute.go`, `xai_executor_stream.go`,
  `xai_executor_media.go`, `internal/auth/xai/*`). This brings the Codex-facing
  compatibility fixes with it: MCP-namespaced `automation_update`
  (`mcp__codex_app__automation_update`, `codex_apps__automation_update`),
  `$ref`/union tool-schema simplification with local `$ref` inlining, the
  200-tool limit with namespace folding + dispatcher tool restoration,
  image/video requests following the OAuth chat base URL, forced
  image-generation tool choices, and `previous_response_id` passthrough on
  `/responses/compact`.
  The bundled `internal/auth/xai/pkce.go` was removed because the reference tree
  dropped PKCE types for the device-code flow.
  Later WebSocket and request-compatibility updates are recorded below; the
  bundled dependency remains v7 rather than importing the entire v8 runtime.

## Grok protocol alignment (2026-10-16)

Reference: `origin/main` at `5be2cdab0869cbaa3d401891f740aebf7e15aa99`.
Read the pinned tree with `git show`; an older local checkout is not the reference.

- Go refreshes the advertised Grok protocol version from the public npm registry
  in the background with a supported fallback, a minimum of 1.0.13, bounded
  requests, and lifecycle cancellation. Invalid or obsolete header overrides
  cannot downgrade requests. Rust uses the same validation and bounded cached
  lookup via the selected transport, without depending on an installed CLI.
- Both paths normalize unsupported web-search fields, forced hosted searches,
  client functions named web_search, and Responses tool schemas. Rust also
  normalizes xAI reasoning text events and status-specific OAuth/quota errors.
- Grok agent-message input adaptation is independent of the optional client
  optimization switch. Completed plaintext collaboration calls carry explicit
  `encrypted_function_args: []`; existing encrypted evidence remains intact.
  Rust also supports inline compaction controls and the official integer-valued
  tool schema fields, without rounding large IDs or modifying custom tool text.
- Compact controls lower their tool history before dropping unsupported fields.
  A valid compact state replaces its old prefix only on the upstream wire; the
  local source history remains available. Rust uses CPA's encrypted-state shape
  checks and does not assign ownership to state omitted from the outgoing request.
  Shape validation does not establish provider provenance or decryptability.
- Go applies payload overrides at the final upstream boundary, restores sparse
  apply_patch events without losing buffered arguments, and rejects truncated
  calls. WebSocket control writes do not wait for data writes; re-registering an
  executor sharing the same session store preserves its active connections.
- Cockpit retains client tool-search execution, account-scoped identities and
  history, automatic continuation, bounded state, and non-destructive replay.
  These are intentional local capabilities, not deletions to match CPA's tool
  filtering. Independent TTS additions and CPA's internal v8 runtime abstractions
  are not imported as part of this Responses compatibility update.

## Portable agent messages and terminal events

Keep the agent-message normalization from Cockpit PR #2609: private routing
metadata belongs in an appended input_text block, not at the top level of a
standard Responses message. Both the provider gateway and xAI executor tests
cover preservation of the original message and its routing information.
Non-streaming Codex execution also recognizes response.done through its
completed, failed or incomplete status (PR #2332), without treating an unknown
terminal status as success.

## Request route observation

Cockpit records the connection actually used by each request. The Codex uTLS
HTTP/2 transport must participate in that observation explicitly: its direct
`http2.ClientConn.RoundTrip` path does not emit the standard transport's
`httptrace.GotConn` callback. Keep this hook and the fallback transport's route
metadata when synchronizing upstream transport changes. Route lookup stays
asynchronous and never substitutes the account's current selection for an
unrecorded historical connection.

## Update procedure

1. Compare the next upstream tag against the targeted `v7.2.157` policy baseline,
   while separately assessing a full dependency-tree upgrade from `v7.2.155`.
   Begin with
   `internal/runtime/executor/codex*`, `internal/client/codex`, `sdk/cliproxy/auth`,
   `sdk/api/handlers`, `internal/config`, `internal/registry`, and `sdk/proxyutil`.
2. Port the complete dependency closure instead of copying a single changed file.
3. Preserve Cockpit-specific executor identity, token ownership, account routing,
   quota, and request-policy compatibility shims.
4. Run the focused Codex executor/Live tests, sidecar tests, and a sidecar build.
