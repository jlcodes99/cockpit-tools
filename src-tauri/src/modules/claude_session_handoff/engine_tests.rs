//! Synthetic fixtures only. Every path is under a UUID-owned temporary root.
use super::*;

#[test]
fn guard_preserves_only_bounded_public_process_diagnostics() {
    let diagnostic = json!({"code":"CLAUDE_WRITER_RUNNING","processId":300,"processRole":"desktop-helper","processName":"Claude Helper","privateExtra":"synthetic excluded payload"}).to_string();
    let error = guard_call(&mut || Err(diagnostic.clone())).unwrap_err();
    let value: Value = serde_json::from_str(&error).unwrap();
    assert_eq!(value["code"], "CLAUDE_WRITER_RUNNING");
    assert_eq!(value["processId"], 300);
    assert!(value.get("privateExtra").is_none());
    let unsafe_name = json!({"code":"CLAUDE_WRITER_RUNNING","processId":300,"processRole":"desktop-helper","processName":"/synthetic/private/path"}).to_string();
    assert_eq!(
        guard_call(&mut || Err(unsafe_name.clone())).unwrap_err(),
        "GUARD_FAILED"
    );
}
#[path = "continuity_tests.rs"]
mod continuity_tests;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::{symlink, PermissionsExt};

fn id(n: u32) -> String {
    format!("00000000-0000-4000-8000-{n:012}")
}
fn sid(n: u32) -> String {
    format!("local_{}", id(n))
}
fn name(n: u32) -> String {
    format!("{}.json", sid(n))
}

fn record(n: u32) -> Value {
    json!({"sessionId":sid(n), "cliSessionId":id(n + 100), "cwd":"/synthetic/project",
        "originCwd":"/synthetic/project", "title":format!("Synthetic {n}"), "titleSource":"user",
        "isArchived":false, "createdAt":1, "lastActivityAt":2, "lastFocusedAt":2,
        "completedTurns":3, "permissionMode":"auto", "model":"synthetic-model", "effort":"high"})
}

struct Fixture {
    root: PathBuf,
    roots: Roots,
    a: Identity,
    b: Identity,
}

