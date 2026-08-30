# CLIProxyAPI upstream sync

The bundled source under `third_party/CLIProxyAPI` is synchronized to the
`CLIProxyAPI` `v7.2.140` source tree with Cockpit compatibility shims. The outer
sidecar module declares the same upstream version and continues to use the local
`replace` target.

## Codex sync scope

- Upstream repository: `https://github.com/router-for-me/CLIProxyAPI`
- Synchronized module baseline: `v7.2.140` (`a7e3596b`)
- Codex Live baseline: the upstream Live implementation plus later capability,
  client-secret, WebSocket, media-relay, and TCP-proxy commits
- Cockpit integration routes: `POST /v1/live`, `GET /v1/live/:call_id`,
  `POST /v1/realtime/calls`, and `GET /v1/realtime/calls/:call_id`

Cockpit uses the upstream Codex executor, Responses WebSocket, model catalog,
auth scheduler, Live request/sideband/realtime implementation, and supporting
config/proxy utilities. The outer relay keeps API-key account scoping and rejects
provider-gateway profiles because Codex Live requires a ChatGPT OAuth credential.

## Update procedure

1. Compare the next upstream tag against `v7.2.140`, beginning with
   `internal/runtime/executor/codex*`, `internal/client/codex`, `sdk/cliproxy/auth`,
   `internal/config`, `internal/registry`, and `sdk/proxyutil`.
2. Port the complete dependency closure instead of copying a single changed file.
3. Preserve Cockpit-specific executor identity, token ownership, account routing,
   quota, and request-policy compatibility shims.
4. Run the focused Codex executor/Live tests, sidecar tests, and a sidecar build.
