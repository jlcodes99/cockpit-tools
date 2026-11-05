"""Recover missing local Codex projects. Stdlib only; default is a read-only audit.

Does not read credentials, copy cloud project ownership, edit rollouts, change
providers, unarchive chats, or expose child-agent threads. Apply only while the
desktop is fully exited. Keep the same CODEX_HOME when switching accounts.
"""
from contextlib import closing
import argparse
import copy
import hashlib
import json
import ntpath
import os
import sqlite3
import sys
import time
from datetime import datetime
from pathlib import Path

STATE = ".codex-global-state.json"
MAP = "app-server-project-id-by-legacy-project-id-by-host"
MIGRATION = "app-server-projects-migration-by-host"
ASSIGN = "thread-project-assignments"
HOSTS = "thread-project-membership-host-ids"
CLOUD = {"codex.chatGptBacking", "codex.chatGptBackingCreation", "codex.retiredChatGptBacking"}


def load(path):
    data = json.loads(path.read_text(encoding="utf-8-sig"))
    if not isinstance(data, dict):
        raise ValueError("Expected a JSON object: " + str(path))
    return data


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def norm(path):
    path = path.strip().replace("/", "\\")
    if path.lower().startswith("\\\\?\\unc\\"):
        path = "\\\\" + path[8:]
    elif path.startswith("\\\\?\\"):
        path = path[4:]
    return ntpath.normpath(path).casefold()


def clean_path(path):
    if path.lower().startswith("\\\\?\\unc\\"):
        return "\\\\" + path[8:]
    return path[4:] if path.startswith("\\\\?\\") else path


def root_thread(row):
    return ("subagent" not in (row.get("source") or "").lower()
            and (row.get("thread_source") or "user") not in ("subagent", "guardian")
            and bool(row.get("has_user_event") or row.get("first_user_message")))


def read_db(home):
    with closing(sqlite3.connect((home / "state_5.sqlite").as_uri() + "?mode=ro", uri=True)) as db, db:
        db.row_factory = sqlite3.Row
        db.execute("BEGIN")
        projects = [dict(r) for r in db.execute("SELECT * FROM projects ORDER BY position")]
        roots = [dict(r) for r in db.execute("SELECT * FROM project_roots ORDER BY position")]
        columns = {r[1] for r in db.execute("PRAGMA table_info(threads)")}
        required = {"id", "cwd", "source", "project_id", "has_user_event"}
        if not required <= columns:
            raise ValueError("Unsupported thread database schema")
        wanted = sorted(columns & {"id", "cwd", "source", "project_id", "has_user_event",
                                  "thread_source", "first_user_message", "archived"})
        threads = [dict(r) for r in db.execute("SELECT " + ",".join(wanted) + " FROM threads")]
        db.rollback()
    return projects, roots, threads


def choose_snapshot(home, state):
    candidates = []
    for p in home.glob("*codex-global-state.json*"):
        if p.name == STATE or not (0 < p.stat().st_size < 20_000_000):
            continue
        try:
            data = load(p)
            if len(data.get("local-projects", {})) > len(state.get("local-projects", {})):
                candidates.append((p.stat().st_mtime_ns, p, data))
        except (OSError, ValueError, TypeError):
            continue
    if not candidates:
        return None, {}
    _, path, data = max(candidates, key=lambda row: row[0])
    return path, data


