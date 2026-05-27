# Codex History Repair Handoff

## Context

The user had CPTools/Codex history damage after CPTools account switch/start triggered Codex thread sync and session visibility repair. Symptoms included slow launch/switch, archived histories reappearing, generated `Documents\Codex\2026-*` project folders showing in Projects, worktrees floating outside their project, and the `Chats` section becoming empty or overfilled during repair attempts.

Use `$codex-history-repair`, `$repo-lessons`, and `$codegraph` for future work here.

## Current Good State

- CPTools launch time is good.
- CPTools automatic thread sync / provider-change session repair was disabled in code.
- Codex Projects list is back to the smaller archived state.
- `Chats` is populated with the intended 8 active raw chats.
- Active chat display names were restored:
  - `Crawl X`
  - `QBO macro close behavior`
  - `Pull compliance files`
  - `FTT assignment package`
  - `Agentic systems setup`
  - `Continue handoff fix`
  - `Hotel options review`
  - `Computer scan and collaboration plan`
- CPTools worktree threads are mapped under:
  `C:\Users\user\Documents\Codex\CPTools\cockpit-tools-codex-lb`

## What Worked

- Reading upstream `openai/codex` was the key unlock.
- `projectless-thread-ids` alone does not make chats show.
- For an active chat, Codex expects:
  - sqlite `threads.archived = 0`
  - sqlite `threads.archived_at = NULL`
  - sqlite `threads.rollout_path` under `C:\Users\user\.codex\sessions\YYYY\MM\DD`
  - rollout file physically moved out of `archived_sessions`
- Proper unarchive behavior is in:
  `codex-rs/thread-store/src/local/unarchive_thread.rs`
- Restoring names worked by setting sqlite `threads.title` to concise titles that differ from `first_user_message`, and updating `session_index.jsonl`.

## What Did Not Work

- Only editing `.codex-global-state.json` did not restore sidebar state.
- Only setting `projectless-thread-ids` did not restore `Chats`.
- Only flipping sqlite `archived`/`archived_at` did not restore active chats while rollout files remained in `archived_sessions`.
- Deleting generated `Documents\Codex\2026-*` sqlite rows was too destructive; it made archived/searchable raw chats disappear.
- Expanding `electron-saved-workspace-roots` / `project-order` back to older 37/41/81-root snapshots unarchived Projects the user did not want.
- Keeping archived IDs inside `projectless-thread-ids` overfilled Chats.

## Important Backups

- Before proper raw-chat unarchive:
  `C:\Users\user\.codex\backup-20260527-145840-proper-unarchive-raw-chats`
- Before trimming active chats back down:
  `C:\Users\user\.codex\backup-20260527-150021-trim-raw-chats-to-previous-active`
- Before restoring chat display names:
  `C:\Users\user\.codex\backup-20260527-150222-restore-chat-display-names`
- Earlier broad backup before derived-cache deletion:
  `C:\Users\user\.codex\backup-20260527-140400-delete-generated-cache-map-worktrees`

## Current Active Chat IDs

These are the intended active `Chats` IDs:

```text
019e17eb-2387-77d3-bf28-128cb3a4b934
019e18f4-a020-74a0-bac3-977636011768
019e222c-28f4-7ea1-a815-bb0702df400d
019e2294-2198-7792-a3c6-ee23ec717551
019e4227-efc1-7201-b990-8762258c9ef8
019e4c3c-4940-7c62-99e7-6209472feaae
019e4d13-2182-7a31-a268-3e7f8674812e
019e5ad4-584f-7343-8b81-c2d8b2906882
```

## Repair Rules For Next Time

1. Do not edit history while Codex Desktop is open unless checking live state only. The app can rewrite state.
2. Before changing anything, back up:
   - `.codex-global-state.json`
   - `state_5.sqlite`
   - `state_5.sqlite-wal`
   - `state_5.sqlite-shm`
   - `session_index.jsonl`
   - any rollout files being moved
3. To archive a chat, move its rollout from `sessions\YYYY\MM\DD` to `archived_sessions`, then set sqlite archived fields.
4. To unarchive a chat, move its rollout from `archived_sessions` back to `sessions\YYYY\MM\DD`, then clear sqlite archived fields.
5. Keep `projectless-thread-ids` containing only active chat IDs.
6. Do not put archived IDs in `projectless-thread-ids`.
7. Do not restore old full `project-order` snapshots unless the user explicitly wants all Projects unarchived.

## Verification Commands

```powershell
python - <<'PY'
import json, sqlite3, os
root=r'C:\Users\user\.codex'
obj=json.load(open(os.path.join(root,'.codex-global-state.json'),encoding='utf-8'))
ids=obj.get('projectless-thread-ids') or []
con=sqlite3.connect(os.path.join(root,'state_5.sqlite'))
rows=con.execute('select id,title,archived,archived_at,rollout_path from threads where id in (%s)' % ','.join('?' for _ in ids), ids).fetchall() if ids else []
print('projectless ids', len(ids))
print('rows', len(rows))
print('active', sum(1 for r in rows if r[2] == 0 and r[3] is None))
print('archived path rows', sum(1 for r in rows if 'archived_sessions' in r[4]))
for r in rows:
    print(r[0], r[1], r[2], r[4])
con.close()
PY
```

Expected:

```text
projectless ids 8
rows 8
active 8
archived path rows 0
```
