# Codex API Service Handoff

## Scope

This note describes the local service shown in the UI as **Codex API Service**.
It is not a hosted Cockpit API. The desktop application creates a local,
authenticated OpenAI-compatible gateway and can redirect Codex profiles to it.

The repository was imported from `jlcodes99/cockpit-tools` at commit
`9525a2b5fc519874d42213e6cac9cc90fd38ae82` on 2026-07-14.

## Architecture

```text
Codex CLI / selected Codex profile
  -> auth.json + config.toml injected by Cockpit
  -> http://localhost:<configured-port>/v1
  -> cockpit-cliproxy sidecar by default
  -> selected Codex OAuth credentials or configured provider API key
  -> OpenAI Codex upstream or provider upstream

React page -> Tauri invoke -> Rust collection/runtime -> sidecar config + manifest
```

Production traffic has a single gateway path:

- Rust materializes config and credentials, then starts the bundled Go executable
  `cockpit-cliproxy`. Every supported route is served by that sidecar.
- `CodexLocalAccessGatewayMode::Legacy` survives only as a persisted value. It is
  migrated to `sidecar` when the collection is read
  (`migrate_legacy_gateway_mode`, `collection_gateway_mode`) and is not a
  selectable runtime mode.
- The retired in-process gateway is compiled only under `#[cfg(test)]` as a test
  oracle (`codex_local_access_recovery.rs`). Do not route production traffic
  through it; route behavior is owned by the sidecar.

## Source Ownership

| Area | Primary files | Responsibility |
| --- | --- | --- |
| UI | `src/pages/CodexApiServicePage.tsx`, `src/components/CodexLocalAccessModal.tsx` | Service screen, account/key/routing controls, test panel and usage views. |
| UI bridge | `src/services/codexLocalAccessService.ts` | Typed wrappers around the Tauri commands. Keep camelCase payload names aligned with Rust command arguments. |
| Contract | `src/types/codexLocalAccess.ts`, `src-tauri/src/models/codex_local_access.rs` | Frontend/Rust state and configuration schema. Rust uses `serde(rename_all = "camelCase")`. |
| Commands | `src-tauri/src/commands/codex.rs` | Tauri command boundary; commands are registered in `src-tauri/src/lib.rs`. |
| Service coordinator | `src-tauri/src/modules/codex_local_access.rs` | Persistence, profile takeover, sidecar lifecycle, routing, quotas, request logs and tests. |
| Sidecar | `sidecars/cockpit-cliproxy/main.go` | Go HTTP gateway based on CLIProxyAPI SDK. Receives generated config and manifest. |

## Runtime Flow

1. The UI reads state through `codex_local_access_get_state` and changes it via
   `codex_local_access_*` commands.
2. Rust persists a `CodexLocalAccessCollection`, synchronizes `GatewayRuntime`,
   and calls `ensure_gateway_matches_runtime` when a gateway-relevant setting
   changes.
3. `prepare_sidecar_launch_config` writes the sidecar `config.json`,
   `manifest.json`, credentials below `auths/`, and quota-reserve state. A stable
   fingerprint decides whether the process must be restarted.
4. The sidecar authenticates the client key, applies model visibility/alias rules,
   selects an account, and sends the upstream request. It reports diagnostics and
   usage back to Rust.
5. Activating the service backs up the target profile before writing a managed
   Codex account bundle (`auth.json` and `config.toml`). Disabling restores saved
   profile state.

## HTTP Surface

The Rust proxy recognizes these client paths; the sidecar is expected to preserve
the same client contract:

- `/v1/models`
- `/v1/chat/completions`
- `/v1/responses` and `/v1/responses/compact`
- `/backend-api/codex/*`, including responses WebSocket traffic
- `/v1/images/generations` and `/v1/images/edits`

The default bind is loopback (`127.0.0.1`). The user can choose LAN scope
(`0.0.0.0`), which expands exposure but still requires a generated API key. Client
Base URL host can be `localhost` or `127.0.0.1`; preserve this when rewriting a
profile, especially for WSL.

## Routing And Safety Rules

- Account pools support auto, random distribution, single-account, quota/plan/
  expiry ordering, and custom priority/weight/backup routing. Random distribution
  shuffles eligible accounts for each new request; a session-affinity binding still
  wins for an existing conversation.
