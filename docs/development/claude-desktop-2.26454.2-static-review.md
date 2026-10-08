# Claude Desktop 2.26454.2 local record contract

Observed 2026-10-08 on the installed macOS application. This patch release
changed the application archive and chunk names, but the persisted record
projection has the same 169 fields and the identical function SHA-256
`56462961252c38c52a9b4ee52d80048401ca194aa36ef5e8ceff5499752e8643`
as the reviewed 2.26454.0 release.

The twelve bounded methods used by the existing native probe are byte-identical:
the six recovery helpers, recovery adoption/set, namespace selection, session
file path, local record loader, and parked namespace path. The updated probe
executed the actual 2.26454.2 method bodies and passed all fifteen recovery and
three loader/path cases. No user data or credentials were read by these probes.

The archive SHA-256 is
`fca80b3f813a6c06f414240d126a39c538ae27e3edb49d433fd465fd5b495a8f`.
The main module is `.vite/build/index.chunk-BuWAu4qX.js`, SHA-256
`8f2d3b9f164c9ccb19fe384b30c4a0971fa75c1587c8ba3cade17e240d276921`.
The record projection module is `.vite/build/index.chunk-DWp6lQwJ.js`, SHA-256
`0c1f60f5900948b471a7f6bd1ed211ec8436308f050acb807b6eed94f5b960e7`.

The ordinary local adapter and limitations from the 2.26454.0 review apply
unchanged. This review permits exactly 2.26454.2; it does not permit arbitrary
future patch releases or a version change during an approved transaction.
The fixture normalizer, filesystem, lease/resume, and scheduler remain mocked.
Actual installation and account handoff acceptance are separate checks.

Reproduce structural inspection with
`node scripts/inspect_claude_continuity_contract.mjs`.
The installed-source probe, method comparison and receipts are retained in
BR's dated local PR-preparation evidence directory.

## Cockpit 1.3.66 data-root compatibility

Upstream now exposes `.cockpit_tools` as a verified alias of the established
`.antigravity_cockpit` store. The handoff's strict no-follow checks originally
rejected this legitimate root alias on a subsequent process launch, preventing
account-list loading, journal access and recovery-path validation. This is
independent of the Desktop patch-version check.

The adapter uses upstream `without_compatibility_alias` for the index, handoff
state and transaction recovery paths. The helper accepts only the standard
root alias pointing to the verified real legacy directory; it does not resolve
nested links or custom roots. File, ancestor, identity and content checks remain
in place. Journal paths match the existing canonical legacy transactions.

A bounded actual GUI switch using the documented explicit canonical data root
completed on 2026-10-08: two new rows, three updated rows, all 175 referenced
active CLI heads present in the target, no stale activity/turn counters, and all
174 source rows unchanged. Four transcript files grew after Claude restarted;
their original complete byte prefixes were preserved. This evidence covers
saved initialized local Code namespaces, not cloud conversations or artifacts.
The permanent alias correction passed 132 handoff and 19 data-path tests.
