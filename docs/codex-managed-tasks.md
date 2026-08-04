# Cockpit Codex managed tasks

Cockpit managed tasks are an explicit opt-in workflow for long-running Codex CLI work. Cockpit starts and owns one `codex exec --json` process, streams normalized status, and keeps additional tasks in a persistent FIFO queue.

## Safety boundary

- Automatic account switching is available only for processes launched by this workflow. Cockpit does not take over arbitrary Codex Desktop or terminal sessions.
- A quota warning, HTTP 429, proxy event, or rollout-file message moves a task to `draining`; it does not stop the process or switch credentials.
- Cockpit switches only after an authoritative `turn.failed` usage-limit terminal event, or after a structured usage-limit error followed by a non-zero process exit. The old process is fully reaped first.
- Successful completion, authentication failures, network failures, context-window errors, model-capacity failures, tool errors, and user cancellation never trigger account rotation.
- The workflow does not invite, remove, or replace Team/Business members or seats.

## Isolation and privacy

Each task has a fixed private runtime directory under the Cockpit application data directory:

```text
codex-managed-tasks/<task-id>/home
codex-managed-tasks/<task-id>/workspace-meta
```

The same task-level `CODEX_HOME` is used before and after an account switch, which preserves the Codex thread history and allows `resume <threadId>`. Only the authentication projection inside that task directory changes. Cockpit does not modify the global `~/.codex/auth.json`, the default Codex account, WSL configuration, or another Codex process.

Managed executions explicitly use `--ask-for-approval never` before the `exec` subcommand and `--sandbox workspace-write` for the turn, including resume attempts. The non-interactive process therefore executes actions allowed inside the workspace, immediately rejects actions outside the sandbox instead of waiting for an unavailable prompt, and never enables the dangerous full-access bypass.

On native Windows, each isolated task home would otherwise have no Windows sandbox selection and current Codex versions deliberately downgrade workspace-write to read-only. Cockpit therefore supplies the official non-admin fallback `windows.sandbox="unelevated"` for managed turns. It retains restricted-token and ACL boundaries without requiring an unattended UAC prompt. Administrators can still use the stronger elevated setup for their ordinary Codex profiles; Cockpit does not modify that global choice.

The supervisor database stores task configuration, lifecycle state, normalized evidence, masked account references, and bounded redacted errors. It does not store account tokens, raw authentication JSON, stdout transcripts, full stderr, assistant messages, reasoning, tool output, or command output. Codex's own thread records remain in the task-level `CODEX_HOME`.

After each child process exits, Cockpit synchronizes any refreshed OAuth token back to the corresponding Cockpit account and removes the task authentication projection. Thread records are retained so the same thread can be resumed.

## Account policy

Tasks can use the current Cockpit pool or a fixed account allowlist. Selection reuses the existing Cockpit routing strategy, including quota/plan ordering, custom priorities, preferred/backup roles, weights, health state, and model cooldowns. The live pool is reread before every switch for `cockpit_pool` tasks.

Only ready OAuth/ChatGPT accounts with injectable Codex CLI credentials are eligible. API-key providers, Web Sessions, Agent Identity accounts, pending OAuth records, accounts requiring login, freshly confirmed zero-quota accounts, health-blocked accounts, model exclusions, and accounts already attempted by the same task are skipped. Stale or unknown quota is allowed; the real Codex terminal event remains authoritative.

## Crash recovery

On startup, Cockpit checks the recorded PID, executable identity, and process start time. A matching live orphan is never duplicated because stdout cannot be reattached safely; the task enters `needs_attention`. If the process is gone, Cockpit performs a bounded one-shot App Server `initialize` + `initialized` + `thread/read` check using the same task home.

- Last turn completed: mark the task completed.
- Last turn failed from quota: select another eligible account and resume the same thread.
- Last turn was interrupted: automatically resume once with the current account.
- Conflicting, active, missing, or unavailable evidence: enter `needs_attention`.

Automatic crash recovery is bounded to one attempt per task lifecycle. A user can later choose “Resume current account” or “Resume next eligible account.”

## Developer verification

```text
npm ci
npx nx run cockpit-tools:verify
npx nx run cockpit-tools:ci
```

The managed-task workflow runs unit/contract tests on Ubuntu and fake Codex CLI process tests on Windows, Ubuntu, and macOS. CI never reads real Codex credentials and never attempts to consume a real account quota.

Pinned research dependencies are listed in `upstreams.lock.json`. Third-party source is not copied into this repository. Projects without an asserted license are restricted to `metadata_only`.

## Manual real-CLI smoke test

Real testing is intentionally local and opt-in:

1. Open **Codex → Managed tasks** and confirm the runtime card reports an executable official Codex CLI and version.
2. Use a new disposable working directory, not this repository or a business repository.
3. Select an OAuth account that you have already imported and authorized.
4. Create a minimal objective that writes a uniquely named marker file in the disposable directory.
5. Confirm the task uses a task-level `CODEX_HOME`, receives `thread.started` / `turn.started` / `turn.completed`, and reaches `completed`.
6. Only if an account is already genuinely exhausted, use it as the initial account and include a healthy account in scope. Do not intentionally consume quota merely to manufacture the condition.
7. Confirm the resumed process uses the original thread ID and completes the same objective. Then confirm task authentication files were removed and the global Codex auth/config hashes did not change.

Do not copy tokens, raw transcripts, or complete stderr into a test report. Record only the CLI version, masked account ID, thread ID, switch count, final status, and pass/fail checks.

Developers can run the same guarded smoke path used during implementation. It is ignored by ordinary `cargo test`, requires an explicit acknowledgement variable, defaults to at most three switches (four real accounts total), and always removes its generated temp root:

```powershell
$env:COCKPIT_MANAGED_REAL_SMOKE = "1"
cargo test -p cockpit-tools real_managed_cli_smoke_uses_isolated_cockpit_projection --lib -- --ignored --nocapture --test-threads=1
```

On Windows, avoid installing the npm CLI beneath an unusually long global prefix. The native sandbox helper may exceed the Windows process-launch path limit even though the file exists, producing `orchestrator_helper_launch_failed` / `os error 3` and a read-only fallback. Install the official CLI under a short trusted prefix, then set Cockpit's Codex CLI path to that installation. Do not work around this condition with the dangerous full-access bypass.