- Session affinity is enabled by default. Routing also observes cooldown and
  account-health state; retries are bounded by the configured credential count and
  delay.
- Named API keys can be disabled, limited by allowed/excluded models, and assigned
  a model prefix. The primary collection key remains for backward compatibility.
- Bound OAuth quota reserve can exclude an OAuth account when its fresh hourly or
  weekly quota snapshot drops below the configured threshold. A failed refresh is
  fail-closed for that reserve decision.
- Model aliases and filters affect both model discovery and request rewriting.
  Update both paths together and test an alias plus a rejected model.
- Image behavior is separately configurable: enabled, images-only, or disabled.
  Image requests and image tool injection have their own validation path. The
  sidecar tracks in-flight image jobs by selected auth. `maxConcurrentImageRequests`
  configures the number of job slots per account (default `1`, range `1-16`).
  It selects an eligible account with the fewest jobs and queues later image
  requests locally until a slot is released when a request ends.
  Image requests bypass session-affinity cache so a prior text/image request
  cannot pin new image jobs to an already busy account.
- `immediateSseResponse` is disabled by default. In sidecar mode it commits a
  `200 OK` SSE response with an ignored `: accepted` comment before the upstream
  stream opens. This also applies to streaming image generation and edits. Once
  HTTP headers are committed, upstream-open failures are sent as SSE `error`
  events and cannot change the HTTP status.

## Persistent Artifacts And Logs

Artifact paths are derived from the application data directory in
`codex_local_access.rs`; do not hard-code an OS-specific profile path.

- `codex_local_access.json`: durable collection and service configuration.
- `codex_local_access_stats.json`: summarized counters/recent events.
- `codex_local_access_logs.sqlite`: queryable request/usage records.
- `codex_local_access_takeover_backups.json`: profile restoration data.
- `codex_local_access_sidecar/`: generated sidecar `config.json`, `manifest.json`,
  auth files, and quota-reserve state.
- `codex_provider_gateway_sidecars/`: isolated generated gateways for direct
  external model providers.

Use `logger::log_codex_api_info`, `log_codex_api_warn`, and
`log_codex_api_error` for Rust service messages. The sidecar emits request
diagnostic and usage payloads; inspect its generated manifest only locally because
it can contain credentials.

## Debug Checklist

1. Call `codex_local_access_get_state`: confirm `collection.enabled`, `running`,
   `baseUrl`, `lastError`, selected account count, mode, and port.
2. Verify the configured port is free or use `codex_local_access_kill_port` only
   after identifying the owning process.
3. In sidecar mode, inspect generated config/manifest for port, bind host, key
   policy, account auth files, and a fresh fingerprint. Never paste their secrets
   into issues, docs, or commits.
4. Run the built-in `codex_local_access_test` or chat test, then query request logs
   using `codex_local_access_query_request_logs`. Correlate request ID, account,
   API key, status, error category, and upstream transport.
5. If Codex itself does not use the gateway, inspect the target profile's managed
   `auth.json` and `config.toml`, then check the saved takeover backup before any
   manual reset. For WSL, verify the resolved WSL access plan and relay.
6. For account rotation faults, check eligibility, quota reserve, cooldown/health,
   account-model restrictions, and session affinity before changing retry limits.

## Coding Conventions

- TypeScript uses React function components, `async` service wrappers, camelCase
  payloads, and explicit types from `src/types`.
- Rust uses `Result<T, String>` at Tauri boundaries, `async` lifecycle functions,
  serialized camelCase model fields, and structured helper functions in the module.
- Keep persistence changes backward compatible: all new serialized fields need a
  `serde(default)` migration story.
- Keep secrets out of logs. Redact proxy URLs and API keys; generated runtime files
  are local artifacts, not test fixtures.
- Add focused tests next to the behavior. Existing coverage is concentrated in the
  bottom `#[cfg(test)]` module of `codex_local_access.rs`, the Go sidecar tests,
  and `tests/codexLocalAccessAccounts.test.ts`.

## Completed And Next Work

Completed: repository import, source map, service-flow review, and this handoff
document.

Before implementing a change, identify whether it belongs to the UI contract, the
generated sidecar contract, or both. Test both the OpenAI-compatible request path
and Codex profile takeover/restore whenever the change affects activation, keys,
base URLs, models, routing, or credentials.