impl Fixture {
    fn new() -> Self {
        let root = fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!("cockpit-handoff-test-{}", Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let roots = Roots {
            records: root.join("records"),
            pool: root.join("pool"),
            state: root.join("state"),
        };
        let a = Identity {
            account: id(1),
            org: id(2),
        };
        let b = Identity {
            account: id(3),
            org: id(4),
        };
        for path in [
            roots.records.join(identity_key(&a).unwrap()),
            roots.records.join(identity_key(&b).unwrap()),
            roots.pool.join("project"),
        ] {
            Directory::open(&path, true).unwrap();
        }
        Self { root, roots, a, b }
    }

    fn dir(&self, identity: &Identity) -> PathBuf {
        self.roots.records.join(identity_key(identity).unwrap())
    }
    fn path(&self, identity: &Identity, n: u32) -> PathBuf {
        self.dir(identity).join(name(n))
    }
    fn transcript(&self, r: &Value) -> PathBuf {
        self.roots
            .pool
            .join("project")
            .join(format!("{}.jsonl", r["cliSessionId"].as_str().unwrap()))
    }
    fn put(&self, identity: &Identity, value: &Value, transcript: bool) {
        let path = self
            .dir(identity)
            .join(format!("{}.json", value["sessionId"].as_str().unwrap()));
        fs::write(path, json_bytes(value).unwrap()).unwrap();
        if transcript {
            fs::write(self.transcript(value), b"{\"synthetic\":true}\n").unwrap();
        }
    }
    fn get(&self, identity: &Identity, n: u32) -> Value {
        decode(&required_blob(&self.path(identity, n)).unwrap()).unwrap()
    }
    fn backup(&self) -> PathBuf {
        self.root.join(format!("backup-{}", Uuid::new_v4()))
    }
    fn preview(&self) -> Preview {
        preview(&self.roots, &self.a, &self.b).unwrap()
    }
    fn apply(&self) -> RunSummary {
        self.transfer(&self.a, &self.b)
    }
    fn transfer(&self, source: &Identity, target: &Identity) -> RunSummary {
        let p = preview(&self.roots, source, target).unwrap();
        apply(
            &self.roots,
            source,
            target,
            &p.fingerprint,
            &self.backup(),
            &mut || Ok(()),
        )
        .unwrap()
    }
    fn pending(&self) -> Journal {
        let run = list_runs(&self.roots)
            .unwrap()
            .into_iter()
            .find(|r| r.state != "applied" && r.state != "rolled_back")
            .unwrap();
        load(&self.roots, &run.id).unwrap()
    }
    fn undo(&self, run: &RunSummary) -> RunSummary {
        rollback(&self.roots, &run.id, &mut || Ok(())).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

fn issue(p: &Preview, n: u32, reason: &str) -> bool {
    p.issues
        .iter()
        .any(|i| i.session_id == sid(n) && i.reason == reason)
}

#[test]
fn one_source_only_canary_uses_journal_and_preserves_other_rows() {
    let f = Fixture::new();
    let selected = record(1);
    let unrelated = record(2);
    let existing = record(3);
    f.put(&f.a, &selected, true);
    f.put(&f.a, &unrelated, true);
    f.put(&f.b, &existing, true);
    let target_before = fs::read(f.path(&f.b, 3)).unwrap();
    let source_before = fs::read(f.path(&f.a, 1)).unwrap();
    let plan = build_plan_filtered(&f.roots, &f.a, &f.b, Some(&name(1))).unwrap();
    assert_eq!(plan.preview.created, 1);
    assert_eq!(plan.preview.updated, 0);
    assert_eq!(plan.preview.missing, 0);
    assert!(plan.preview.issues.is_empty());
    assert_eq!(plan.transcripts.len(), 1);
    let run = apply_observed_filtered(
        &f.roots,
        &f.a,
        &f.b,
        &plan.preview.fingerprint,
        &f.backup(),
        &mut || Ok(()),
        None,
        &mut |_| {},
        Some(&name(1)),
    )
    .unwrap();
    assert_eq!(run.state, "applied");
    assert!(f.path(&f.b, 1).is_file());
    assert!(!f.path(&f.b, 2).exists());
    assert_eq!(fs::read(f.path(&f.b, 3)).unwrap(), target_before);
    assert_eq!(fs::read(f.path(&f.a, 1)).unwrap(), source_before);
    let rolled_back = f.undo(&run);
    assert_eq!(rolled_back.state, "rolled_back");
    assert!(!f.path(&f.b, 1).exists());
    assert_eq!(fs::read(f.path(&f.b, 3)).unwrap(), target_before);
    assert_eq!(fs::read(f.path(&f.a, 1)).unwrap(), source_before);
    assert!(!baseline_path(&f.roots, &f.a, &f.b).unwrap().exists());
}

#[test]
fn opposite_direction_source_only_rows_can_be_added_without_replacing_either_side() {
    let f = Fixture::new();
    f.put(&f.a, &record(1), true);
    f.put(&f.b, &record(2), true);
    let a_before = fs::read(f.path(&f.a, 1)).unwrap();
    let b_before = fs::read(f.path(&f.b, 2)).unwrap();
    let forward = build_plan_filtered(&f.roots, &f.a, &f.b, Some(&name(1))).unwrap();
    let forward_run = apply_observed_filtered(
        &f.roots,
        &f.a,
        &f.b,
        &forward.preview.fingerprint,
        &f.backup(),
        &mut || Ok(()),
        None,
        &mut |_| {},
        Some(&name(1)),
    )
    .unwrap();
    assert_eq!((forward_run.created, forward_run.updated), (1, 0));
    let reverse = build_plan_filtered(&f.roots, &f.b, &f.a, Some(&name(2))).unwrap();
    assert_eq!((reverse.preview.created, reverse.preview.updated), (1, 0));
    assert!(reverse.preview.issues.is_empty());
    let reverse_run = apply_observed_filtered(
        &f.roots,
        &f.b,
        &f.a,
        &reverse.preview.fingerprint,
        &f.backup(),
        &mut || Ok(()),
        None,
        &mut |_| {},
        Some(&name(2)),
    )
    .unwrap();
    assert_eq!((reverse_run.created, reverse_run.updated), (1, 0));
    assert!(f.path(&f.a, 2).is_file());
    assert!(f.path(&f.b, 1).is_file());
    assert_eq!(fs::read(f.path(&f.a, 1)).unwrap(), a_before);
    assert_eq!(fs::read(f.path(&f.b, 2)).unwrap(), b_before);
    assert_eq!(f.undo(&reverse_run).state, "rolled_back");
    assert_eq!(f.undo(&forward_run).state, "rolled_back");
    assert!(!f.path(&f.a, 2).exists());
    assert!(!f.path(&f.b, 1).exists());
    assert_eq!(fs::read(f.path(&f.a, 1)).unwrap(), a_before);
    assert_eq!(fs::read(f.path(&f.b, 2)).unwrap(), b_before);
}

#[test]
fn earlier_baseline_rollback_follows_only_verified_later_undo_inodes() {
    let f = Fixture::new();
    f.put(&f.a, &record(1), true);
    f.put(&f.b, &record(2), true);
    f.put(&f.a, &record(3), true);
    let transfer = |source: &Identity, target: &Identity, n: u32| {
        let plan = build_plan_filtered(&f.roots, source, target, Some(&name(n))).unwrap();
        apply_observed_filtered(
            &f.roots,
            source,
            target,
            &plan.preview.fingerprint,
            &f.backup(),
            &mut || Ok(()),
            None,
            &mut |_| {},
            Some(&name(n)),
        )
        .unwrap()
    };
    let first = transfer(&f.a, &f.b, 1);
    let second = transfer(&f.b, &f.a, 2);
    let third = transfer(&f.a, &f.b, 3);
    assert_eq!(f.undo(&third).state, "rolled_back");
    assert_eq!(f.undo(&second).state, "rolled_back");
    let baseline = baseline_path(&f.roots, &f.a, &f.b).unwrap();
    let replacement = baseline.with_extension("replacement");
    fs::write(&replacement, fs::read(&baseline).unwrap()).unwrap();
    fs::rename(&replacement, &baseline).unwrap();
    assert_eq!(
        rollback(&f.roots, &first.id, &mut || Ok(())).unwrap_err(),
        "ROLLBACK_DRIFT"
    );
    assert!(f.path(&f.b, 1).is_file());
}

#[test]
fn earlier_baseline_rollback_handles_multiple_verified_later_undos() {
    let f = Fixture::new();
    f.put(&f.a, &record(1), true);
    f.put(&f.b, &record(2), true);
    f.put(&f.a, &record(3), true);
    let transfer = |source: &Identity, target: &Identity, n: u32| {
        let plan = build_plan_filtered(&f.roots, source, target, Some(&name(n))).unwrap();
        apply_observed_filtered(
            &f.roots,
            source,
            target,
            &plan.preview.fingerprint,
            &f.backup(),
            &mut || Ok(()),
            None,
            &mut |_| {},
            Some(&name(n)),
        )
        .unwrap()
    };
    let first = transfer(&f.a, &f.b, 1);
    let second = transfer(&f.b, &f.a, 2);
    let third = transfer(&f.a, &f.b, 3);
    assert_eq!(f.undo(&third).state, "rolled_back");
    assert_eq!(f.undo(&second).state, "rolled_back");
    assert_eq!(f.undo(&first).state, "rolled_back");
    assert!(!f.path(&f.b, 1).exists());
    assert!(!f.path(&f.a, 2).exists());
    assert!(!f.path(&f.b, 3).exists());
}

#[test]
fn source_only_canary_rejects_shared_row_wrong_fingerprint_and_unselected_parent() {
    let f = Fixture::new();
    f.put(&f.a, &record(1), true);
    f.put(&f.b, &record(1), true);
    assert_eq!(
        build_plan_filtered(&f.roots, &f.a, &f.b, Some(&name(1)))
            .err()
            .unwrap(),
        "INVALID_SOURCE_ONLY_SELECTION"
    );
    let mut child = record(2);
    child["forkedFromSessionId"] = json!(sid(3));
    f.put(&f.a, &child, true);
    f.put(&f.a, &record(3), true);
    let selected = build_plan_filtered(&f.roots, &f.a, &f.b, Some(&name(2))).unwrap();
    assert!(issue(&selected.preview, 2, "EXCLUDED_PARENT"));
    let wrong = f.preview().fingerprint;
    assert_eq!(
        apply_observed_filtered(
            &f.roots,
            &f.a,
            &f.b,
            &wrong,
            &f.backup(),
            &mut || Ok(()),
            None,
            &mut |_| {},
            Some(&name(2))
        )
        .unwrap_err(),
        "PREVIEW_CHANGED"
    );
    assert_eq!(
        apply_observed_filtered(
            &f.roots,
            &f.a,
            &f.b,
            &selected.preview.fingerprint,
            &f.backup(),
            &mut || Ok(()),
            None,
            &mut |_| {},
            Some(&name(2))
        )
        .unwrap_err(),
        "CANARY_NOT_CLEAN"
    );
    assert!(!f.path(&f.b, 2).exists());
}

#[test]
fn source_only_canary_checks_selected_rewind_history_without_unrelated_transcripts() {
    let f = Fixture::new();
    let mut source = record(10);
    let parent = id(200);
    let middle = id(201);
    let active = source["cliSessionId"].as_str().unwrap().to_owned();
    source["priorCliSessionIds"] = json!([parent, middle]);
    source["rewindEdges"] = json!([
        {"id":id(300),"parent":parent,"child":middle,
            "at":1000,"cwd":"/synthetic/project","forkPoint":id(400)},
        {"id":id(301),"parent":middle,"child":active,
            "at":2000,"cwd":"/synthetic/project","forkPoint":id(401)}
    ]);
    source["transcriptModelStates"] = json!({
        middle.clone():{"model":"synthetic-model"},
        active.clone():{"model":"synthetic-model"}
    });
    f.put(&f.a, &source, true);
    f.put(&f.a, &record(11), true);
    for id in [&parent, &middle] {
        fs::write(
            f.roots.pool.join("project").join(format!("{id}.jsonl")),
            b"{}\n",
        )
        .unwrap();
    }
    let plan = build_plan_filtered(&f.roots, &f.a, &f.b, Some(&name(10))).unwrap();
    assert_eq!(plan.transcripts.len(), 3);
    assert_eq!(plan.preview.created, 1);
    assert!(plan.preview.issues.is_empty());
    let run = apply_observed_filtered(
        &f.roots,
        &f.a,
        &f.b,
        &plan.preview.fingerprint,
        &f.backup(),
        &mut || Ok(()),
        None,
        &mut |_| {},
        Some(&name(10)),
    )
    .unwrap();
    let imported = f.get(&f.b, 10);
    assert_eq!(imported["rewindEdges"], source["rewindEdges"]);
    assert_eq!(imported["priorCliSessionIds"], source["priorCliSessionIds"]);
    assert!(!f.path(&f.b, 11).exists());
    assert_eq!(f.undo(&run).state, "rolled_back");
}

#[test]
fn newer_desktop_state_is_skipped_instead_of_silently_dropped() {
    let plain = record(1);
    let tasks = BTreeSet::new();
    assert_eq!(exclusion(&plain, &tasks, &plain), None);
    for key in [
        "asides",
        "gitAnchors",
        "withheldConnectorHosts",
        "turnBoxMounted",
        "remoteControlDescendant",
    ] {
        let mut changed = plain.clone();
        changed[key] = if key.ends_with("Mounted") || key.ends_with("Descendant") {
            json!(true)
        } else {
            json!(["synthetic"])
        };
        assert_eq!(
            exclusion(&changed, &tasks, &changed),
            Some("UNSUPPORTED_HISTORY_STATE"),
            "{key}"
        );
    }
}

#[test]
fn account_capability_and_recomputable_subagent_marker_do_not_hide_local_sessions() {
    let f = Fixture::new();
    let mut capability = record(10);
    capability["autoModeServerFallbackPrompt"] = json!(true);
    f.put(&f.a, &capability, true);
    let mut truncated = record(11);
    truncated["subagentsTruncatedFor"] = truncated["cliSessionId"].clone();
    f.put(&f.a, &truncated, true);
    let preview = f.preview();
    assert_eq!((preview.created, preview.updated), (2, 0));
    assert_eq!(preview.missing, 0);
    assert!(preview.issues.is_empty());
    f.apply();
    assert!(f
        .get(&f.b, 10)
        .get("autoModeServerFallbackPrompt")
        .is_none());
    assert_eq!(
        f.get(&f.b, 11)["subagentsTruncatedFor"],
        truncated["cliSessionId"]
    );
    assert_eq!(f.get(&f.a, 10), capability);
    assert_eq!(f.get(&f.a, 11), truncated);
}

#[test]
fn malformed_capability_or_stale_subagent_marker_still_skips_a_record() {
    for (key, value) in [
        ("autoModeServerFallbackPrompt", json!({"pending":true})),
        ("subagentsTruncatedFor", json!(id(999))),
    ] {
        let f = Fixture::new();
        let mut source = record(10);
        source[key] = value;
        f.put(&f.a, &source, true);
        let preview = f.preview();
        assert_eq!((preview.created, preview.missing), (0, 1), "{key}");
        assert!(issue(&preview, 10, "UNSUPPORTED_HISTORY_STATE"), "{key}");
    }
}

#[test]
fn target_capability_is_kept_and_old_subagent_marker_is_cleared_on_pointer_change() {
    let f = Fixture::new();
    f.put(&f.a, &record(10), true);
    f.apply();
    let mut target = f.get(&f.b, 10);
    target["autoModeServerFallbackPrompt"] = json!(true);
    target["subagentsTruncatedFor"] = target["cliSessionId"].clone();
    f.put(&f.b, &target, false);
    let mut source = record(10);
    source["cliSessionId"] = json!(id(888));
    source["priorCliSessionIds"] = json!([id(110)]);
    f.put(&f.a, &source, true);
    let preview = f.preview();
    assert_eq!(preview.updated, 1);
    f.apply();
    let after = f.get(&f.b, 10);
    assert_eq!(after["autoModeServerFallbackPrompt"], true);
    assert!(after.get("subagentsTruncatedFor").is_none());
}

#[test]
fn verified_local_rewind_graph_and_model_state_survive_first_import() {
    let f = Fixture::new();
    let mut source = record(10);
    let parent = id(200);
    let middle = id(201);
    let child = source["cliSessionId"].as_str().unwrap().to_owned();
    source["rewindEdges"] = json!([
        {"id":id(300),"parent":parent,"child":middle,
            "at":1000,"cwd":"/synthetic/project","forkPoint":id(400)},
        {"id":id(301),"parent":middle,"child":child,
            "at":2000,"cwd":"/synthetic/project","forkPoint":id(401)}
    ]);
    source["transcriptModelStates"] = json!({
        middle.clone():{"model":"synthetic-model"},
        child.clone():{"model":"synthetic-model"}
    });
    f.put(&f.a, &source, true);
    fs::write(
        f.roots.pool.join("project").join(format!("{parent}.jsonl")),
        b"{}\n",
    )
    .unwrap();
    fs::write(
        f.roots.pool.join("project").join(format!("{middle}.jsonl")),
        b"{}\n",
    )
    .unwrap();
    let initial = f.preview();
    assert_eq!((initial.created, initial.updated), (1, 0));
    assert!(initial.issues.is_empty());
    f.apply();
    let target = f.get(&f.b, 10);
    assert_eq!(target["rewindEdges"], source["rewindEdges"]);
    assert_eq!(
        target["transcriptModelStates"],
        source["transcriptModelStates"]
    );
    assert_eq!(f.get(&f.a, 10), source);
    let reverse = preview(&f.roots, &f.b, &f.a).unwrap();
    assert!(!issue(&reverse, 10, "UNSUPPORTED_HISTORY_STATE"));
}

#[test]
fn rewind_graph_with_missing_branch_or_invalid_model_state_is_not_copied() {
    for missing_branch in [true, false] {
        let f = Fixture::new();
        let mut source = record(10);
        let child = source["cliSessionId"].as_str().unwrap().to_owned();
        source["rewindEdges"] = json!([{"id":id(300),"parent":id(200),"child":child,
            "at":1000,"cwd":"/synthetic/project","forkPoint":id(400)}]);
        source["transcriptModelStates"] = if missing_branch {
            json!({child.clone():{"model":"synthetic-model"}})
        } else {
            json!({child.clone():{"model":"synthetic-model","unknownExecutionState":true}})
        };
        f.put(&f.a, &source, true);
        if !missing_branch {
            fs::write(
                f.roots
                    .pool
                    .join("project")
                    .join(format!("{}.jsonl", id(200))),
                b"{}\n",
            )
            .unwrap();
        }
        let preview = f.preview();
        assert_eq!((preview.created, preview.updated), (0, 0));
        assert_eq!(preview.missing, 1);
        assert!(issue(&preview, 10, "UNSUPPORTED_HISTORY_STATE"));
        assert_eq!(f.apply().skipped_missing, 1);
        assert!(!f.path(&f.b, 10).exists());
    }
}

#[test]
fn a_local_rewind_graph_does_not_overwrite_a_divergent_target_pointer() {
    let f = Fixture::new();
    let mut source = record(10);
    let child = source["cliSessionId"].as_str().unwrap().to_owned();
    source["rewindEdges"] = json!([{"id":id(300),"parent":id(200),"child":child,
        "at":1000,"cwd":"/synthetic/project","forkPoint":id(400)}]);
    source["transcriptModelStates"] = json!({child.clone():{"model":"synthetic-model"}});
    f.put(&f.a, &source, true);
    fs::write(
        f.roots
            .pool
            .join("project")
            .join(format!("{}.jsonl", id(200))),
        b"{}\n",
    )
    .unwrap();
    let mut target = record(10);
    target["cliSessionId"] = json!(id(999));
    f.put(&f.b, &target, true);
    let preview = f.preview();
    assert_eq!(
        (
            preview.created,
            preview.updated,
            preview.missing,
            preview.stale
        ),
        (0, 0, 0, 1)
    );
    assert!(issue(&preview, 10, "DIVERGENT_METADATA"));
    assert_eq!(f.apply().skipped_stale, 1);
    assert_eq!(f.get(&f.b, 10), target);
}

#[test]
fn a_prior_pointer_does_not_authorize_overwriting_target_activity() {
    let f = Fixture::new();
    let mut source = record(10);
    source["priorCliSessionIds"] = json!([id(999)]);
    source["lastActivityAt"] = json!(100);
    f.put(&f.a, &source, true);
    let mut target = record(10);
    target["cliSessionId"] = json!(id(999));
    target["lastActivityAt"] = json!(101);
    f.put(&f.b, &target, true);
    let preview = f.preview();
    assert!(issue(&preview, 10, "DIVERGENT_METADATA"));
    assert_eq!(preview.stale, 1);
    assert_eq!(f.apply().skipped_stale, 1);
    assert_eq!(f.get(&f.b, 10), target);
}

#[test]
fn newer_source_lineage_replaces_active_pointer_with_reversible_target_preimage() {
    let f = Fixture::new();
    let mut source = record(10);
    source["priorCliSessionIds"] = json!([id(999)]);
    source["lastActivityAt"] = json!(100);
    source["completedTurns"] = json!(12);
    f.put(&f.a, &source, true);
    let mut target = record(10);
    target["cliSessionId"] = json!(id(999));
    target["lastActivityAt"] = json!(90);
    target["priorCliSessionIds"] = json!([id(777), id(888)]);
    target["contextExceededCount"] = json!(7);
    target["postTurnSummary"] = json!({"status_category":"blocked"});
    target["postTurnSummaryFor"] = json!(id(999));
    f.put(&f.b, &target, true);

    let preview = f.preview();
    assert_eq!(
        (
            preview.created,
            preview.updated,
            preview.stale,
            preview.replaced_branches
        ),
        (0, 1, 0, 1)
    );
    assert!(
        preview
            .warnings
            .iter()
            .any(|warning| warning.session_id == sid(10)
                && warning.reason == "SOURCE_BRANCH_SELECTED")
    );
    let run = f.apply();
    assert_eq!(run.replaced_branches, 1);
    let after = f.get(&f.b, 10);
    assert_eq!(after["cliSessionId"], source["cliSessionId"]);
    assert_eq!(after["priorCliSessionIds"], source["priorCliSessionIds"]);
    assert_eq!(after["lastActivityAt"], 100);
    assert!(after.get("postTurnSummary").is_none());
    assert!(after.get("contextExceededCount").is_none());
    let backup: Value = serde_json::from_slice(
        &fs::read(Path::new(&run.backup_dir).join("target").join(name(10))).unwrap(),
    )
    .unwrap();
    assert_eq!(backup, target);
    assert_eq!(f.undo(&run).state, "rolled_back");
    assert_eq!(f.get(&f.b, 10), target);
    assert_eq!(f.get(&f.a, 10), source);
}

#[test]
fn newer_source_pointer_does_not_require_complete_prior_id_history() {
    let f = Fixture::new();
    let mut source = record(10);
    source["lastActivityAt"] = json!(100);
    f.put(&f.a, &source, true);
    let mut target = record(10);
    target["cliSessionId"] = json!(id(999));
    target["lastActivityAt"] = json!(90);
    f.put(&f.b, &target, true);
    let preview = f.preview();
    assert_eq!(
        (preview.updated, preview.replaced_branches, preview.stale),
        (1, 1, 0)
    );
    assert_eq!(f.apply().replaced_branches, 1);
    assert_eq!(f.get(&f.b, 10)["cliSessionId"], source["cliSessionId"]);
}

#[test]
fn equal_or_older_source_branch_stays_skipped() {
    for prior in [false, true] {
        let f = Fixture::new();
        let mut source = record(10);
        source["lastActivityAt"] = json!(100);
        if prior {
            source["priorCliSessionIds"] = json!([id(999)]);
        }
        f.put(&f.a, &source, true);
        let mut target = record(10);
        target["cliSessionId"] = json!(id(999));
        target["lastActivityAt"] = json!(if prior { 100 } else { 101 });
        f.put(&f.b, &target, true);
        let preview = f.preview();
        assert_eq!(preview.replaced_branches, 0);
        assert_eq!(preview.stale, 1);
        assert!(issue(&preview, 10, "DIVERGENT_METADATA"));
        assert_eq!(f.apply().replaced_branches, 0);
        assert_eq!(f.get(&f.b, 10), target);
    }
}

#[test]
fn newer_source_retires_only_the_old_pointer_quit_marker() {
    let f = Fixture::new();
    let mut source = record(10);
    source["lastActivityAt"] = json!(100);
    f.put(&f.a, &source, true);
    let mut target = record(10);
    target["cliSessionId"] = json!(id(999));
    target["lastActivityAt"] = json!(90);
    target["interruptedByQuitAt"] = json!(80);
    f.put(&f.b, &target, true);
    let preview = f.preview();
    assert_eq!(
        (preview.updated, preview.replaced_branches, preview.stale),
        (1, 1, 0)
    );
    let run = f.apply();
    assert_eq!(run.replaced_branches, 1);
    let after = f.get(&f.b, 10);
    assert_eq!(after["cliSessionId"], source["cliSessionId"]);
    assert!(after.get("interruptedByQuitAt").is_none());
    f.undo(&run);
    assert_eq!(f.get(&f.b, 10), target);

    target["pendingFirstStart"] = json!(true);
    f.put(&f.b, &target, true);
    let blocked = f.preview();
    assert_eq!(
        (blocked.updated, blocked.replaced_branches, blocked.stale),
        (0, 0, 1)
    );
    assert!(issue(&blocked, 10, "TARGET_UNFINISHED_SESSION"));
}

#[test]
fn source_rollover_does_not_resolve_unrelated_metadata_conflicts() {
    let f = Fixture::new();
    let mut source = record(10);
    source["priorCliSessionIds"] = json!([id(999)]);
    source["lastActivityAt"] = json!(100);
    source["title"] = json!("Source title");
    f.put(&f.a, &source, true);
    let mut target = record(10);
    target["cliSessionId"] = json!(id(999));
    target["lastActivityAt"] = json!(90);
    target["title"] = json!("Target title");
    f.put(&f.b, &target, true);
    let preview = f.preview();
    assert_eq!(preview.replaced_branches, 0);
    assert_eq!(preview.stale, 1);
    assert_eq!(preview.updated, 0);
    assert_eq!(f.apply().skipped_stale, 1);
    assert_eq!(f.get(&f.b, 10), target);
}

#[test]
fn older_run_receipts_without_missing_count_remain_readable() {
    let f = Fixture::new();
    f.put(&f.a, &record(10), true);
    let run = f.apply();
    let mut value = serde_json::to_value(&run).unwrap();
    value.as_object_mut().unwrap().remove("skippedMissing");
    value.as_object_mut().unwrap().remove("replacedBranches");
    let old: RunSummary = serde_json::from_value(value).unwrap();
    assert_eq!(old.skipped_missing, 0);
    assert_eq!(old.replaced_branches, 0);
}

#[test]
fn changing_transcript_clears_destination_turn_summary() {
    let source = record(1);
    let mut target = record(1);
    target["cliSessionId"] = json!(id(999));
    target["postTurnSummary"] = json!({"status_category":"blocked"});
    target["postTurnSummaryFor"] = json!(id(999));
    target["lastAssistantUuid"] = json!(id(999));
    target["turnWrapUp"] = json!({"synthetic":true});
    let merged = materialize(&portable(&source), &source, Some(&target));
    for key in [
        "postTurnSummary",
        "postTurnSummaryFor",
        "lastAssistantUuid",
        "turnWrapUp",
    ] {
        assert!(merged.get(key).is_none(), "{key}");
    }
}

#[test]
fn preview_is_read_only_and_fingerprint_binds_raw_snapshots_and_identities() {
    let f = Fixture::new();
    f.put(&f.a, &record(10), true);
    let before = required_blob(&f.path(&f.a, 10)).unwrap();
    let p = f.preview();
    assert!(!f.roots.state.exists());
    assert_eq!(p.created, 1);
    assert_eq!(p.fingerprint, f.preview().fingerprint);
    let mut whitespace = before.bytes.clone();
    whitespace.extend(b" \n");
    fs::write(f.path(&f.a, 10), whitespace).unwrap();
    assert_ne!(p.fingerprint, f.preview().fingerprint);
    f.put(&f.b, &record(10), true);
    assert_ne!(
        f.preview().fingerprint,
        preview(&f.roots, &f.b, &f.a).unwrap().fingerprint
    );
    assert!(!f.roots.state.exists());
}

#[test]
fn new_record_has_default_permissions_and_preserves_all_source_bytes_and_history() {
    let f = Fixture::new();
    let mut r = record(10);
    r["isArchived"] = json!(true);
    r["priorCliSessionIds"] = json!([id(999)]);
    for (key, value) in [
        ("bridgeSessionIds", json!(["foreign"])),
        ("spawnSeed", json!({"systemPrompt":"synthetic"})),
        ("sessionSettings", json!({"permission":"unsafe"})),
        ("enabledMcpTools", json!({"tool":true})),
        ("remoteMcpServersConfig", json!([{"id":"synthetic"}])),
        ("cuAllowedApps", json!(["synthetic-app"])),
        ("automaticDispatch", json!(true)),
        ("autoChosenInApp", json!(true)),
        ("bypassChosenInApp", json!(true)),
        ("error", json!("Session limit reached. Resets at noon")),
    ] {
        r[key] = value;
    }
    f.put(&f.a, &r, true);
    let original = required_blob(&f.path(&f.a, 10)).unwrap();
    let history = fs::read(f.transcript(&r)).unwrap();
    let backup = f.backup();
    fs::create_dir(&backup).unwrap();
    let run = apply(
        &f.roots,
        &f.a,
        &f.b,
        &f.preview().fingerprint,
        &backup,
        &mut || Ok(()),
    )
    .unwrap();
    let out = f.get(&f.b, 10);
    assert_eq!(out["permissionMode"], "default");
    assert_eq!(out["chromePermissionMode"], "ask");
    assert_eq!(
        out["cuGrantFlags"],
        json!({"clipboardRead":false,"clipboardWrite":false,"systemKeyCombos":false})
    );
    assert_eq!(out["enabledMcpTools"], json!({}));
    assert_eq!(out["remoteMcpServersConfig"], json!([]));
    assert_eq!(out["priorCliSessionIds"], r["priorCliSessionIds"]);
    assert_eq!(out["isArchived"], true);
    assert_eq!(out["bypassChosenInApp"], false);
    for key in [
        "spawnSeed",
        "sessionSettings",
        "automaticDispatch",
        "autoChosenInApp",
        "error",
    ] {
        assert!(out.get(key).is_none());
    }
    assert_eq!(
        required_blob(&f.path(&f.a, 10)).unwrap().stamp,
        original.stamp
    );
    assert_eq!(fs::read(f.transcript(&r)).unwrap(), history);
    assert_eq!(run.state, "applied");
    for path in [&f.roots.state, &backup, &f.roots.state.join(&run.id)] {
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    for path in [f.path(&f.b, 10), journal_path(&f.roots, &run.id)] {
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    assert_eq!(f.preview().unchanged, 1);
    f.undo(&run);
    assert!(!f.path(&f.b, 10).exists());
    assert!(!baseline_path(&f.roots, &f.a, &f.b).unwrap().exists());
    assert_eq!(
        required_blob(&f.path(&f.a, 10)).unwrap().stamp,
        original.stamp
    );
}

#[test]
fn round_trip_pointer_rollover_and_independent_target_changes_merge_in_both_directions() {
    let f = Fixture::new();
    let mut a = record(10);
    a["rewindEdges"] = json!([]);
    a["transcriptCuts"] = json!({});
    a["transcriptModelStates"] = json!({});
    f.put(&f.a, &a, true);
    f.apply();
    let mut b = f.get(&f.b, 10);
    b["cliSessionId"] = json!(id(999));
    b["priorCliSessionIds"] = json!([a["cliSessionId"]]);
    b["title"] = json!("Changed in B");
    b["lastActivityAt"] = json!(200);
    b["completedTurns"] = json!(20);
    f.put(&f.b, &b, true);
    a["color"] = json!("blue");
    f.put(&f.a, &a, false);
    let reverse = f.transfer(&f.b, &f.a);
    assert_eq!(reverse.updated, 1);
    let merged = f.get(&f.a, 10);
    assert_eq!(merged["cliSessionId"], id(999));
    assert_eq!(merged["priorCliSessionIds"], json!([id(110)]));
    assert_eq!(merged["title"], "Changed in B");
    assert_eq!(merged["color"], "blue");
    assert_eq!(merged["completedTurns"], 20);
    for key in [
        "armedWorkAtQuit",
        "interruptedByQuitAt",
        "interruptedUnseenResume",
    ] {
        assert!(merged.get(key).is_none());
    }
    f.apply();
    assert_eq!(f.get(&f.b, 10)["color"], "blue");
    assert_eq!(f.preview().unchanged, 1);
    assert_eq!(
        baseline_path(&f.roots, &f.a, &f.b).unwrap(),
        baseline_path(&f.roots, &f.b, &f.a).unwrap()
    );
}

#[test]
fn target_only_edits_are_retained_until_reverse_handoff_and_conflicts_skip_whole_record() {
    let f = Fixture::new();
    f.put(&f.a, &record(10), true);
    f.apply();
    let mut b = f.get(&f.b, 10);
    b["title"] = json!("B only");
    f.put(&f.b, &b, false);
    assert_eq!(f.preview().unchanged, 1);
    f.apply();
    f.transfer(&f.b, &f.a);
    assert_eq!(f.get(&f.a, 10)["title"], "B only");
    let mut a = f.get(&f.a, 10);
    a["title"] = json!("A collision");
    b["title"] = json!("B collision");
    b["isArchived"] = json!(true);
    f.put(&f.a, &a, false);
    f.put(&f.b, &b, false);
    let p = f.preview();
    assert_eq!(p.updated, 0);
    assert!(issue(&p, 10, "DIVERGENT_METADATA"));
    f.apply();
    assert_eq!(f.get(&f.b, 10), b);
}

#[test]
fn no_baseline_divergence_is_never_resolved_by_recency() {
    let f = Fixture::new();
    let a = record(10);
    let mut b = a.clone();
    b["title"] = json!("different");
    b["lastActivityAt"] = json!(999999);
    f.put(&f.a, &a, true);
    f.put(&f.b, &b, true);
    assert!(issue(&f.preview(), 10, "DIVERGENT_METADATA"));
    f.apply();
    assert_eq!(f.get(&f.b, 10), b);
}

#[test]
fn portable_groups_merge_independently_and_optional_field_removal_is_preserved() {
    let mut base = record(10);
    base["color"] = json!("red");
    let mut a = base.clone();
    let mut b = base.clone();
    a["title"] = json!("source");
    a.as_object_mut().unwrap().remove("color");
    b["isArchived"] = json!(true);
    let merged = merge(&portable(&a), &portable(&b), Some(&portable(&base))).unwrap();
    let out = materialize(&merged, &a, Some(&b));
    assert_eq!(out["title"], "source");
    assert_eq!(out["isArchived"], true);
    assert!(out.get("color").is_none());
    a["cwd"] = json!("/synthetic/a");
    b["branch"] = json!("different");
    assert!(merge(&portable(&a), &portable(&b), Some(&portable(&base))).is_none());
}

#[test]
fn ordinary_quota_pause_is_eligible_and_same_pointer_reset_is_narrow_and_reversible() {
    let f = Fixture::new();
    let mut a = record(10);
    a["error"] = json!("Your session limit resets at noon");
    a["errorAt"] = json!(100);
    a["lastActivityAt"] = json!(200);
    a["completedTurns"] = json!(50);
    f.put(&f.a, &a, true);
    let mut b = a.clone();
    b["lastActivityAt"] = json!(101);
    b["completedTurns"] = json!(45);
    b["priorErrorMark"] = json!({"at":100,"source":"synthetic"});
    b["interruptedByQuitAt"] = json!(102);
    b["interruptedUnseenResume"] = json!(true);
    f.put(&f.b, &b, false);
    let before = fs::read(f.path(&f.b, 10)).unwrap();
    let p = f.preview();
    assert!(p.issues.is_empty());
    assert_eq!(p.quota_pauses_cleared, 1);
    let run = f.apply();
    let out = f.get(&f.b, 10);
    assert_eq!(out["completedTurns"], 50);
    assert_eq!(out["priorErrorMark"], b["priorErrorMark"]);
    assert_eq!(out["permissionMode"], "auto");
    for key in [
        "error",
        "errorAt",
        "interruptedByQuitAt",
        "interruptedUnseenResume",
    ] {
        assert!(out.get(key).is_none());
    }
    assert_eq!(f.preview().quota_pauses_cleared, 0);
    f.undo(&run);
    assert_eq!(fs::read(f.path(&f.b, 10)).unwrap(), before);
}

#[test]
fn materialize_does_not_broaden_quota_cleanup_for_unrecognized_context() {
    let mut source = record(10);
    source["lastActivityAt"] = json!(200);
    for patch in [
        json!({"error":"Authentication failed"}),
        json!({"error":"Session limit reached. Resets at noon"}),
        json!({"errorCategory":"auth"}),
        json!({"tccFolderKind":"documents"}),
        json!({"armedWorkAtQuit":{}}),
        json!({"pendingFirstStart":{}}),
        json!({"errorAt":300}),
        json!({"errorAt":null}),
        json!({"error":"Your session limits resets soon"}),
    ] {
        let mut target = record(10);
        target["error"] = json!("Your session limit resets at noon");
        target["errorAt"] = json!(100);
        target["interruptedByQuitAt"] = json!(101);
        target
            .as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        let out = materialize(&portable(&source), &source, Some(&target));
        assert_eq!(out["error"], target["error"]);
        assert_eq!(out["errorAt"], target["errorAt"]);
        assert_eq!(out["interruptedByQuitAt"], 101);
    }
}

#[test]
fn missing_empty_and_duplicate_transcripts_are_excluded_and_duplicate_addition_invalidates_preview()
{
    let f = Fixture::new();
    for n in 10..13 {
        f.put(&f.a, &record(n), n != 10);
    }
    fs::write(f.transcript(&record(11)), []).unwrap();
    let other = f.roots.pool.join("other");
    Directory::open(&other, true).unwrap();
    fs::copy(
        f.transcript(&record(12)),
        other.join(format!("{}.jsonl", id(112))),
    )
    .unwrap();
    let p = f.preview();
    assert_eq!(p.created, 0);
    assert!(issue(&p, 10, "MISSING_TRANSCRIPT"));
    assert!(issue(&p, 11, "EMPTY_TRANSCRIPT"));
    assert!(issue(&p, 12, "AMBIGUOUS_TRANSCRIPT"));
    fs::remove_file(other.join(format!("{}.jsonl", id(112)))).unwrap();
    let fingerprint = f.preview().fingerprint;
    fs::copy(
        f.transcript(&record(12)),
        other.join(format!("{}.jsonl", id(112))),
    )
    .unwrap();
    assert_eq!(
        apply(&f.roots, &f.a, &f.b, &fingerprint, &f.backup(), &mut || Ok(
            ()
        ))
        .unwrap_err(),
        "PREVIEW_CHANGED"
    );
}

#[test]
fn target_deletion_is_not_resurrected_and_source_deletion_never_propagates() {
    let f = Fixture::new();
    for n in 10..13 {
        f.put(&f.a, &record(n), true);
    }
    f.apply();
    fs::remove_file(f.path(&f.b, 10)).unwrap();
    fs::remove_file(f.path(&f.a, 11)).unwrap();
    let target_only = required_blob(&f.path(&f.b, 11)).unwrap().stamp;
    let p = f.preview();
    assert!(issue(&p, 10, "PREVIOUSLY_SYNCED_TARGET_MISSING"));
    f.apply();
    assert!(!f.path(&f.b, 10).exists());
    assert_eq!(required_blob(&f.path(&f.b, 11)).unwrap().stamp, target_only);
}

#[test]
fn tasks_remote_and_dispatch_relationships_are_excluded_on_both_sides() {
    for target_side in [false, true] {
        for patch in [
            json!({"spawnedFrom":{"taskId":"synthetic"}}),
            json!({"scheduledTaskId":"synthetic"}),
            json!({"notifySessionId":sid(99)}),
            json!({"dispatchParentId":sid(99)}),
            json!({"dispatchParentOrigin":"scheduled"}),
            json!({"sshConfig":{"host":"synthetic"}}),
            json!({"wslConfig":{"distro":"synthetic"}}),
            json!({"backend":"remote"}),
            json!({"cloudSessionId":"synthetic"}),
            json!({"remoteControlSpawn":true}),
        ] {
            let f = Fixture::new();
            let plain = record(10);
            let mut excluded = plain.clone();
            excluded
                .as_object_mut()
                .unwrap()
                .extend(patch.as_object().unwrap().clone());
            f.put(&f.a, if target_side { &plain } else { &excluded }, true);
            if target_side {
                f.put(&f.b, &excluded, false);
            }
            let p = f.preview();
            assert_eq!((p.created, p.updated, p.issues.len()), (0, 0, 1), "{patch}");
        }
    }
}

#[test]
fn excluded_parent_closure_covers_existing_destination_parents_and_nested_children() {
    let f = Fixture::new();
    for n in 10..13 {
        let mut r = record(n);
        if n != 10 {
            r["forkedFromSessionId"] = json!(sid(n - 1));
        }
        f.put(&f.a, &r, true);
    }
    f.put(&f.b, &record(10), false);
    fs::write(
        f.dir(&f.b).join("scheduled-tasks.json"),
        json_bytes(&json!({"scheduledTasks":[{"notifySessionId":sid(10)}]})).unwrap(),
    )
    .unwrap();
    let p = f.preview();
    assert_eq!(p.created, 0);
    assert_eq!(p.issues.len(), 3);
    assert!(issue(&p, 11, "EXCLUDED_PARENT"));
    assert!(issue(&p, 12, "EXCLUDED_PARENT"));
}

#[test]
fn dispatch_child_also_protects_its_parent_and_compatible_missing_lineage_is_preserved() {
    let f = Fixture::new();
    f.put(&f.a, &record(10), true);
    let mut child = record(11);
    child["dispatchParentId"] = json!(sid(10));
    f.put(&f.b, &child, true);
    assert!(issue(&f.preview(), 10, "OWNED_OR_SPAWNED_SESSION"));
    let mut ordinary = record(12);
    ordinary["forkedFromSessionId"] = json!(sid(999));
    ordinary["forkedAtMessageUuid"] = json!(id(888));
    f.put(&f.a, &ordinary, true);
    let p = f.preview();
    assert_eq!(p.created, 1);
    assert_eq!(p.warnings[0].reason, "PREEXISTING_MISSING_PARENT");
    f.apply();
    assert_eq!(f.get(&f.b, 12)["forkedFromSessionId"], sid(999));
    assert_eq!(f.get(&f.b, 12)["forkedAtMessageUuid"], id(888));
}

#[test]
fn unknown_nonempty_task_registry_fails_closed_and_raw_registry_drift_is_bound() {
    let f = Fixture::new();
    f.put(&f.a, &record(10), true);
    let file = f.dir(&f.a).join("scheduled-tasks.json");
    for body in [
        json!([{"unknown":true}]),
        json!({"tasks":[]}),
        json!({"scheduledTasks":[{}]}),
        json!({"scheduledTasks":[{"notifySessionId":"../bad"}]}),
    ] {
        fs::write(&file, json_bytes(&body).unwrap()).unwrap();
        assert_eq!(
            preview(&f.roots, &f.a, &f.b).unwrap_err(),
            "TASK_REGISTRY_REQUIRES_REVIEW"
        );
    }
    fs::write(&file, b"{\"scheduledTasks\":[]}").unwrap();
    let before = f.preview().fingerprint;
    fs::write(&file, b"{ \"scheduledTasks\": [] }\n").unwrap();
    assert_ne!(before, f.preview().fingerprint);
}

#[test]
fn symlink_ancestors_records_transcripts_and_state_are_never_followed() {
    for kind in [
        "ancestor",
        "record",
        "transcript",
        "state",
        "registry",
        "pool",
    ] {
        let f = Fixture::new();
        f.put(&f.a, &record(10), true);
        let mut roots = f.roots.clone();
        match kind {
            "ancestor" => {
                let alias = f.root.join("alias");
                symlink(&f.roots.records, &alias).unwrap();
                roots.records = alias;
            }
            "record" => {
                symlink(f.path(&f.a, 10), f.path(&f.b, 10)).unwrap();
            }
            "transcript" => {
                let p = f.transcript(&record(10));
                fs::remove_file(&p).unwrap();
                symlink(f.path(&f.a, 10), p).unwrap();
            }
            "state" => {
                let elsewhere = f.root.join("elsewhere");
                fs::create_dir(&elsewhere).unwrap();
                symlink(elsewhere, &f.roots.state).unwrap();
            }
            "registry" => {
                symlink(f.path(&f.a, 10), f.dir(&f.a).join("scheduled-tasks.json")).unwrap();
            }
            "pool" => {
                symlink(f.roots.pool.join("project"), f.roots.pool.join("alias")).unwrap();
            }
            _ => unreachable!(),
        }
        assert!(preview(&roots, &f.a, &f.b).is_err(), "{kind}");
    }
}

#[test]
fn traversal_missing_namespaces_overlapping_roots_and_insecure_state_are_refused() {
    let f = Fixture::new();
    f.put(&f.a, &record(10), true);
    let mut bad = f.a.clone();
    bad.account = "../records".into();
    assert_eq!(
        preview(&f.roots, &bad, &f.b).unwrap_err(),
        "INVALID_IDENTITY"
    );
    let mut roots = f.roots.clone();
    roots.pool = f.root.join("pool/../pool");
    assert_eq!(preview(&roots, &f.a, &f.b).unwrap_err(), "UNSAFE_PATH");
    roots = f.roots.clone();
    roots.state = roots.records.join("state");
    assert_eq!(
        preview(&roots, &f.a, &f.b).unwrap_err(),
        "OVERLAPPING_ROOTS"
    );
    let absent = Identity {
        account: id(5),
        org: id(6),
    };
    assert_eq!(
        preview(&f.roots, &f.a, &absent).unwrap_err(),
        "MISSING_PATH"
    );
    assert!(!f.dir(&absent).exists());
    fs::create_dir(&f.roots.state).unwrap();
    fs::set_permissions(&f.roots.state, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(f.preview_error(), "INSECURE_DIRECTORY");
}

impl Fixture {
    fn preview_error(&self) -> String {
        preview(&self.roots, &self.a, &self.b).unwrap_err()
    }
}

#[test]
fn source_target_registry_transcript_and_baseline_drift_after_guard_prevent_first_write() {
    for kind in [
        "source",
        "target",
        "registry",
        "transcript",
        "baseline",
        "duplicate",
    ] {
        let f = Fixture::new();
        f.put(&f.a, &record(10), true);
        let p = f.preview();
        let mut calls = 0;
        let error = apply(
            &f.roots,
            &f.a,
            &f.b,
            &p.fingerprint,
            &f.backup(),
            &mut || {
                calls += 1;
                if calls == 2 {
                    match kind {
                        "source" => {
                            let mut r = record(10);
                            r["title"] = json!("drift");
                            f.put(&f.a, &r, false);
                        }
                        "target" => f.put(&f.b, &record(99), true),
                        "registry" => {
                            fs::write(f.dir(&f.b).join("scheduled-tasks.json"), b"[]").unwrap()
                        }
                        "transcript" => {
                            fs::write(f.transcript(&record(10)), b"synthetic drift\n").unwrap()
                        }
                        "baseline" => {
                            let path = baseline_path(&f.roots, &f.a, &f.b).unwrap();
                            Directory::open(path.parent().unwrap(), true).unwrap();
                            fs::write(path, b"{}").unwrap();
                        }
                        "duplicate" => {
                            let other = f.roots.pool.join("other");
                            fs::create_dir(&other).unwrap();
                            fs::copy(
                                f.transcript(&record(10)),
                                other.join(format!("{}.jsonl", id(110))),
                            )
                            .unwrap();
                        }
                        _ => unreachable!(),
                    }
                }
                Ok(())
            },
        )
        .unwrap_err();
        assert!(
            [
                "SNAPSHOT_DRIFT",
                "REGISTRY_DRIFT",
                "TRANSCRIPT_DRIFT",
                "BASELINE_DRIFT"
            ]
            .contains(&error.as_str()),
            "{kind}: {error}"
        );
        assert!(!f.path(&f.b, 10).exists());
    }
}

#[test]
fn concurrent_file_lock_covers_apply_baseline_finalization_and_rollback() {
    let f = Fixture::new();
    f.put(&f.a, &record(10), true);
    let p = f.preview();
    let mut probes = 0;
    let run = apply(
        &f.roots,
        &f.a,
        &f.b,
        &p.fingerprint,
        &f.backup(),
        &mut || {
            probes += 1;
            assert!(matches!(Lock::acquire(&f.roots.state), Err(e) if e == "LOCKED"));
            Ok(())
        },
    )
    .unwrap();
    assert!(probes >= 6);
    let mut probes = 0;
    rollback(&f.roots, &run.id, &mut || {
        probes += 1;
        assert!(matches!(Lock::acquire(&f.roots.state), Err(e) if e == "LOCKED"));
        Ok(())
    })
    .unwrap();
    assert!(probes >= 4);
    assert!(Lock::acquire(&f.roots.state).is_ok());
}

#[test]
fn every_apply_guard_interruption_is_recoverable_and_pending_blocks_new_runs() {
    for stop in 2..=6 {
        let f = Fixture::new();
        f.put(&f.a, &record(10), true);
        let p = f.preview();
        let mut calls = 0;
        let error = apply(
            &f.roots,
            &f.a,
            &f.b,
            &p.fingerprint,
            &f.backup(),
            &mut || {
                calls += 1;
                if calls == stop {
                    Err("synthetic private error text".into())
                } else {
                    Ok(())
                }
            },
        )
        .unwrap_err();
        assert_eq!(error, "GUARD_FAILED");
        let run = f.pending().summary;
        assert_eq!(
            apply(
                &f.roots,
                &f.a,
                &f.b,
                &p.fingerprint,
                &f.backup(),
                &mut || Ok(())
            )
            .unwrap_err(),
            "PENDING_RUN"
        );
        f.undo(&run);
        assert!(!f.path(&f.b, 10).exists());
        assert!(!baseline_path(&f.roots, &f.a, &f.b).unwrap().exists());
        assert_eq!(f.undo(&run).state, "rolled_back");
        f.apply();
    }
}

#[test]
fn failed_apply_exposes_only_its_published_journal_for_automatic_recovery() {
    let f = Fixture::new();
    f.put(&f.a, &record(10), true);
    let p = f.preview();
    let mut published = None;
    let mut guards = 0;
    let error = apply_observed(
        &f.roots,
        &f.a,
        &f.b,
        &p.fingerprint,
        &f.backup(),
        &mut || {
            guards += 1;
            if guards == 3 {
                Err("synthetic writer".into())
            } else {
                Ok(())
            }
        },
        None,
        &mut |id| published = Some(id.to_owned()),
    )
    .unwrap_err();
    assert_eq!(error, "GUARD_FAILED");
    let id = published.expect("published journal id");
    assert_eq!(f.pending().summary.id, id);
    record_failure(&f.roots, &id, &error).unwrap();
    assert_eq!(
        load(&f.roots, &id).unwrap().summary.last_error.as_deref(),
        Some("GUARD_FAILED")
    );
    assert_eq!(
        rollback(&f.roots, &id, &mut || Ok(())).unwrap().state,
        "rolled_back"
    );
    assert!(!f.path(&f.b, 10).exists());
    assert!(require_no_pending(&f.roots).is_ok());
}

#[test]
fn optimized_apply_limits_full_guards_to_transaction_boundaries() {
    let f = Fixture::new();
    f.put(&f.a, &record(10), true);
    f.put(&f.a, &record(11), true);
    let p = f.preview();
    let mut full_calls = 0;
    let mut quick_calls = 0;
    let run = apply_observed(
        &f.roots,
        &f.a,
        &f.b,
        &p.fingerprint,
        &f.backup(),
        &mut || {
            full_calls += 1;
            Ok(())
        },
        Some(&mut || {
            quick_calls += 1;
            Ok(())
        }),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(run.state, "applied");
    assert_eq!(full_calls, 2);
    assert_eq!(quick_calls, 2 * 3); // two records and one baseline
}

#[test]
fn default_profile_startup_guard_blocks_pending_journals_but_keeps_custom_profiles_independent() {
    let f = Fixture::new();
    let default = f.root.join("default-profile");
    let custom = f.root.join("custom-profile");
    fs::create_dir(&default).unwrap();
    fs::create_dir(&custom).unwrap();
    f.put(&f.a, &record(10), true);
    let applied = f.apply();
    let mut journal = load(&f.roots, &applied.id).unwrap();
    journal.summary.state = "applying".into();
    save(&f.roots, &journal).unwrap();
    let ready = super::super::require_no_pending_for_profile;
    assert_eq!(
        ready(&default, &default, &f.roots.state),
        Err("PENDING_RUN".into())
    );
    assert_eq!(ready(&custom, &default, &f.roots.state), Ok(()));
    #[cfg(unix)]
    {
        let alias = f.root.join("default-alias");
        symlink(&default, &alias).unwrap();
        assert_eq!(
            ready(&alias, &default, &f.roots.state),
            Err("PENDING_RUN".into())
        );
    }
}

#[test]
fn desktop_switch_gate_reads_pending_state_without_profile_roots() {
    let f = Fixture::new();
    assert!(require_no_pending_state(&f.roots.state).is_ok());
    f.put(&f.a, &record(10), true);
    let applied = f.apply();
    assert!(require_no_pending_state(&f.roots.state).is_ok());
    let mut journal = load(&f.roots, &applied.id).unwrap();
    journal.summary.state = "applying".into();
    save(&f.roots, &journal).unwrap();
    assert_eq!(
        require_no_pending_state(&f.roots.state),
        Err("PENDING_RUN".into())
    );
}

#[test]
fn crash_after_publication_before_completion_receipt_recovers_records_and_baseline() {
    for crash_baseline in [false, true] {
        let f = Fixture::new();
        f.put(&f.a, &record(10), true);
        let p = f.preview();
        let error = apply(
            &f.roots,
            &f.a,
            &f.b,
            &p.fingerprint,
            &f.backup(),
            &mut || {
                let Ok(runs) = list_runs(&f.roots) else {
                    return Ok(());
                };
                let Some(run) = runs.first() else {
                    return Ok(());
                };
                let j = load(&f.roots, &run.id)?;
                if let Some(op) = j
                    .operations
                    .iter()
                    .find(|op| op.baseline == crash_baseline && op.phase == "write_pending")
                {
                    let path = operation_path(&f.roots, &j, op)?;
                    let after = Stamp {
                        hash: op.after_hash.clone(),
                        id: op.write_id.clone().unwrap(),
                    };
                    publish_operation(&path, &op.temp, &op.before, &after)?;
                    return Err("SIMULATED_CRASH".into());
                }
                Ok(())
            },
        )
        .unwrap_err();
        assert_eq!(error, "SIMULATED_CRASH");
        let j = f.pending();
        assert_ne!(j.summary.state, "applied");
        assert_eq!(
            baseline_path(&f.roots, &f.a, &f.b).unwrap().exists(),
            crash_baseline
        );
        f.undo(&j.summary);
        assert!(!f.path(&f.b, 10).exists());
        assert!(!baseline_path(&f.roots, &f.a, &f.b).unwrap().exists());
    }
}

#[test]
fn crash_after_baseline_commit_restores_previous_baseline_and_exact_record_preimages() {
    let f = Fixture::new();
    f.put(&f.a, &record(10), true);
    f.apply();
    let baseline = baseline_path(&f.roots, &f.a, &f.b).unwrap();
    let old_base = fs::read(&baseline).unwrap();
    let old_target = fs::read(f.path(&f.b, 10)).unwrap();
    let mut a = record(10);
    a["title"] = json!("new source title");
    f.put(&f.a, &a, false);
    let p = f.preview();
    let error = apply(
        &f.roots,
        &f.a,
        &f.b,
        &p.fingerprint,
        &f.backup(),
        &mut || {
            if let Ok(runs) = list_runs(&f.roots) {
                for run in runs.iter().filter(|r| r.state == "applying") {
                    let j = load(&f.roots, &run.id)?;
                    if j.operations.iter().all(|o| o.phase == "written") {
                        return Err("SIMULATED_CRASH".into());
                    }
                }
            }
            Ok(())
        },
    )
    .unwrap_err();
    assert_eq!(error, "SIMULATED_CRASH");
    assert_ne!(fs::read(&baseline).unwrap(), old_base);
    let pending = f.pending();
    f.undo(&pending.summary);
    assert_eq!(fs::read(&baseline).unwrap(), old_base);
    assert_eq!(fs::read(f.path(&f.b, 10)).unwrap(), old_target);
    assert_eq!(f.get(&f.a, 10), a);
}

#[test]
fn conditional_undo_checks_all_records_and_baseline_before_any_undo() {
    for change in ["first", "last", "baseline", "backup"] {
        let f = Fixture::new();
        if change == "backup" {
            // A previous completed run gives the next run a baseline preimage.
            f.put(&f.a, &record(9), true);
            f.put(&f.b, &record(9), false);
            f.apply();
        }
        for n in 10..13 {
            f.put(&f.a, &record(n), true);
        }
        let run = f.apply();
        let baseline = baseline_path(&f.roots, &f.a, &f.b).unwrap();
        let untouched = fs::read(f.path(&f.b, 11)).unwrap();
        let original_baseline = fs::read(&baseline).unwrap();
        let path = match change {
            "first" => f.path(&f.b, 10),
            "last" => f.path(&f.b, 12),
            "baseline" => baseline.clone(),
            "backup" => Path::new(&run.backup_dir).join("baseline.json"),
            _ => unreachable!(),
        };
        fs::write(path, b"synthetic new edit").unwrap();
        let error = rollback(&f.roots, &run.id, &mut || Ok(())).unwrap_err();
        assert!(["ROLLBACK_DRIFT", "BACKUP_DRIFT"].contains(&error.as_str()));
        assert_eq!(fs::read(f.path(&f.b, 11)).unwrap(), untouched);
        if change != "baseline" {
            assert_eq!(fs::read(&baseline).unwrap(), original_baseline);
        }
    }
}

#[test]
fn same_bytes_replacement_is_refused_for_created_updated_and_baseline_files() {
    for kind in ["created", "updated", "baseline"] {
        let f = Fixture::new();
        f.put(&f.a, &record(10), true);
        if kind == "updated" {
            let mut target = record(10);
            target["lastActivityAt"] = json!(1);
            f.put(&f.b, &target, false);
        }
        let run = f.apply();
        let path = if kind == "baseline" {
            baseline_path(&f.roots, &f.a, &f.b).unwrap()
        } else {
            f.path(&f.b, 10)
        };
        let original = required_blob(&path).unwrap();
        let replacement = f.root.join("replacement");
        fs::write(&replacement, &original.bytes).unwrap();
        fs::rename(replacement, &path).unwrap();
        assert_ne!(required_blob(&path).unwrap().stamp.id, original.stamp.id);
        assert_eq!(
            rollback(&f.roots, &run.id, &mut || Ok(())).unwrap_err(),
            "ROLLBACK_DRIFT"
        );
        assert_eq!(fs::read(path).unwrap(), original.bytes);
    }
}

#[test]
fn missing_backup_or_replaced_backup_directory_refuses_recovery_without_mutation() {
    for replace in [false, true] {
        let f = Fixture::new();
        f.put(&f.a, &record(10), true);
        let run = f.apply();
        let target = required_blob(&f.path(&f.b, 10)).unwrap().stamp;
        fs::rename(&run.backup_dir, f.root.join("moved-backup")).unwrap();
        if replace {
            Directory::open(Path::new(&run.backup_dir), true).unwrap();
        }
        let error = rollback(&f.roots, &run.id, &mut || Ok(())).unwrap_err();
        assert_eq!(
            error,
            if replace {
                "BACKUP_REPLACED"
            } else {
                "BACKUP_UNAVAILABLE"
            }
        );
        assert_eq!(required_blob(&f.path(&f.b, 10)).unwrap().stamp, target);
        assert!(journal_path(&f.roots, &run.id).exists());
    }
}

#[test]
fn interrupted_rollback_resumes_and_restoration_replacement_is_refused() {
    for replacement in [false, true] {
        let f = Fixture::new();
        for n in 10..12 {
            let source = record(n);
            let mut target = source.clone();
            target["lastActivityAt"] = json!(1);
            f.put(&f.a, &source, true);
            f.put(&f.b, &target, false);
        }
        let old = fs::read(f.path(&f.b, 11)).unwrap();
        let run = f.apply();
        let mut calls = 0;
        assert_eq!(
            rollback(&f.roots, &run.id, &mut || {
                calls += 1;
                if calls == 6 {
                    Err("SIMULATED_CRASH".into())
                } else {
                    Ok(())
                }
            })
            .unwrap_err(),
            "SIMULATED_CRASH"
        );
        assert_eq!(fs::read(f.path(&f.b, 11)).unwrap(), old);
        if replacement {
            let path = f.root.join("replacement");
            fs::write(&path, &old).unwrap();
            fs::rename(path, f.path(&f.b, 11)).unwrap();
            assert_eq!(
                rollback(&f.roots, &run.id, &mut || Ok(())).unwrap_err(),
                "ROLLBACK_DRIFT"
            );
        } else {
            assert_eq!(f.undo(&run).state, "rolled_back");
            assert_eq!(f.get(&f.b, 10)["lastActivityAt"], 1);
        }
    }
}

#[test]
fn crash_after_undo_publish_before_journal_completion_is_recoverable() {
    let f = Fixture::new();
    f.put(&f.a, &record(10), true);
    let mut target = record(10);
    target["lastActivityAt"] = json!(1);
    f.put(&f.b, &target, false);
    let before = fs::read(f.path(&f.b, 10)).unwrap();
    let run = f.apply();
    let error = rollback(&f.roots, &run.id, &mut || {
        let j = load(&f.roots, &run.id)?;
        if let Some(op) = j
            .operations
            .iter()
            .find(|o| !o.baseline && o.phase == "undo_pending")
        {
            let path = operation_path(&f.roots, &j, op)?;
            let expected = Some(Stamp {
                hash: op.after_hash.clone(),
                id: op.write_id.clone().unwrap(),
            });
            let restored = Stamp {
                hash: op.before.as_ref().unwrap().hash.clone(),
                id: op.undo_id.clone().unwrap(),
            };
            publish_operation(&path, &op.undo_temp, &expected, &restored)?;
            return Err("SIMULATED_CRASH".into());
        }
        Ok(())
    })
    .unwrap_err();
    assert_eq!(error, "SIMULATED_CRASH");
    f.undo(&run);
    assert_eq!(fs::read(f.path(&f.b, 10)).unwrap(), before);
}

#[test]
fn unjournaled_undo_stage_does_not_block_safe_recovery_or_get_overwritten() {
    let f = Fixture::new();
    f.put(&f.a, &record(10), true);
    let mut target = record(10);
    target["lastActivityAt"] = json!(1);
    f.put(&f.b, &target, false);
    let run = f.apply();
    let j = load(&f.roots, &run.id).unwrap();
    let op = j.operations.iter().find(|o| !o.baseline).unwrap();
    let path = operation_path(&f.roots, &j, op)
        .unwrap()
        .with_file_name(&op.undo_temp);
    fs::write(&path, b"unowned synthetic stage").unwrap();
    f.undo(&run);
    assert_eq!(fs::read(path).unwrap(), b"unowned synthetic stage");
    assert_eq!(f.get(&f.b, 10)["lastActivityAt"], 1);
}

#[test]
fn later_baseline_run_prevents_out_of_order_undo_until_verified_later_undo() {
    let f = Fixture::new();
    f.put(&f.a, &record(10), true);
    let first = f.apply();
    let second = f.apply();
    let before = fs::read(f.path(&f.b, 10)).unwrap();
    assert_eq!(
        rollback(&f.roots, &first.id, &mut || Ok(())).unwrap_err(),
        "ROLLBACK_DRIFT"
    );
    assert_eq!(fs::read(f.path(&f.b, 10)).unwrap(), before);
    f.undo(&second);
    // A completed newer rollback records the new baseline inode and its exact
    // predecessor, so the earlier run can now be undone without trusting bytes alone.
    assert_eq!(f.undo(&first).state, "rolled_back");
    assert!(!f.path(&f.b, 10).exists());
}

#[test]
fn invalid_run_traversal_and_incomplete_run_directory_fail_closed() {
    let f = Fixture::new();
    f.put(&f.a, &record(10), true);
    assert_eq!(
        rollback(&f.roots, "../bad", &mut || Ok(())).unwrap_err(),
        "INVALID_RUN_ID"
    );
    Directory::open(&f.roots.state.join(format!("run-{}", Uuid::new_v4())), true).unwrap();
    assert_eq!(f.preview_error(), "PENDING_RUN");
}

#[test]
fn api_preview_can_show_local_titles_but_operational_summaries_and_journals_omit_them() {
    let f = Fixture::new();
    f.put(&f.a, &record(10), false);
    let preview = serde_json::to_value(f.preview()).unwrap();
    assert!(preview.get("quotaPausesCleared").is_some());
    assert!(preview.get("baselineChanged").is_some());
    assert_eq!(preview["issues"][0]["sessionId"], sid(10));
    assert_eq!(preview["issues"][0]["title"], "Synthetic 10");
    f.put(&f.a, &record(10), true);
    let run = f.apply();
    let serialized = serde_json::to_value(&run).unwrap();
    assert!(serialized.get("createdAt").is_some());
    assert!(serialized.get("backupDir").is_some());
    assert!(!serialized.to_string().contains("Synthetic 10"));
    let journal = fs::read_to_string(journal_path(&f.roots, &run.id)).unwrap();
    assert!(!journal.contains("Synthetic 10"));
    assert_eq!(
        guard_call(&mut || Err("Synthetic 10".into())).unwrap_err(),
        "GUARD_FAILED"
    );
}

#[test]
fn equal_existing_records_can_commit_initial_baseline_then_reverse_pointer_rollover() {
    let f = Fixture::new();
    let original = record(10);
    f.put(&f.a, &original, true);
    f.put(&f.b, &original, false);
    let p = f.preview();
    assert_eq!((p.created, p.updated, p.unchanged), (0, 0, 1));
    assert!(p.baseline_changed);
    let before = required_blob(&f.path(&f.b, 10)).unwrap().stamp;
    f.apply();
    assert_eq!(required_blob(&f.path(&f.b, 10)).unwrap().stamp, before);
    assert!(!f.preview().baseline_changed);
    let mut advanced = original.clone();
    advanced["cliSessionId"] = json!(id(999));
    advanced["priorCliSessionIds"] = json!([original["cliSessionId"]]);
    advanced["lastActivityAt"] = json!(100);
    advanced["completedTurns"] = json!(10);
    f.put(&f.b, &advanced, true);
    let reverse = preview(&f.roots, &f.b, &f.a).unwrap();
    assert_eq!(reverse.updated, 1);
    assert!(reverse.issues.is_empty());
    f.transfer(&f.b, &f.a);
    assert_eq!(f.get(&f.a, 10)["cliSessionId"], id(999));
    assert_eq!(f.get(&f.a, 10)["priorCliSessionIds"], json!([id(110)]));
}

#[test]
fn separate_thread_cannot_enter_apply_while_first_transaction_holds_lock() {
    let f = Fixture::new();
    f.put(&f.a, &record(10), true);
    let p = f.preview();
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        let fixture = &f;
        let fingerprint = &p.fingerprint;
        let holder = scope.spawn(move || {
            let mut entered = false;
            apply(
                &fixture.roots,
                &fixture.a,
                &fixture.b,
                fingerprint,
                &fixture.backup(),
                &mut || {
                    if !entered {
                        entered = true;
                        entered_tx.send(()).map_err(|_| "TEST_CHANNEL_CLOSED")?;
                        release_rx
                            .recv_timeout(std::time::Duration::from_secs(5))
                            .map_err(|_| "TEST_CHANNEL_TIMEOUT")?;
                    }
                    Ok(())
                },
            )
        });
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let error = apply(
            &f.roots,
            &f.a,
            &f.b,
            &p.fingerprint,
            &f.backup(),
            &mut || Ok(()),
        )
        .unwrap_err();
        release_tx.send(()).unwrap();
        assert_eq!(error, "LOCKED");
        assert_eq!(holder.join().unwrap().unwrap().state, "applied");
    });
}

fn unrelated_pair(f: &Fixture) -> (Identity, Identity) {
    let source = Identity {
        account: id(5),
        org: id(6),
    };
    let target = Identity {
        account: id(7),
        org: id(8),
    };
    Directory::open(&f.dir(&source), true).unwrap();
    Directory::open(&f.dir(&target), true).unwrap();
    f.put(&source, &record(20), true);
    (source, target)
}

#[test]
fn completed_history_survives_missing_namespaces_and_allows_unrelated_pair() {
    for undone in [false, true] {
        let f = Fixture::new();
        f.put(&f.a, &record(10), true);
        let run = f.apply();
        let completed = if undone { f.undo(&run) } else { run };
        fs::remove_dir_all(f.dir(&f.a)).unwrap();
        let history = list_runs(&f.roots).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].id, completed.id);
        assert_eq!(history[0].state, completed.state);
        fs::remove_dir_all(f.dir(&f.b)).unwrap();
        assert_eq!(list_runs(&f.roots).unwrap().len(), 1);
        assert_eq!(preview(&f.roots, &f.a, &f.b).unwrap_err(), "MISSING_PATH");

        let (source, target) = unrelated_pair(&f);
        let p = preview(&f.roots, &source, &target).unwrap();
        assert_eq!(p.created, 1);
        assert!(p.issues.is_empty());
        assert_eq!(f.transfer(&source, &target).state, "applied");
        assert_eq!(list_runs(&f.roots).unwrap().len(), 2);
    }
}

#[test]
fn completed_history_remains_visible_without_pool_but_apply_still_requires_it() {
    for undone in [false, true] {
        let f = Fixture::new();
        f.put(&f.a, &record(10), true);
        let run = f.apply();
        let completed = if undone { f.undo(&run) } else { run };
        let p = f.preview();
        fs::remove_dir_all(&f.roots.pool).unwrap();
        let history = list_runs(&f.roots).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].id, completed.id);
        assert_eq!(history[0].state, completed.state);
        no_pending(&f.roots, None).unwrap();
        assert_eq!(preview(&f.roots, &f.a, &f.b).unwrap_err(), "MISSING_PATH");
        let backup = f.backup();
        assert_eq!(
            apply(
                &f.roots,
                &f.a,
                &f.b,
                &p.fingerprint,
                &backup,
                &mut || Ok(())
            )
            .unwrap_err(),
            "MISSING_PATH"
        );
        assert!(!backup.exists());
        assert!(!f.roots.pool.exists());
    }
}

#[test]
fn rollback_needs_target_images_and_backups_but_not_untouched_source_or_pool() {
    let f = Fixture::new();
    f.put(&f.a, &record(10), true);
    let run = f.apply();
    fs::remove_dir_all(f.dir(&f.a)).unwrap();
    fs::remove_dir_all(&f.roots.pool).unwrap();
    assert_eq!(f.undo(&run).state, "rolled_back");
    assert!(!f.path(&f.b, 10).exists());
    assert!(!baseline_path(&f.roots, &f.a, &f.b).unwrap().exists());
    assert!(!f.dir(&f.a).exists());
    assert!(!f.roots.pool.exists());
    assert_eq!(list_runs(&f.roots).unwrap()[0].state, "rolled_back");
}

#[test]
fn rollback_missing_or_symlink_target_refuses_before_any_undo_even_for_baseline_only_run() {
    for baseline_only in [false, true] {
        for symlink_target in [false, true] {
            let f = Fixture::new();
            f.put(&f.a, &record(10), true);
            if baseline_only {
                f.put(&f.b, &record(10), false);
            }
            let run = f.apply();
            assert_eq!(run.created, usize::from(!baseline_only));
            assert_eq!(run.updated, 0);
            let baseline = baseline_path(&f.roots, &f.a, &f.b).unwrap();
            let baseline_before = required_blob(&baseline).unwrap().stamp;
            let journal_before = required_blob(&journal_path(&f.roots, &run.id))
                .unwrap()
                .stamp;
            let source_before = required_blob(&f.path(&f.a, 10)).unwrap().stamp;
            let target_before = required_blob(&f.path(&f.b, 10)).unwrap().stamp;
            let retired = f.root.join("retired-target");
            fs::rename(f.dir(&f.b), &retired).unwrap();
            if symlink_target {
                symlink(&retired, f.dir(&f.b)).unwrap();
            }

            assert_eq!(list_runs(&f.roots).unwrap()[0].state, "applied");
            let error = rollback(&f.roots, &run.id, &mut || Ok(())).unwrap_err();
            assert_eq!(
                error,
                if symlink_target {
                    "UNSAFE_PATH"
                } else {
                    "ROLLBACK_TARGET_MISSING"
                }
            );
            assert_eq!(required_blob(&baseline).unwrap().stamp, baseline_before);
            assert_eq!(
                required_blob(&journal_path(&f.roots, &run.id))
                    .unwrap()
                    .stamp,
                journal_before
            );
            assert_eq!(
                required_blob(&f.path(&f.a, 10)).unwrap().stamp,
                source_before
            );
            assert_eq!(
                required_blob(&retired.join(name(10))).unwrap().stamp,
                target_before
            );
            if !symlink_target {
                assert!(!f.dir(&f.b).exists());
            }
        }
    }
}

#[test]
fn rollback_rechecks_target_namespace_after_guard_before_baseline_undo() {
    let f = Fixture::new();
    f.put(&f.a, &record(10), true);
    f.put(&f.b, &record(10), false);
    let run = f.apply();
    let baseline = baseline_path(&f.roots, &f.a, &f.b).unwrap();
    let baseline_before = required_blob(&baseline).unwrap().stamp;
    let target_before = required_blob(&f.path(&f.b, 10)).unwrap().stamp;
    let retired = f.root.join("retired-target");
    let mut calls = 0;
    assert_eq!(
        rollback(&f.roots, &run.id, &mut || {
            calls += 1;
            if calls == 2 {
                fs::rename(f.dir(&f.b), &retired).unwrap();
            }
            Ok(())
        })
        .unwrap_err(),
        "ROLLBACK_TARGET_MISSING"
    );
    assert_eq!(required_blob(&baseline).unwrap().stamp, baseline_before);
    assert_eq!(
        required_blob(&retired.join(name(10))).unwrap().stamp,
        target_before
    );
    assert_eq!(list_runs(&f.roots).unwrap()[0].state, "rolling_back");
}

#[test]
fn missing_old_source_never_bypasses_pending_or_corrupt_journal_validation() {
    for fault in ["pending", "json", "identity", "roots", "operation"] {
        let f = Fixture::new();
        f.put(&f.a, &record(10), true);
        let run = f.apply();
        let mut journal = load(&f.roots, &run.id).unwrap();
        match fault {
            "pending" => journal.summary.state = "applying".into(),
            "identity" => journal.summary.source.account = "../invalid".into(),
            "roots" => journal.roots_hash = digest(b"different roots"),
            "operation" => journal.operations[0].name = "../invalid.json".into(),
            "json" => (),
            _ => unreachable!(),
        }
        if fault == "json" {
            fs::write(journal_path(&f.roots, &run.id), b"{").unwrap();
        } else {
            save(&f.roots, &journal).unwrap();
        }
        fs::remove_dir_all(f.dir(&f.a)).unwrap();
        let (source, target) = unrelated_pair(&f);
        if fault == "pending" {
            assert_eq!(list_runs(&f.roots).unwrap()[0].state, "applying");
        } else {
            assert!(list_runs(&f.roots).is_err());
        }
        assert_eq!(
            preview(&f.roots, &source, &target).unwrap_err(),
            "PENDING_RUN"
        );
        assert_eq!(
            apply(
                &f.roots,
                &source,
                &target,
                "unused",
                &f.backup(),
                &mut || Ok(())
            )
            .unwrap_err(),
            "PENDING_RUN"
        );
        assert!(!f.path(&target, 20).exists());
    }
}

fn without_cli(mut value: Value) -> Value {
    value.as_object_mut().unwrap().remove("cliSessionId");
    value
}

fn marked_quota(n: u32) -> Value {
    let mut r = record(n);
    r["error"] = json!("Your session limit resets at noon");
    r["errorAt"] = json!(100);
    r["lastActivityAt"] = json!(101);
    r["interruptedByQuitAt"] = json!(102);
    r["interruptedUnseenResume"] = json!(true);
    r
}

#[test]
fn mixed_namespaces_classify_nonresumable_pending_and_remote_rows_before_local_validation() {
    let f = Fixture::new();
    f.put(&f.a, &record(10), true);
    let cleared = without_cli(record(11));
    let mut pending = without_cli(record(12));
    pending["pendingFirstStart"] = json!({"prompt":"synthetic"});
    pending["cwd"] = json!("~");
    let mut windows = record(13);
    windows["sshConfig"] = json!({"host":"synthetic"});
    windows["cwd"] = json!("C:\\synthetic\\project");
    let mut remote_home = without_cli(record(14));
    remote_home["sshConfig"] = json!({"host":"synthetic"});
    remote_home["cwd"] = json!("~/synthetic");
    let mut task = without_cli(record(15));
    task["spawnedFrom"] = json!({"taskId":"synthetic", "sessionId":sid(19)});
    for value in [&cleared, &pending, &windows, &remote_home, &task] {
        f.put(&f.a, value, false);
        f.put(&f.b, value, false);
    }
    for n in [16, 17, 19] {
        f.put(&f.a, &record(n), true);
    }
    f.put(&f.b, &without_cli(record(16)), false);
    let mut target_pending = without_cli(record(17));
    target_pending["armedWorkAtQuit"] = json!({"kind":"synthetic"});
    f.put(&f.b, &target_pending, false);
    f.put(&f.b, &without_cli(record(18)), false);
    let mut descendant = record(20);
    descendant["forkedFromSessionId"] = json!(sid(11));
    f.put(&f.a, &descendant, true);
    let source_before = record_stamps(&records(&f.dir(&f.a)).unwrap());
    let target_before = record_stamps(&records(&f.dir(&f.b)).unwrap());
    let p = f.preview();
    assert_eq!((p.created, p.updated), (1, 0));
    for (n, reason) in [
        (11, "NON_RESUMABLE_SESSION"),
        (12, "UNFINISHED_SESSION"),
        (13, "NONLOCAL_SESSION"),
        (14, "NONLOCAL_SESSION"),
        (15, "OWNED_OR_SPAWNED_SESSION"),
        (16, "TARGET_NON_RESUMABLE_SESSION"),
        (17, "TARGET_UNFINISHED_SESSION"),
        (19, "OWNED_OR_SPAWNED_SESSION"),
        (20, "EXCLUDED_PARENT"),
    ] {
        assert!(issue(&p, n, reason), "{n} {reason}");
    }
    let run = f.apply();
    assert_eq!(
        record_stamps(&records(&f.dir(&f.a)).unwrap()),
        source_before
    );
    for (name, stamp) in target_before {
        assert_eq!(
            required_blob(&f.dir(&f.b).join(&name)).unwrap().stamp,
            stamp
        );
        assert!(!Path::new(&run.backup_dir)
            .join("target")
            .join(name)
            .exists());
    }
    assert_eq!(f.get(&f.b, 10)["cliSessionId"], id(110));
}

#[test]
fn excluded_mixed_namespace_rows_stay_in_drift_checks_without_extra_backups() {
    for source_side in [false, true] {
        let f = Fixture::new();
        f.put(&f.a, &record(10), true);
        let excluded_identity = if source_side { &f.a } else { &f.b };
        f.put(excluded_identity, &without_cli(record(11)), false);
        let p = f.preview();
        let mut calls = 0;
        assert_eq!(
            apply(
                &f.roots,
                &f.a,
                &f.b,
                &p.fingerprint,
                &f.backup(),
                &mut || {
                    calls += 1;
                    if calls == 2 {
                        let mut row = without_cli(record(11));
                        row["title"] = json!("Changed excluded row");
                        f.put(excluded_identity, &row, false);
                    }
                    Ok(())
                }
            )
            .unwrap_err(),
            "SNAPSHOT_DRIFT"
        );
        assert!(!f.path(&f.b, 10).exists());
    }
}

#[test]
fn backup_contains_only_changed_target_preimage_and_rollback_restores_it() {
    let f = Fixture::new();
    let mut source = record(10);
    source["lastActivityAt"] = json!(100);
    let target = record(10);
    f.put(&f.a, &source, true);
    f.put(&f.b, &target, false);
    f.put(&f.a, &record(11), true);
    f.put(&f.b, &record(11), false);
    let target_before = fs::read(f.path(&f.b, 10)).unwrap();
    assert_eq!(f.preview().updated, 1);
    let run = f.apply();
    let backup = Path::new(&run.backup_dir);
    assert_eq!(
        fs::read(backup.join("target").join(name(10))).unwrap(),
        target_before
    );
    assert!(!backup.join("target").join(name(11)).exists());
    assert!(!backup.join("source").exists());
    assert!(!backup.join("baseline.json").exists());
    assert_eq!(fs::read_dir(backup).unwrap().count(), 1);
    f.undo(&run);
    assert_eq!(fs::read(f.path(&f.b, 10)).unwrap(), target_before);
}

#[test]
fn ordinary_candidate_validation_is_strict_on_source_and_matching_target() {
    for target_side in [false, true] {
        for patch in [
            json!({"cliSessionId":null}),
            json!({"cliSessionId":"not-a-uuid"}),
            json!({"cliSessionId":42}),
            json!({"cwd":"C:\\synthetic"}),
            json!({"cwd":"~/synthetic"}),
        ] {
            let f = Fixture::new();
            let mut malformed = record(10);
            malformed
                .as_object_mut()
                .unwrap()
                .extend(patch.as_object().unwrap().clone());
            f.put(&f.a, &record(10), true);
            f.put(if target_side { &f.b } else { &f.a }, &malformed, false);
            assert_eq!(f.preview_error(), "INVALID_RECORD_IDENTITY");
        }
    }
}

#[test]
fn malformed_ingestion_cannot_be_hidden_behind_an_excluded_state() {
    for kind in ["json", "object", "identity", "alias"] {
        let f = Fixture::new();
        f.put(&f.a, &record(10), true);
        match kind {
            "json" => fs::write(f.path(&f.a, 11), b"{").unwrap(),
            "object" => fs::write(f.path(&f.a, 11), b"[]").unwrap(),
            "identity" => {
                let mut wrong = without_cli(record(99));
                wrong["pendingFirstStart"] = json!({});
                fs::write(f.path(&f.a, 11), json_bytes(&wrong).unwrap()).unwrap();
            }
            "alias" => fs::hard_link(f.path(&f.a, 10), f.path(&f.a, 11)).unwrap(),
            _ => unreachable!(),
        }
        assert_eq!(
            f.preview_error(),
            if kind == "json" {
                "INVALID_JSON"
            } else {
                "INVALID_RECORD_IDENTITY"
            }
        );
    }
}

#[test]
fn cloud_move_graphs_and_staging_are_excluded_on_both_affected_sides() {
    for target_side in [false, true] {
        for (field, value, reason) in [
            (
                "movedToCloud",
                json!({"cloudSessionId":"synthetic"}),
                "UNSUPPORTED_HISTORY_STATE",
            ),
            (
                "rewindEdges",
                json!([{"from":id(110),"to":id(111)}]),
                "UNSUPPORTED_HISTORY_STATE",
            ),
            (
                "transcriptCuts",
                json!({id(110):"synthetic-cut"}),
                "UNSUPPORTED_HISTORY_STATE",
            ),
            (
                "transcriptModelStates",
                json!({id(110):"synthetic-model"}),
                "UNSUPPORTED_HISTORY_STATE",
            ),
            (
                "stagedTranscriptPath",
                json!("/synthetic/staged.jsonl"),
                "UNFINISHED_SESSION",
            ),
        ] {
            let f = Fixture::new();
            f.put(&f.a, &record(10), true);
            let mut unsupported = record(10);
            unsupported[field] = value;
            f.put(if target_side { &f.b } else { &f.a }, &unsupported, false);
            let before =
                record_stamps(&records(&f.dir(if target_side { &f.b } else { &f.a })).unwrap());
            let p = f.preview();
            assert_eq!((p.created, p.updated), (0, 0));
            assert!(issue(
                &p,
                10,
                &format!("{}{reason}", if target_side { "TARGET_" } else { "" })
            ));
            f.apply();
            assert_eq!(
                record_stamps(&records(&f.dir(if target_side { &f.b } else { &f.a })).unwrap()),
                before
            );
        }
    }
}

#[test]
fn unsupported_history_blocks_repeat_reverse_and_rollover_after_a_baseline_exists() {
    let f = Fixture::new();
    f.put(&f.a, &record(10), true);
    f.apply();
    let mut target = f.get(&f.b, 10);
    target["rewindEdges"] = json!([{"from":id(110), "to":id(999)}]);
    f.put(&f.b, &target, false);
    assert!(issue(&f.preview(), 10, "TARGET_UNSUPPORTED_HISTORY_STATE"));
    assert!(issue(
        &preview(&f.roots, &f.b, &f.a).unwrap(),
        10,
        "UNSUPPORTED_HISTORY_STATE"
    ));
    let mut source = record(10);
    source["cliSessionId"] = json!(id(888));
    source["priorCliSessionIds"] = json!([id(110)]);
    f.put(&f.a, &source, true);
    assert!(issue(&f.preview(), 10, "TARGET_UNSUPPORTED_HISTORY_STATE"));
    f.apply();
    assert_eq!(f.get(&f.b, 10), target);
}

#[test]
fn empty_new_history_containers_are_ordinary_and_target_defaults_are_preserved() {
    let f = Fixture::new();
    let mut source = record(10);
    source["lastActivityAt"] = json!(100);
    let mut target = record(10);
    for field in [
        "movedToCloud",
        "rewindEdges",
        "transcriptCuts",
        "transcriptModelStates",
    ] {
        source[field] = json!([]);
        target[field] = json!({});
    }
    target["permissionMode"] = json!("acceptEdits");
    target["autoChosenInApp"] = json!(false);
    f.put(&f.a, &source, true);
    f.put(&f.b, &target, false);
    let p = f.preview();
    assert_eq!(p.updated, 1);
    assert!(p.issues.is_empty());
    f.apply();
    let out = f.get(&f.b, 10);
    assert!(out.get("rewindEdges").is_none());
    assert_eq!(out["permissionMode"], "acceptEdits");
    assert_eq!(out["autoChosenInApp"], false);
}

#[test]
fn pending_and_unknown_quit_states_are_excluded_even_on_same_pointer_updates() {
    for target_side in [false, true] {
        for patch in [
            json!({"pendingFirstStart":{}}),
            json!({"armedWorkAtQuit":{}}),
            json!({"interruptedByQuitAt":5}),
            json!({"interruptedUnseenResume":true}),
            json!({"interruptedByQuitAt":5,"error":"Authentication failed"}),
        ] {
            let f = Fixture::new();
            let mut source = record(10);
            source["lastActivityAt"] = json!(200);
            f.put(&f.a, &source, true);
            let mut marked = if target_side { record(10) } else { source };
            marked
                .as_object_mut()
                .unwrap()
                .extend(patch.as_object().unwrap().clone());
            f.put(if target_side { &f.b } else { &f.a }, &marked, false);
            let p = f.preview();
            assert!(issue(
                &p,
                10,
                if target_side {
                    "TARGET_UNFINISHED_SESSION"
                } else {
                    "UNFINISHED_SESSION"
                }
            ));
            assert_eq!((p.created, p.updated), (0, 0));
        }
    }
}

#[test]
fn known_quota_source_with_quit_markers_imports_with_default_grants_and_manual_continuation() {
    let f = Fixture::new();
    let mut source = marked_quota(10);
    source["autoChosenInApp"] = json!(true);
    source["bypassChosenInApp"] = json!(true);
    f.put(&f.a, &source, true);
    let before = required_blob(&f.path(&f.a, 10)).unwrap().stamp;
    let p = f.preview();
    assert!(p.issues.is_empty());
    assert_eq!(p.created, 1);
    f.apply();
    let target = f.get(&f.b, 10);
    assert!(!quit_marked(&target));
    assert!(target.get("error").is_none());
    assert_eq!(target["permissionMode"], "default");
    assert_eq!(target["bypassChosenInApp"], false);
    assert!(target.get("autoChosenInApp").is_none());
    assert_eq!(required_blob(&f.path(&f.a, 10)).unwrap().stamp, before);
}

#[test]
fn quota_quit_exception_does_not_allow_newer_errors_pending_work_or_unknown_marker_shapes() {
    for patch in [
        json!({"error":"Session limit reached. Resets at noon"}),
        json!({"errorAt":300}),
        json!({"errorCategory":"authentication"}),
        json!({"tccFolderKind":"documents"}),
        json!({"armedWorkAtQuit":{}}),
        json!({"pendingFirstStart":{}}),
        json!({"interruptedByQuitAt":"unknown"}),
        json!({"interruptedUnseenResume":{}}),
    ] {
        let f = Fixture::new();
        let mut source = record(10);
        source["lastActivityAt"] = json!(200);
        f.put(&f.a, &source, true);
        let mut target = marked_quota(10);
        target
            .as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        f.put(&f.b, &target, false);
        assert!(issue(&f.preview(), 10, "TARGET_UNFINISHED_SESSION"));
        f.apply();
        assert_eq!(f.get(&f.b, 10), target);
    }
}

#[test]
fn quota_marked_source_rollover_clears_markers_but_target_exception_requires_same_pointer() {
    for source_side in [false, true] {
        let f = Fixture::new();
        f.put(&f.a, &record(10), true);
        f.apply();
        let mut source = if source_side {
            marked_quota(10)
        } else {
            record(10)
        };
        source["cliSessionId"] = json!(id(999));
        source["priorCliSessionIds"] = json!([id(110)]);
        source["lastActivityAt"] = json!(200);
        f.put(&f.a, &source, true);
        if !source_side {
            f.put(&f.b, &marked_quota(10), false);
        }
        let p = f.preview();
        assert_eq!(p.updated, usize::from(source_side));
        if !source_side {
            assert!(issue(&p, 10, "TARGET_UNFINISHED_SESSION"));
        }
        f.apply();
        let target = f.get(&f.b, 10);
        if source_side {
            assert_eq!(target["cliSessionId"], id(999));
            assert!(!quit_marked(&target));
            assert!(target.get("error").is_none());
        } else {
            assert_eq!(target, marked_quota(10));
        }
    }
}

#[test]
fn retained_target_pointer_cannot_keep_quota_quit_markers_after_unrelated_metadata_update() {
    let f = Fixture::new();
    f.put(&f.a, &record(10), true);
    f.apply();
    let mut target = marked_quota(10);
    target["cliSessionId"] = json!(id(999));
    target["priorCliSessionIds"] = json!([id(110)]);
    f.put(&f.b, &target, true);
    let mut source = record(10);
    source["lastActivityAt"] = json!(300);
    source["title"] = json!("Source title change");
    f.put(&f.a, &source, false);
    assert!(issue(&f.preview(), 10, "TARGET_UNFINISHED_SESSION"));
    f.apply();
    assert_eq!(f.get(&f.b, 10), target);
}

#[test]
fn quota_cleanup_never_overrides_a_portable_metadata_conflict() {
    let f = Fixture::new();
    let mut source = record(10);
    source["lastActivityAt"] = json!(200);
    source["title"] = json!("Source title");
    f.put(&f.a, &source, true);
    let target = marked_quota(10);
    f.put(&f.b, &target, false);
    assert!(issue(&f.preview(), 10, "DIVERGENT_METADATA"));
    f.apply();
    assert_eq!(f.get(&f.b, 10), target);
}

#[test]
fn quit_markers_added_after_preview_or_by_guard_are_detected_before_first_record_write() {
    for during_guard in [false, true] {
        let f = Fixture::new();
        f.put(&f.a, &record(10), true);
        let p = f.preview();
        let mutate = || {
            let mut changed = record(10);
            changed["interruptedByQuitAt"] = json!(5);
            f.put(&f.a, &changed, false);
        };
        if !during_guard {
            mutate();
        }
        let mut calls = 0;
        let error = apply(
            &f.roots,
            &f.a,
            &f.b,
            &p.fingerprint,
            &f.backup(),
            &mut || {
                calls += 1;
                if during_guard && calls == 2 {
                    mutate();
                }
                Ok(())
            },
        )
        .unwrap_err();
        assert_eq!(
            error,
            if during_guard {
                "SNAPSHOT_DRIFT"
            } else {
                "PREVIEW_CHANGED"
            }
        );
        assert!(!f.path(&f.b, 10).exists());
    }
}

#[test]
fn prior_lineage_missing_empty_and_ambiguous_are_warnings_and_ids_are_preserved() {
    let f = Fixture::new();
    let mut source = record(10);
    source["priorCliSessionIds"] = json!([id(110), id(201), id(202), id(203), id(203)]);
    f.put(&f.a, &source, true);
    let project = f.roots.pool.join("project");
    fs::write(project.join(format!("{}.jsonl", id(202))), []).unwrap();
    fs::write(
        project.join(format!("{}.jsonl", id(203))),
        b"synthetic first",
    )
    .unwrap();
    let other = f.roots.pool.join("other");
    Directory::open(&other, true).unwrap();
    fs::write(
        other.join(format!("{}.jsonl", id(203))),
        b"synthetic second",
    )
    .unwrap();
    let p = f.preview();
    assert_eq!(p.created, 1);
    assert!(p.issues.is_empty());
    let codes: BTreeSet<_> = p.warnings.iter().map(|w| w.reason.as_str()).collect();
    assert_eq!(
        codes,
        BTreeSet::from([
            "PRIOR_TRANSCRIPT_MISSING",
            "PRIOR_TRANSCRIPT_EMPTY",
            "PRIOR_TRANSCRIPT_AMBIGUOUS"
        ])
    );
    assert!(p
        .warnings
        .iter()
        .all(|w| w.session_id == sid(10) && w.title.as_deref() == Some("Synthetic 10")));
    let witnesses = transcripts(
        &f.roots.pool,
        &BTreeSet::from([id(110), id(201), id(202), id(203)]),
    )
    .unwrap();
    f.apply();
    assert_eq!(
        f.get(&f.b, 10)["priorCliSessionIds"],
        source["priorCliSessionIds"]
    );
    assert_eq!(
        transcripts(&f.roots.pool, &witnesses.keys().cloned().collect()).unwrap(),
        witnesses
    );
}

#[test]
fn prior_equal_to_current_is_deduplicated_for_inventory_without_a_warning() {
    let f = Fixture::new();
    let mut source = record(10);
    source["priorCliSessionIds"] = json!([id(110), id(110)]);
    f.put(&f.a, &source, true);
    let p = f.preview();
    assert!(p.issues.is_empty());
    assert!(p.warnings.is_empty());
    f.apply();
    assert_eq!(
        f.get(&f.b, 10)["priorCliSessionIds"],
        source["priorCliSessionIds"]
    );
}

#[test]
fn prior_only_changes_bind_preview_and_are_rechecked_during_mutation() {
    for change in ["content", "appeared", "empty", "duplicate"] {
        for during_guard in [false, true] {
            let f = Fixture::new();
            let mut source = record(10);
            source["priorCliSessionIds"] = json!([id(201)]);
            f.put(&f.a, &source, true);
            let prior = f
                .roots
                .pool
                .join("project")
                .join(format!("{}.jsonl", id(201)));
            if change != "appeared" {
                fs::write(
                    &prior,
                    if change == "empty" {
                        b"".as_slice()
                    } else {
                        b"synthetic before".as_slice()
                    },
                )
                .unwrap();
            }
            let p = f.preview();
            let active_before = fs::read(f.transcript(&source)).unwrap();
            let mutate = || {
                if change == "duplicate" {
                    let other = f.roots.pool.join("other");
                    Directory::open(&other, true).unwrap();
                    fs::write(
                        other.join(format!("{}.jsonl", id(201))),
                        b"synthetic duplicate",
                    )
                    .unwrap();
                } else {
                    fs::write(&prior, b"synthetic after").unwrap();
                }
            };
            if !during_guard {
                mutate();
            }
            let mut calls = 0;
            let error = apply(
                &f.roots,
                &f.a,
                &f.b,
                &p.fingerprint,
                &f.backup(),
                &mut || {
                    calls += 1;
                    if during_guard && calls == 2 {
                        mutate();
                    }
                    Ok(())
                },
            )
            .unwrap_err();
            assert_eq!(
                error,
                if during_guard {
                    "TRANSCRIPT_DRIFT"
                } else {
                    "PREVIEW_CHANGED"
                }
            );
            assert!(!f.path(&f.b, 10).exists());
            assert_eq!(fs::read(f.transcript(&source)).unwrap(), active_before);
        }
    }
}

#[test]
fn historical_inventory_is_bounded_to_last_two_hundred_unique_ids_without_rewriting_them() {
    let f = Fixture::new();
    let mut source = record(10);
    let ids: Vec<_> = (1000..1205).map(id).collect();
    source["priorCliSessionIds"] = json!(ids);
    let inventory = prior_ids(&source);
    assert_eq!(inventory.len(), MAX_PRIOR_IDS);
    assert!(!inventory.contains(&id(1000)));
    assert!(inventory.contains(&id(1005)));
    assert!(inventory.contains(&id(1204)));
    f.put(&f.a, &source, true);
    let p = f.preview();
    assert_eq!(p.created, 1);
    assert_eq!(p.warnings.len(), 1);
    f.apply();
    assert_eq!(
        f.get(&f.b, 10)["priorCliSessionIds"],
        source["priorCliSessionIds"]
    );
}

#[test]
fn vendor_ten_mib_generated_record_limit_is_checked_before_metadata_or_backup_writes() {
    let f = Fixture::new();
    let mut source = record(10);
    source["title"] = json!("");
    let base = serde_json::to_vec(&source).unwrap().len();
    source["title"] = json!("x".repeat(MAX_JSON as usize - base));
    let compact = serde_json::to_vec(&source).unwrap();
    assert_eq!(compact.len() as u64, MAX_JSON);
    fs::write(f.path(&f.a, 10), &compact).unwrap();
    fs::write(f.transcript(&source), b"synthetic transcript").unwrap();
    assert_eq!(f.preview_error(), "JSON_TOO_LARGE");
    assert!(!f.roots.state.exists());
    let backup = f.backup();
    assert_eq!(
        apply(&f.roots, &f.a, &f.b, "unused", &backup, &mut || Ok(())).unwrap_err(),
        "JSON_TOO_LARGE"
    );
    assert!(!backup.exists());
    assert!(!f.path(&f.b, 10).exists());
    assert!(list_runs(&f.roots).unwrap().is_empty());
    assert_eq!(fs::read(f.path(&f.a, 10)).unwrap(), compact);
    fs::write(f.path(&f.a, 11), vec![b' '; MAX_JSON as usize + 1]).unwrap();
    assert_eq!(f.preview_error(), "JSON_TOO_LARGE");
}

#[test]
fn issue_title_uses_local_source_then_target_and_missing_strings_are_omitted() {
    for source_title in [json!(null), json!({"not":"a string"})] {
        let f = Fixture::new();
        let mut source = record(10);
        source["title"] = source_title;
        f.put(&f.a, &source, false);
        f.put(&f.b, &record(10), false);
        let p = f.preview();
        assert_eq!(p.issues[0].title.as_deref(), Some("Synthetic 10"));
        let mut target = record(10);
        target.as_object_mut().unwrap().remove("title");
        f.put(&f.b, &target, false);
        let p = f.preview();
        assert!(p.issues[0].title.is_none());
        assert!(serde_json::to_value(&p.issues[0])
            .unwrap()
            .get("title")
            .is_none());
    }
}

#[test]
fn imported_records_require_explicit_boolean_confirmation_on_both_affected_sides() {
    for target_side in [false, true] {
        for confirmed in [
            None,
            Some(Value::Null),
            Some(json!(false)),
            Some(json!("true")),
            Some(json!(1)),
            Some(json!({})),
        ] {
            let f = Fixture::new();
            f.put(&f.a, &record(10), true);
            let mut imported = record(10);
            imported["importedFrom"] = json!({"source":"synthetic"});
            if let Some(value) = &confirmed {
                imported["resumeConfirmed"] = value.clone();
            }
            f.put(if target_side { &f.b } else { &f.a }, &imported, false);
            let malformed = confirmed
                .as_ref()
                .map(|v| !v.is_null() && !v.is_boolean())
                .unwrap_or(false);
            let reason = if malformed {
                "UNSUPPORTED_HISTORY_STATE"
            } else {
                "UNFINISHED_SESSION"
            };
            let p = f.preview();
            assert!(issue(
                &p,
                10,
                &format!("{}{reason}", if target_side { "TARGET_" } else { "" })
            ));
            assert_eq!((p.created, p.updated), (0, 0));
            f.apply();
            assert_eq!(f.get(if target_side { &f.b } else { &f.a }, 10), imported);
        }
    }
}

#[test]
fn confirmed_import_without_staging_is_eligible_and_plain_rows_need_no_confirmation() {
    for target_side in [false, true] {
        let f = Fixture::new();
        let mut source = record(10);
        source["lastActivityAt"] = json!(200);
        let mut imported = if target_side {
            record(10)
        } else {
            source.clone()
        };
        imported["importedFrom"] = json!({"source":"synthetic"});
        imported["resumeConfirmed"] = json!(true);
        f.put(&f.a, &source, true);
        f.put(if target_side { &f.b } else { &f.a }, &imported, false);
        let p = f.preview();
        assert!(p.issues.is_empty());
        assert_eq!(p.created + p.updated, 1);
        f.apply();
        assert!(!quit_marked(&f.get(&f.b, 10)));
        imported["stagedTranscriptPath"] = json!("/synthetic/staged.jsonl");
        f.put(if target_side { &f.b } else { &f.a }, &imported, false);
        assert!(issue(
            &f.preview(),
            10,
            if target_side {
                "TARGET_UNFINISHED_SESSION"
            } else {
                "UNFINISHED_SESSION"
            }
        ));
    }
}

#[test]
fn replaced_target_pointer_still_reports_missing_prior_history_after_rollover() {
    let f = Fixture::new();
    let original = record(10);
    f.put(&f.a, &original, true);
    f.apply();
    let mut source = original.clone();
    source["cliSessionId"] = json!(id(999));
    source["priorCliSessionIds"] = json!([id(110)]);
    f.put(&f.a, &source, true);
    fs::remove_file(f.transcript(&original)).unwrap();
    let p = f.preview();
    assert_eq!(p.updated, 1);
    assert!(p.issues.is_empty());
    assert_eq!(p.warnings.len(), 1);
    assert_eq!(p.warnings[0].reason, "PRIOR_TRANSCRIPT_MISSING");
    f.apply();
    assert_eq!(f.get(&f.b, 10)["priorCliSessionIds"], json!([id(110)]));
    assert_eq!(f.get(&f.b, 10)["cliSessionId"], id(999));
}
