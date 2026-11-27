# PR #3312 CI repair after upstream sync

Status: approved by the user on 2026-11-27; user subsequently required merging latest upstream first.

## Baseline and order

- Existing published feature/fix head: `4750d3c4`.
- Latest release page currently identifies v1.5.2. Fetch verified upstream main `f74f80f1`.
- First merged upstream non-rewritingly as `358e4c3ce`; 42 upstream files merged without conflicts. Preserve the native WorkBuddy bridge and managed-key lifecycle repair.
- Reassess the failures on that merged baseline before implementing the following minimal repair.

## Approved approach

Synchronize the existing desktop OAuth scope semantics into cockpit-core, rather than removing shared tests or redesigning module ownership. Include the optional response/model fields, scope interpretation, refresh fallback, import mapping and scope-only persistence. Preserve compatibility with old serialized tokens and current refresh-token, id-token, project/session and retry behavior. Add behavioral coverage and a narrowly scoped drift guard for the two currently equivalent OAuth/token implementations. Do not copy unrelated account/import architecture.

A single-source production-module refactor is a larger follow-up, not required for this CI repair. No new runtime dependencies, UI behavior or real provider/model calls are needed.

## Security boundary

Keep the JavaScript/TypeScript and Rust CodeQL analysis jobs and existing SHA-256 ownership/change checks. The checked upstream snapshots contain no Go source/module and their workflow matches the feature workflow; do not invent a Go job, empty SARIF or renamed category to satisfy an old Go baseline. Ask maintainers to reconcile that baseline using actual repository analysis/configuration evidence.

The public path viewer still does not expose the complete #579 flow to this account. Source review identifies opaque-token generation, key ownership and file-change fingerprints, not a confirmed human-password verifier. Do not label the alert a confirmed false positive, dismiss it, suppress the rule, blindly change algorithms or weaken revocation. Request the actual source/sink trace before any separate security-design change.

## Acceptance

The full cockpit-core CI command must compile and pass, not merely the desktop app package. Run relevant desktop OAuth/WorkBuddy and merged-upstream regressions, frontend tests/type/locale validation and production build as applicable. Review the final diff, verify PR state, publish without force and report the exact new CI state. An unresolved maintainer-only CodeQL gate must remain explicitly disclosed, not called all-green.
