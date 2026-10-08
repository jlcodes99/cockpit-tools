# CLIProxyAPI upstream sync

The bundled source under `third_party/CLIProxyAPI` is synchronized to the
`CLIProxyAPI` `v7.2.155` source tree with Cockpit compatibility shims. The outer
sidecar module declares the same upstream version and continues to use the local
`replace` target.

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
  Cockpit keeps `gpt-5.5` and `gpt-image-2.5` as its local image defaults.
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
  Known divergence: `xai_websockets_executor.go` stays at the bundled revision —
  the reference implementation depends on the newer Codex WebSocket session
  layer (`codex_websockets_connection.go` / `codex_websockets_execute.go`) that is
  outside this synchronization scope. Cockpit does not route Grok models over
  Responses WebSocket (the catalog clears `prefer_websockets` for them), so the
  HTTP chat path is authoritative.

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
