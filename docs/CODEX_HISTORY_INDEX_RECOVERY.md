# Codex history index recovery

The client can display a chat while `thread/revert` and `thread/fork` fail with
`durable rollout shrank before projection`. These operations run in the client's
local app-server before any Responses inference request. OAuth and gateway
session-ID settings do not repair this local storage condition.

## Supported recovery

Use **Session management → Repair chat index**. The same dedicated
entry is available from the launch preview. Select the affected conversation
and instance, preview the result, completely exit that instance's client and
related CLI processes, then preview again and confirm repair.

The operation follows the selected conversation's physical rollout ancestry.
It supports plain JSONL and Zstandard-compressed rollouts. A projection is eligible
only when its saved byte cursor exceeds the decoded source length and complete,
contiguous durable ordinals still cover the saved projection ordinal. It does not
attempt to reconstruct missing messages or repair damaged source files.

Recovery saves a typed, checksummed backup of the affected rows, then atomically
invalidates those physical rollout IDs in the four derived history tables. The
official client regenerates the indexes when the conversation is reopened.
The result reports index invalidation, not successful model generation.

## Preservation and concurrency

- Rollout bytes, ciphertext, `history_base`, account credentials, provider settings
  and the primary thread database remain unchanged.
- A physical rollout ID may differ from the logical conversation ID after a revert;
  the canonical filename and ancestry determine the correct cache rows.
- An old ancestor byte cutoff need not exactly match an ordinal's line boundary.
  Recovery preserves it and validates the bounds used by the client.
- Unknown schemas, missing ancestors, truncated compression, incomplete records,
  missing ordinals, changed source files or changed cached rows stop recovery.
- The client/CLI process guard runs again around the write transaction. Any failure
  before commit rolls back that profile's cache changes and retains the backup.
- Different profiles use separate transactions. If a later profile changes after
  preview, the error reports the number of projections already reset and a backup
  location; completed repairs are not silently reverted over newer client state.

The backup directory is returned in `backupDirs`; each repair contains
`history-projection.sqlite` and its SHA256 file. It contains only the affected
derived rows and audit metadata, not a copy of the full history database. Restoring
it blindly after the client has rebuilt newer rows would reintroduce stale state.

## Prevention

Existing history remains append-only and byte-addressed. Provider normalization,
wire item compatibility and signing adaptation belong outside existing rollout
files. Import and synchronization must preserve prefix/lineage checks and never
replace a newer history with an older snapshot. The chat index repair is explicit
maintenance on a blocking worker, with bounded scanning and progress reporting;
automatic launch and account-switch checks do not perform full history scans.
