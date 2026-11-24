# Updater Test Harness Fix Design

## Intent

Restore the updater regression test so it executes the same update-error message helper that the application uses. The current VM harness omits that helper and fails with `ReferenceError: resolveUpdateApplyFailureMessage is not defined` before it can assert the intended behavior.

## Scope

- Add a standalone fix branch based on the upstream API-key base.
- Import the existing `resolveUpdateApplyFailureMessage` helper into the updater test.
- Inject that helper into the VM context used to execute the App callbacks.
- Do not change production updater behavior, OAuth code, or the existing OAuth pull request.

## Alternatives

1. Inject the existing helper into the test VM context (selected): smallest change and keeps the test coupled to production behavior.
2. Copy the helper into the test: rejected because it duplicates behavior and can drift.
3. Rebuild the VM harness around automatic module loading: rejected because it expands the fix beyond the isolated regression.

## Verification

Run the updater test suite and the complete TypeScript test suite. Confirm the previously failing install/relaunch test passes and the branch diff contains only the test harness, this design note, and the resulting fix metadata.
