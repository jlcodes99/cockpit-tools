# WorkBuddy native tool bridge for Codex CLI

This is an **opt-in Native API Service integration**, separate from the existing WorkBuddy text-only account link. WorkBuddy provides model inference; **Codex remains responsible for local file access, command execution, approvals and sandboxing**. The bridge does not execute project tools itself.

## Set up

1. Install WorkBuddy and import a valid **personal** WorkBuddy account into Cockpit. An enterprise account is rejected until native CLI tenant pinning can be verified.
2. Create and enable the **Native Rust** API service in Cockpit. This feature does not switch a sidecar service automatically.
3. On the WorkBuddy accounts page, open **Codex 原生工具桥**, select the source, read its current model catalog, and choose an explicit model and an effort supported by that exact model. Automatic/default model aliases are not accepted.
4. Create a dedicated key and an independent Codex configuration. Copy the displayed launch command, for example:

   ```sh
   CODEX_HOME='/path/to/generated/wb-native-profile' codex
   ```

   Keep using that independent `CODEX_HOME` for this connection. The ordinary Codex configuration is not overwritten. The profile contains only a scoped **Cockpit local key**, not the WorkBuddy access token.
5. Start a new Codex conversation in your project. Keep Codex's own approval and sandbox settings appropriate for that project. Do not assume the WorkBuddy CLI's private working directory is an operating-system security sandbox.

Each profile permits only its selected WorkBuddy source and public model. There is no account-pool fallback, automatic provider reselection, or fallback to the old text-only bridge. If the service port changes, revoke the old profile and create a new one. Dedicated local keys cannot be reset through the generic API Service key controls: revoke the managed profile and create a new one instead. This restriction does not prevent WorkBuddy itself from refreshing its upstream access token.

## Scope and limits

- Uses HTTP Responses. WebSockets and hosted web search are explicitly disabled in the generated production profile.
- Supports declared local function tools, a single layer of namespaces, and freeform text custom tools. Hosted tools, grammar-constrained custom tools, image input, background/stored Responses, compact, forced tool choice and unsupported schemas fail explicitly rather than silently losing constraints.
- The native WorkBuddy session is retained between tool handoffs and subsequent user turns. The original pending tool callback receives the actual Codex tool result at most once.
- Service/process restart, source credential changes, expired sessions, changed tools or model/effort mismatches can invalidate a live conversation. Start a new conversation after an explicit expired/conflict error; the bridge never reconstructs a pending tool execution by replaying history to another account.
- Resource limits include eight total actors, two per caller/source, eight pending calls per actor, bounded frames/history/results, and bounded startup/turn/idle lifetimes. Idle completed sessions expire after three minutes; actors expire absolutely after thirty minutes.
- Codex 0.172 can warn that the custom public model has no built-in metadata and use its generic capability metadata. This is not account/provider fallback; the actual tested route remains pinned. The acceptance task passed with that warning; richer Codex model-metadata integration is not claimed.
- Usage is not invented. Some CLI versions report usage only in ACP notifications that this initial adapter does not map to Responses usage; those responses omit usage. See WorkBuddy's account page for the authoritative credit information.
- Direct WorkBuddy account injection into Codex and Provider Gateway binding remain unsupported. The older **接入 Codex（纯文本）** action retains its existing text-only semantics.

## Revoke and restore

The native bridge dialog lists all managed profiles, including those whose WorkBuddy source has been removed. Use **撤销专属 Key 并恢复此配置** to revoke a profile.

Revocation is permanent for that key identity: unrelated delayed settings/usage saves cannot re-enable it. Create a new profile/key to authorize a new connection. Unmodified owned configuration/authentication files are removed, while Codex sessions and other user data are preserved. If you changed the generated files, the matching key is still revoked but the changed files are retained; the dialog explains that cleanup was not completed and lets you retry after inspection.

## Security review

See [the hash-use and managed-key security review](workbuddy-codex-native-security.md) for the fingerprint data flows, the managed-key rotation restriction, and the remaining filesystem/CodeQL review boundaries. A successful functional test run is not a substitute for resolving a security finding with its actual data-flow evidence.

## Verification boundary

The deterministic suite covers the real Codex 0.172 tool declaration shapes and the installed WorkBuddy delegate callback shape, along with ownership, schema checking, duplicate results, cancellation, child-process cleanup and revocation races. These tests are not a substitute for a real Codex read/edit/test acceptance run. Platform/version claims must be limited to the actual acceptance evidence included in the PR; Windows and other installed CLI versions require separate validation.
