# Claude Desktop local Code continuity

Claude Desktop stores its Code sidebar records by account and organization,
while CLI transcripts remain in a shared local pool. Switching Desktop login
can therefore hide a conversation or show an older active branch even when the
transcript itself is still on disk. This feature adds an optional conversation
handoff to Cockpit's existing Desktop OAuth switch flow.

## User flow

Select the destination account using its existing switch action. Identify the
current source account if Cockpit cannot confirm it; the dialog automatically
previews local conversations from every saved, initialized namespace. One
**Handoff and switch** action normally closes Claude, backs up and publishes the
destination sidebar, restores the saved destination login through Cockpit's
existing switcher, and reopens Claude. **Switch account only** remains available.

Progress names the current stage and shows elapsed time and write counts. A
complete, warning-free receipt displays a prominent success status, destination
account and transferred counts, and a **Done** action. Partial writes, restart
failures, unresolved records and warnings retain their recovery/detail state;
a progress event alone cannot display success.

## Scope

The adapter currently supports the default Claude Desktop profile on macOS and
explicitly reviewed Desktop versions. See [the 2.26454.0 record contract](claude-desktop-2.26454.0-static-review.md)
for the most recent review. Unknown or mid-operation version changes stop the
handoff. Other platforms can still use the existing account-only switch.

Only local Code records and their local history references are transferred.
Cloud Chat, Cowork, cloud Artifact ownership, remote sessions and unsupported
execution state are outside this adapter. A divergent worktree that needs a new
Desktop ID requires a native fork; it is reported as unresolved. Missing active
transcripts and other unresolved records block publication rather than producing
a partial success. Missing prior history references produce explicit warnings.

No message is replayed, prompt submitted or transcript rewritten by the handoff.
Quota settings and the source namespace are unchanged. Authentication belongs to
the existing Cockpit profile switcher, not the metadata engine.

## Catalog and branches

The Rust engine collects all saved account/organization namespaces. For each
logical conversation it retains every distinct active CLI transcript. A later
snapshot of the same transcript refreshes its local conversation state; an
existing destination's different active transcript is never silently replaced.
Additional branches receive stable Desktop row IDs recorded in a journaled
catalog. The newest branch uses the plain title and the others show a preserved
branch suffix. Parent navigation is projected onto the corresponding branch.

Destination-only preferences and grants stay with the destination. New imports
use normal permission defaults rather than copying the source account's tool or
connector grants. Changed working directories reset grants. Source queued starts and quit dispatch are not imported. Existing destination
work remains intact; moving its workspace while that work is queued is unresolved.
Ordinary quota-pause error state can be cleared. Native context-recovery state
is validated before applying the reviewed restart semantics; active or unknown
recovery descriptors are unresolved.

## Transaction and recovery

React communicates with Rust through the existing Tauri command boundary. The
coordinator shares an operation lock with account/profile injection. Preview
uses the identity index without repairing profiles or reading authentication
stores. Approval binds source/target identities, the saved namespace roster and
Desktop version; applying rebuilds the plan after normal shutdown.

Before the first publication and at commit, the engine hashes the input records
and referenced transcripts. Per-write directory/file identity, size and timestamp
witnesses prevent replacement or concurrent-write races without repeatedly
hashing the entire transcript pool. Files are accessed through anchored,
non-following directory handles. No process is force-killed.

A write-ahead journal records preimages and expected published images. Required
backups live in the central backup module's durable transaction-recovery area,
which ordinary behavior-backup pruning and relocation cannot remove. Conditional
recovery refuses to overwrite later edits or replacement files. Verified later
undo generations are tracked so earlier recoveries remain possible without
accepting unrelated equal-byte replacements.

Data commit and login/relaunch are separate outcomes. If the metadata committed
but account restoration or startup fails, the applied receipt remains available;
the UI does not claim that the full switch completed. Recovery leaves Claude
closed and restores only verified images. Backups contain private conversation
metadata and must remain local.

## Validation

The default native suite uses temporary synthetic roots. It covers multi-account
catalog collection, branch preservation, both transfer directions, pointer
rollover, permission isolation, source preservation, source/destination recovery
validation, drift, symlinks, interrupted writes, conditional undo and lifecycle
process classification. Workstation-specific live acceptance scripts are not
part of the contributed test suite.

The browser fixture uses the actual React component with synthetic Tauri IPC.
It checks automatic preview, reselecting the same source, duplicate clicks,
visible progress, incomplete/start-failed receipts, prominent completion,
keyboard focus, localization, reduced motion and small viewports. It uses an
isolated headless browser and never accesses accounts or Desktop data.

```sh
npm ci
npm run typecheck
npm test
cargo test --locked --package cockpit-tools --lib claude_session_handoff
cargo test --locked --package cockpit-tools --lib modules::backup_storage::tests
COCKPIT_PLAYWRIGHT_MODULE=/path/to/playwright \
PLAYWRIGHT_CHROMIUM_EXECUTABLE=/path/to/chromium \
node tests/claude-handoff/continuity-flow.spec.cjs
```

The browser harness starts its own localhost Vite fixture. No personal browser
profile or external Node runtime is needed by the installed product. Native
builds use the repository's usual Tauri build process.

Synthetic checks and static vendor inspection do not prove authenticated
Desktop continuation. Real acceptance should independently verify the active
account, latest visible content, all expected branches and source checksums,
then add a bounded test turn and repeat through a return switch. During active
user work, defer this acceptance rather than closing or switching Claude.
