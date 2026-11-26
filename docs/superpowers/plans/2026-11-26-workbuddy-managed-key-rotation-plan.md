# WorkBuddy managed-key rotation implementation plan

Approved design: [managed-key rotation design](../specs/2026-11-26-workbuddy-managed-key-rotation-design.md)

1. Integrate upstream into the still-open PR branch, preserving both upstream and feature UI behavior. Do not force-push.
2. Add a backend rotation eligibility guard used by both named and legacy-primary rotation. Add deterministic command-level regression fixtures using only temporary storage and in-memory synthetic accounts.
3. In parallel, restrict the frontend action and handler, test the production component/handler, and add the one explanatory string in all 18 languages. Each worker owns disjoint files.
4. Document the hash data flows and their security boundaries without dismissing CodeQL #579 or asserting an unavailable data-flow trace.
5. Review the final diff and run focused Rust tests, the related WorkBuddy/native regressions, frontend tests, locales, typecheck and production build. Check no test invoked a real model and no generated credential or profile was staged.
6. Push the reviewed commits to the existing feature branch; verify the PR head and check status; add a precise follow-up explaining the lifecycle fix and requesting security review of the fingerprint use. Do not claim all CI green until verified.