def build_plan(home, exists=None):
    exists = exists or (lambda p: Path(p).is_dir())
    state_path = home / STATE
    state = load(state_path)
    original_hash = digest(state_path)
    snapshot_path, old = choose_snapshot(home, state)
    projects, roots, threads = read_db(home)
    thread_by_id = {r["id"]: r for r in threads}
    next_state = copy.deepcopy(state)
    host = "local:" + str(home)
    saved = state.get("local-projects", {})
    recovered = copy.deepcopy(saved)
    mapping = dict(state.get(MAP, {}).get(host, {}))
    old_mapping = old.get(MAP, {}).get(host, {})
    old_projects = old.get("local-projects", {})
    roots_by_project = {}
    for r in roots:
        roots_by_project.setdefault(r["project_id"], []).append(r["path"])
    pending = state.get("app-server-pending-project-deletions-by-host", {}).get(host, {})
    blocked = set(pending) | {p.get("projectId") for p in pending.values() if isinstance(p, dict)}
    allowed = {}
    native = []
    skipped = []
    # Current cloud-associated entries stay unchanged. Never reconstruct a cloud
    # mirror or a backing that belongs to an account from a previous snapshot.
    for p in projects:
        server_id = p["id"]
        paths = roots_by_project.get(server_id, [])
        metadata = json.loads(p.get("metadata") or "{}")
        current_id = next((k for k, v in mapping.items() if v == server_id and k in saved), None)
        if current_id is None:
            current_id = next((k for k,v in saved.items() if {norm(x) for x in v.get("rootPaths", [])} == {norm(x) for x in paths} and paths), None)
        cloud = bool(CLOUD & metadata.keys()) or any("\\.chatgpt-projects\\" in norm(x) for x in paths)
        if cloud:
            if current_id is not None:
                mapping[current_id] = server_id
                allowed[current_id] = server_id
            else:
                skipped.append({"id": server_id, "name": p["name"], "reason": "account_scoped_cloud_project"})
            continue
        if server_id in blocked:
            skipped.append({"id": server_id, "name": p["name"], "reason": "pending_deletion"})
            continue
        if not paths or not all(exists(x) for x in paths):
            skipped.append({"id": server_id, "name": p["name"], "reason": "missing_root"})
            continue
        legacy_id = current_id
        if legacy_id is None:
            legacy_id = next((k for k,v in old_mapping.items()
                              if v == server_id and k in old_projects and k not in blocked
                              and not any(old_projects[k].get(x) for x in ("chatGptBacking", "chatGptBackingCreation", "retiredChatGptBacking"))
                              and {norm(x) for x in old_projects[k].get("rootPaths", [])} == {norm(x) for x in paths}), server_id)
        if legacy_id in blocked:
            continue
        if legacy_id not in recovered:
            recovered[legacy_id] = {"id": legacy_id, "name": p["name"], "rootPaths": paths,
                                    "createdAt": p["created_at_ms"], "updatedAt": p["updated_at_ms"]}
        mapping[legacy_id] = server_id
        allowed[legacy_id] = server_id
        native.append({"id": legacy_id, "serverId": server_id, "name": p["name"], "roots": paths})
    next_state["local-projects"] = recovered
    next_state.setdefault(MAP, {})[host] = mapping
    order = []
    for pid in state.get("project-order", []) + old.get("project-order", []) + list(recovered):
        if pid in recovered and pid not in order:
            order.append(pid)
    next_state["project-order"] = order
    by_root = {}
    for legacy_id in allowed:
        for path in recovered[legacy_id].get("rootPaths", []):
            by_root.setdefault(norm(path), []).append(legacy_id)
    # Respect deliberate projectless chats in both current and last intact state.
    projectless = set(state.get("projectless-thread-ids", []))
    old_projectless = set(old.get("projectless-thread-ids", [])) | set(old.get("thread-projectless-output-directories", {}))
    assignments = copy.deepcopy(state.get(ASSIGN, {}))
    # Preserve this exclusion after the intact snapshot is no longer selected;
    # otherwise a second run could assign deliberately projectless old chats.
    inherited_projectless = [tid for tid in sorted(old_projectless)
                            if tid in thread_by_id and root_thread(thread_by_id[tid])
                            and tid not in assignments]
    if inherited_projectless:
        projectless.update(inherited_projectless)
        next_state["projectless-thread-ids"] = list(dict.fromkeys(state.get("projectless-thread-ids", []) + inherited_projectless))
    hosts = dict(state.get(HOSTS, {}))
    patches = []
    assignment_count = 0
    for tid, row in thread_by_id.items():
        if not root_thread(row) or tid in projectless:
            continue
        assignment = assignments.get(tid)
        if assignment is not None:
            if assignment.get("projectKind") != "local" or assignment.get("projectId") not in allowed:
                continue
            pid = assignment["projectId"]
        else:
            if tid in old_projectless:
                continue
            old_assignment = old.get(ASSIGN, {}).get(tid, {})
            pid = old_assignment.get("projectId") if old_assignment.get("projectKind") == "local" else None
            if pid not in allowed:
                matches = by_root.get(norm(row.get("cwd") or ""), [])
                pid = matches[0] if len(matches) == 1 else None
            if pid is None:
                continue
        server_id = allowed[pid]
        if row["project_id"] not in (None, server_id):
            continue  # Never undo an explicit newer assignment.
        new_assignment = {"projectKind": "local", "projectId": pid}
        assignment_count += int(assignments.get(tid) != new_assignment)
        assignments[tid] = new_assignment
        hosts[tid] = "local"
        after_cwd = clean_path(row["cwd"])
        patches.append({"id": tid, "beforeProject": row["project_id"], "afterProject": server_id,
                        "beforeCwd": row["cwd"], "afterCwd": after_cwd, "archived": row.get("archived", 0)})
    next_state[ASSIGN] = assignments
    next_state[HOSTS] = hosts
    if patches:
        checkpoint = next_state.setdefault(MIGRATION, {}).setdefault(host, {"version": 1})
        checkpoint["projectsMigrated"] = True
        checkpoint["threadAssignmentsMigrated"] = False
        checkpoint["pendingThreadAssignmentIds"] = list(dict.fromkeys(checkpoint.get("pendingThreadAssignmentIds", []) + [p["id"] for p in patches]))
    changed_keys = [k for k,v in next_state.items() if state.get(k) != v]
    db_patches = [p for p in patches if p["beforeProject"] != p["afterProject"] or p["beforeCwd"] != p["afterCwd"]]
    catalog = []
    catalog_path = home / "sqlite/codex-dev.db"
    if catalog_path.exists():
        with closing(sqlite3.connect(catalog_path.as_uri() + "?mode=ro", uri=True)) as db, db:
            db.row_factory = sqlite3.Row
            cols = {r[1] for r in db.execute("PRAGMA table_info(local_thread_catalog)")}
            if {"thread_id", "host_id", "project_id", "cwd"} <= cols:
                desired = {p["id"]: p for p in patches}
                for r in db.execute("SELECT host_id,thread_id,project_id,cwd FROM local_thread_catalog WHERE host_id='local'"):
                    p = desired.get(r["thread_id"])
                    if p and r["project_id"] in (None, p["afterProject"]):
                        after_cwd = clean_path(r["cwd"]) if r["cwd"] else r["cwd"]
                        if r["project_id"] != p["afterProject"] or r["cwd"] != after_cwd:
                            catalog.append({"host": r["host_id"], "id": r["thread_id"], "beforeProject": r["project_id"], "afterProject": p["afterProject"], "beforeCwd": r["cwd"], "afterCwd": after_cwd})
    return {"home": str(home), "sourceHash": original_hash, "snapshot": str(snapshot_path) if snapshot_path else None,
            "stateBefore": state, "stateAfter": next_state, "changedKeys": changed_keys,
            "threadPatches": db_patches, "catalogPatches": catalog,
            "summary": {"databaseProjects": len(projects), "nativeProjects": len(native),
                        "visibleProjectsBefore": len(saved), "visibleProjectsAfter": len(recovered),
                        "primaryThreads": sum(root_thread(r) for r in threads),
                        "restoredAssignments": assignment_count, "threadRowsToUpdate": len(db_patches),
                        "catalogRowsToUpdate": len(catalog), "nativeProjectList": native, "skipped": skipped}}


