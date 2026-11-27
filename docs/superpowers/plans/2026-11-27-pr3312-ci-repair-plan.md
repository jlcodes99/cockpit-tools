# PR #3312 CI repair implementation plan

Approved design: [CI repair design](../specs/2026-11-27-pr3312-ci-repair-design.md).

1. Completed first: fetch latest main with sufficient ancestry; non-force merge `f74f80f1` into the published feature branch as `358e4c3ce`; verify no conflicts and retain the old feature/fix history.
2. Reproduce the unchanged cockpit-core compile failure on that baseline using the real CI package/test selection. Use local synthetic fixtures only.
3. Synchronize core OAuth and TokenData scope behavior, retaining the explicit shared-test path. Extend shared behavioral tests and add a narrow production-source parity guard without printing source/credential values.
4. Preserve scope in four core import paths and in account/wakeup refresh persistence, including scope-only changes. Add actual mapping and persistence regressions; preserve other account fields and existing retries.
5. Run full core tests, both packages' OAuth coverage, WorkBuddy and merged-upstream focused regressions; verify frontend tests, types, all locales and a production build after Cargo work. Avoid simultaneous Cargo writers to the shared target and do not invoke real WorkBuddy inference.
6. Independently review the final diff and security boundary. Leave CodeQL enabled and document the absent-Go baseline / unavailable #579 trace. Refresh PR state, commit and non-force push to the appropriate existing branch, post accurate verification details, and check the resulting CI jobs. Distinguish remaining maintainer gates and canceled jobs from successful checks.
