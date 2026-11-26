# WorkBuddy managed-key rotation: approved minimal design

## Intent and approval

The user approved the read-only audit and then this minimal remediation on 2026-11-26. Preserve the existing dedicated-profile lifecycle: revoke a managed profile, then create a new one to change its local key. Do not add in-place managed-key rotation.

## Finding

Generic named-key rotation and the legacy primary-key rotation can replace a WorkBuddy dedicated key without updating its profile marker. Restore then fails its key-fingerprint ownership check before disabling the new key. This is a lifecycle mismatch, not SHA-256 password cracking. The code-path finding requires a deterministic regression; no real WorkBuddy inference is needed.

## Approaches

1. **Approved: reject generic rotation of managed keys.** Guard both backend entry points, disable the frontend action with an accessible localized explanation, preserve all ownership checks, and retain ordinary-key rotation. This is the smallest change consistent with immutable dedicated grants.
2. **Not selected: implement atomic dedicated rotation.** This would require coordinating key storage, profile files, markers, live sessions and rollback across failures. It expands the feature rather than repairing its current lifecycle contract.

## Security boundary

Keep SHA-256 for credential-generation fingerprints, local high-entropy token identity and file-change detection. These are not user-password storage or password-verifier functions. Do not remove ownership checks, rename code to hide a finding, replace SHA-256 with SHA-512, suppress CodeQL, or claim the private alert is a confirmed false positive without its source-to-sink trace. Public alert #579 points at the include site; its public path viewer was still loading during investigation. Add an honest security rationale for maintainer review rather than resolving or dismissing the alert ourselves.

The generated key uses 32 random alphanumeric characters. Direct modification of trusted local configuration is not an entropy guarantee for every loaded key. A file digest is not a MAC or an operating-system boundary against a same-user writer. Existing path-based cleanup does not claim dirfd-based TOCTOU protection.

## Changes and acceptance

- One shared backend guard, invoked before mutation by named and legacy-primary rotation. Return `WORKBUDDY_DELEGATE_KEY_ROTATION_UNSUPPORTED` for a managed key.
- Backend regressions execute both entry points: rejected managed rotation changes neither key nor persisted ownership; restore can still revoke; normal keys still rotate.
- Frontend disables the managed action, protects direct handler invocation, and maps the backend error to the same localized explanation. All 18 locales receive `workbuddy.nativeCodex.managedKeyRotation`.
- Keep modified-file preservation, forged-marker rejection and revocation tombstones intact.
- Integrate upstream v1.5.2 without losing its unrelated changes or rewriting published branch history. The existing PR #3312 was verified open with a one-file merge conflict; that conflict is resolved by keeping the upstream icon action and the separate native bridge action.
- Run focused and related Rust tests, production frontend regressions, locale checks, typecheck and production frontend build. Publish only after final diff review. No additional real model calls.