def atomic_json(path, data):
    temp = path.with_name(path.name + ".repair-tmp-" + str(os.getpid()))
    try:
        with temp.open("w", encoding="utf-8", newline="\n") as f:
            json.dump(data, f, ensure_ascii=False, separators=(",", ":"))
            f.flush()
            os.fsync(f.fileno())
        os.replace(temp, path)
    finally:
        if temp.exists():
            temp.unlink()


def desktop_running():
    if os.name != "nt":
        return False
    # This is process inventory, not UI automation. Do not close any process here.
    import subprocess
    result = subprocess.run(["powershell.exe", "-NoProfile", "-NonInteractive", "-Command",
        "Get-Process -Name ChatGPT,Codex -ErrorAction SilentlyContinue | Where-Object { -not $_.Path -or $_.Path -like '*OpenAI.Codex*' -or $_.Path -like '*OpenAI\\Codex*' } | Select-Object -ExpandProperty Id"],
        capture_output=True, text=True, creationflags=0x08000000)
    if result.returncode not in (0, 1):
        raise RuntimeError("Unable to establish whether desktop is exited")
    return bool(result.stdout.strip())


def backup_database(path, target):
    with closing(sqlite3.connect(path.as_uri() + "?mode=ro", uri=True)) as src, closing(sqlite3.connect(target)) as dst, dst:
        src.backup(dst)
        if dst.execute("PRAGMA quick_check").fetchone()[0] != "ok":
            raise RuntimeError("Backup integrity check failed")


