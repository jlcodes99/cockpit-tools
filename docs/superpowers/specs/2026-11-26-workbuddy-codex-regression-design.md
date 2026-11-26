# WorkBuddy Codex integration regression repair (#3298)

## Intent and approved scope
Restore the existing stateless, text-only native WorkBuddy ACP MVP after #3282. The user approved the minimal repair, not a complete Codex agent integration.

## Approaches
1. **Selected: repair provider dispatch and capability boundaries.** Recognize linked WorkBuddy accounts before generic quota, client credential projection and HTTP upstream processing. Keep the existing ACP executor and HTTP security checks.
2. Implement full Codex tools/context support: not selected; a separate protocol/capability project requiring broader validation.
3. Fabricate an OpenAI key or relax URL checks globally: rejected; hides unsupported behavior and weakens credential/URL boundaries.

## Design
- Treat WorkBuddy references as native ACP credentials, never as relay/OpenAI keys. Account refresh must not request relay quota or manufacture Codex quota windows; credits remain on the WorkBuddy page.
- Dispatch only validated WorkBuddy references to the ACP executor with the exact internal marker. Ordinary accounts and HTTP upstream validation remain unchanged.
- Direct Codex login/configuration is unsupported for this text-only MVP. Fail before credential projection with an actionable explanation; do not persist WorkBuddy secrets or a fake key.
- Preserve errors for unsupported tools, images and stateful history; do not claim full Codex CLI compatibility.
- Add deterministic dummy-account/local-fixture regression tests, avoiding real credentials and live WorkBuddy requests.

## Constraints and verification boundaries
Base the PR on upstream main in an isolated worktree. Do not include earlier local features. Verify focused Rust tests, relevant frontend checks if changed, formatting/diff and final PR contents. This macOS environment cannot establish Windows installed-build or real-account success; disclose that in the PR.

## Implementation plan
1. Trace and patch WorkBuddy quota classification/refresh, preserving ordinary API and OAuth behavior; add tests.
2. Patch native route construction so linked WorkBuddy reaches ACP without entering HTTP-only transport setup; add route/security regressions.
3. Guard direct client projection and expose clear unsupported capability messaging, with tests.
4. Run focused tests and checks, independently review the complete diff, commit and push the repair-only branch to fx-an/cockpit-tools, then open a PR against upstream main referencing #3298.

## Review refinements
- Validate both direct and provider-gateway client bindings before launch proxy preparation, stopping an existing client, or profile mutation. API-service bindings remain allowed.
- Reject `/responses/compact` explicitly at the ACP executor entry; it must not silently become an ordinary conversation.
