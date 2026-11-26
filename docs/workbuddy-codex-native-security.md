# WorkBuddy native bridge: hash-use and managed-key security review

Scope: PR #3312, initially reviewed at `0ecc47c3`; the lifecycle follow-up also integrates upstream v1.5.2 (`b418133c`). This is a source review, not a claim that every installed client, filesystem race or external CLI has been security-validated.

## CodeQL alert #579

The public PR bot comment links to alert #579 and reports **Use of a broken or weak cryptographic hashing algorithm on sensitive data**, treating the input as a password and the digest as SHA-256. The reported include location is not itself a cryptographic operation. The alert detail page was unavailable to the reviewing contributor, and the public **Show paths** dialog did not load its trace. Accordingly, this document does **not** identify a specific CodeQL source-to-sink path, dismiss the alert, or claim it is a formally confirmed false positive.

The following actual data flows are relevant:

| Data flow | Purpose | Storage/exposure | Security assessment |
| --- | --- | --- | --- |
| WorkBuddy access token and pinned source identity → generation digest | Reject continuation after a credential/source change | Internal selection and actor registry; not serialized into profiles or public response IDs | Change detection, not a password verifier. No assumption that every imported token has a particular entropy count is needed to justify this internal comparison. |
| Generated local API key → `local_key_sha256` | Confirm that a mutable profile marker still refers to the same dedicated key before revocation | Secret-written marker in the independent profile directory | Local token identity, not human-password storage. Normal creation generates 32 random alphanumeric characters; direct modification of trusted local configuration is outside that generation guarantee. |
| Generated config/auth contents → file digests | Refuse cleanup of files changed after creation | Same private marker; config/auth files already hold the scoped local key needed by Codex | File-change detection, not encryption or a password KDF. |
| Canonical tool definitions/history/results → digests | Stable tool aliases, catalog identity, bounded response cache and result deduplication | Tool aliases or internal actor state depending on the digest | Not an authentication credential. Public IDs are references and still require caller/thread ownership. |

Actual HTTP bearer authentication compares the supplied original key to the configured original key and then enforces enabled state, budget and dedicated scope. None of the profile or generation digests is accepted as a bearer token.

SHA-256 is unsuitable as a fast unsalted verifier for low-entropy human passwords. That is not the normal purpose of these flows. Replacing it with SHA-512, adding cosmetic renaming, or disabling the query would not be a valid security fix. A maintainer should review the actual alert trace against these purposes before deciding whether to dismiss it.

## Confirmed separate lifecycle defect

The generic named-key command and legacy primary-key command could rotate a dedicated local key without updating its independent profile marker. Restore then encountered a key-fingerprint ownership mismatch before disabling the rotated key. The old bearer was invalidated by rotation, but the replacement key could remain enabled while profile revocation failed visibly. This is not password cracking or proof of remote privilege escalation.

The approved minimum correction rejects managed-key rotation at both backend entry points before mutation and disables the frontend action with an accessible explanation. Changing a managed key uses the existing revoke-and-create lifecycle. Ordinary-key rotation remains supported. The original fingerprint ownership checks and permanent revocation tombstones remain necessary; removing them would weaken forged-marker protection.

## Boundaries retained

- Config/auth contain only the scoped Cockpit key, not the WorkBuddy access token. On Unix the profile directory is mode 0700 and secret files are written using the existing secret writer.
- The marker is not a MAC or an unforgeable signature against a process with the same directory-write privileges. Hashing does not make same-user filesystem modification impossible.
- Symlink/ownership/file-change checks are path-based. This feature does not claim a dirfd-anchored defense against every check-to-use race, and it does not delete the profile directory or user session data.
- The external installed WorkBuddy CLI is not treated as an OS security sandbox. Codex retains the configured tool sandbox and approval controls.
- This follow-up requires deterministic local tests only. It neither uploads a real project nor performs additional WorkBuddy inference. It does not resolve or close the security alert on the maintainer's behalf.