def apply_plan(plan, backup_root, check_running=True):
    home = Path(plan["home"])
    if check_running and desktop_running():
        raise RuntimeError("Codex desktop is still running. Exit it fully before applying.")
    if digest(home / STATE) != plan["sourceHash"]:
        raise RuntimeError("Global state changed after audit. Run a fresh audit.")
    if not plan["changedKeys"] and not plan["threadPatches"] and not plan["catalogPatches"]:
        return {"status": "no_changes", "summary": plan["summary"]}
    stamp = datetime.now().strftime("%Y%m%d-%H%M%S-%f")
    backup = backup_root / ("local-project-visibility-" + stamp)
    backup.mkdir(parents=True, exist_ok=False)
    for name in (STATE, STATE + ".bak"):
        source = home / name
        if source.exists():
            (backup / name).write_bytes(source.read_bytes())
    backup_database(home / "state_5.sqlite", backup / "state_5.sqlite")
    cat_path = home / "sqlite/codex-dev.db"
    if cat_path.exists():
        backup_database(cat_path, backup / "codex-dev.db")
    atomic_json(backup / "repair-plan.json", plan)
    db = sqlite3.connect(home / "state_5.sqlite", timeout=10)
    if plan["catalogPatches"]:
        db.execute("ATTACH DATABASE ? AS catalog", (str(cat_path),))
    state_written = False
    try:
        db.execute("BEGIN IMMEDIATE")
        for p in plan["threadPatches"]:
            cursor = db.execute("UPDATE threads SET project_id=?,cwd=? WHERE id=? AND project_id IS ? AND cwd IS ?",
                                (p["afterProject"], p["afterCwd"], p["id"], p["beforeProject"], p["beforeCwd"]))
            if cursor.rowcount != 1:
                raise RuntimeError("Thread changed concurrently: " + p["id"])
        for p in plan["catalogPatches"]:
            cursor = db.execute("UPDATE catalog.local_thread_catalog SET project_id=?,cwd=? WHERE host_id=? AND thread_id=? AND project_id IS ? AND cwd IS ?",
                                (p["afterProject"], p["afterCwd"], p["host"], p["id"], p["beforeProject"], p["beforeCwd"]))
            if cursor.rowcount != 1:
                raise RuntimeError("Catalog changed concurrently: " + p["id"])
        if plan["catalogPatches"]:
            db.execute("UPDATE catalog.local_thread_catalog_metadata SET catalog_revision=catalog_revision+1")
        if digest(home / STATE) != plan["sourceHash"]:
            raise RuntimeError("Global state changed during repair")
        atomic_json(home / STATE, plan["stateAfter"])
        state_written = True
        atomic_json(home / (STATE + ".bak"), plan["stateAfter"])
        db.commit()
    except BaseException:
        db.rollback()
        if state_written:
            atomic_json(home / STATE, plan["stateBefore"])
            backup_bak = backup / (STATE + ".bak")
            if backup_bak.exists():
                atomic_json(home / (STATE + ".bak"), load(backup_bak))
        raise
    finally:
        db.close()
    result = {"status": "applied", "backup": str(backup), "summary": plan["summary"]}
    atomic_json(backup / "result.json", result)
    return result


def rollback(backup, check_running=True):
    if check_running and desktop_running():
        raise RuntimeError("Exit Codex fully before rollback")
    plan = load(backup / "repair-plan.json")
    home = Path(plan["home"])
    current = load(home / STATE)
    changed = []
    for key in plan["changedKeys"]:
        if current.get(key) == plan["stateAfter"].get(key):
            if key in plan["stateBefore"]:
                current[key] = plan["stateBefore"][key]
            else:
                current.pop(key, None)
            changed.append(key)
    db = sqlite3.connect(home / "state_5.sqlite", timeout=10)
    try:
        db.execute("BEGIN IMMEDIATE")
        for p in plan["threadPatches"]:
            db.execute("UPDATE threads SET project_id=?,cwd=? WHERE id=? AND project_id IS ? AND cwd IS ?",
                       (p["beforeProject"], p["beforeCwd"], p["id"], p["afterProject"], p["afterCwd"]))
        db.commit()
    finally:
        db.close()
    cat_path = home / "sqlite/codex-dev.db"
    if plan["catalogPatches"] and cat_path.exists():
        with closing(sqlite3.connect(cat_path)) as db, db:
            for p in plan["catalogPatches"]:
                db.execute("UPDATE local_thread_catalog SET project_id=?,cwd=? WHERE host_id=? AND thread_id=? AND project_id IS ? AND cwd IS ?",
                           (p["beforeProject"], p["beforeCwd"], p["host"], p["id"], p["afterProject"], p["afterCwd"]))
            db.execute("UPDATE local_thread_catalog_metadata SET catalog_revision=catalog_revision+1")
    # Never restore whole DB snapshots over newer chat history.
    before_rollback = home / (STATE + ".pre-rollback-" + datetime.now().strftime("%Y%m%d-%H%M%S-%f"))
    before_rollback.write_bytes((home / STATE).read_bytes())
    atomic_json(home / STATE, current)
    atomic_json(home / (STATE + ".bak"), current)
    return {"status": "rolled_back", "restoredKeys": changed, "preservedNewerData": True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--codex-home", type=Path, default=Path.home() / ".codex")
    parser.add_argument("--apply", action="store_true")
    parser.add_argument("--rollback", type=Path)
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    if args.rollback:
        result = rollback(args.rollback.resolve())
    else:
        plan = build_plan(args.codex_home.resolve())
        result = apply_plan(plan, args.codex_home.resolve() / "backups") if args.apply else {"status": "audit_only", "summary": plan["summary"], "changedKeys": plan["changedKeys"], "snapshot": plan["snapshot"]}
    if args.report:
        atomic_json(args.report, result)
    print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8")
    main()
