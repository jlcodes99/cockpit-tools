# Codex host handoff records and native continuity

## Symptom and cause

A native gateway conversation can continue on its original account but fail to
rebuild on another eligible account after a quota error, while a new conversation
works. One reproducible cause is a complete transcript containing these Codex app
host notifications:

```json
{
  "type": "function_call_output",
  "namespace": "codex_app",
  "name": "create_thread",
  "output": "Synthetic host handoff result"
}
```

The second supported name is `send_message_to_thread`. These observed host
envelopes have no `call_id`: they are not results of an ordinary model function
call. The old replay pairing check rejected them as missing tool history. Wire
normalization could also incorrectly give them an unrelated pending call ID.

This is a narrow compatibility fix, not a replacement for the account scheduler,
a quota increase, an encryption bypass, or a guarantee that every old transcript
is portable. Upstream account/model limits still apply.

## Invariants

- Recognize only the exact type, namespace and two names above, with string
  `output` and a completely absent `call_id` field.
- A present null, empty or invalid `call_id` does not qualify for the exemption.
- A same-qualified real function call anywhere in the input makes an ID-less
  notification ambiguous. Existing flattened call spellings are also treated as
  conflicts, not as additional accepted standalone output shapes.
- Keep all host fields, extension metadata and input ordering unchanged. Do not
  invent a call or consume another tool's pending result ID.
- Keep ordinary function/custom-tool pairing, caller isolation, resource checks
  and encrypted-content provenance checks in place.
- Use the existing rebuild/checkpoint path; do not publish candidate ownership
  or a portable checkpoint before successful upstream completion.

Only self-contained, validated history is eligible for the existing fallback.
Opaque reasoning and message ciphertext are preserved, not decrypted, rewritten,
or deleted. Server-only references or incomplete history may still prevent a
safe cross-account rebuild.

## Offline reproduction and regression tests

No real account or external network service is needed. Tests use synthetic input,
synthetic ciphertext and local listeners bound to `127.0.0.1:0`.

```sh
cargo test --locked -p cockpit-tools --lib codex_app_outputs -- --test-threads=1
cargo test --locked -p cockpit-tools --lib codex_wire::calls::tests -- --test-threads=1
cargo test --locked -p cockpit-tools --lib codex_app -- --test-threads=1
cargo test --locked -p cockpit-tools --lib same_qualified_real_calls -- --test-threads=1
cargo test --locked -p cockpit-tools --lib modules::codex_native_api -- --test-threads=1
```

The loopback OAuth fixture first completes a response on A and stores its
checkpoint, then injects `usage_limit_reached` on the next A attempt. It verifies
that the actual retry path sends the full original history to B. Coverage includes
HTTP 429 and an HTTP-200 SSE error containing status 429, both full-input and
cached-previous-response continuation, both host envelope types, real tool pairs,
and synthetic encrypted reasoning items. Assertions compare complete input
arrays, not just selected fields.

Negative tests cover a failed B candidate (no ownership/checkpoint commit) and
missing/ambiguous ordinary tool history (no cross-account retry). Synthetic
ciphertext proves preservation only; it does not prove upstream decryptability.

## Frontend build-path regression

A Windows SUBST drive or linked checkout can give the same React runtime multiple
module identities. That can produce a blank UI with a null hook dispatcher after
a locally built installation. Vite now uses the canonical project root and
dedupes React, React DOM and scheduler. A build guard rejects multiple included
React core module IDs, including aliases of one physical file, while ignoring
CommonJS virtual wrappers and tree-shaken copies.

```sh
npm ci
node --import tsx --test tests/reactSingleton.test.ts
npx tsc -p tsconfig.node.json --noEmit
npm test
npm run build
```

The frontend regression uses disposable synthetic runtime files and a directory
link (a junction on Windows). It requires no live desktop or API account. This
build fix is independent of the native continuation compatibility change.
