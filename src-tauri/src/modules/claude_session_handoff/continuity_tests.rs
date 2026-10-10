use super::*;

fn updater_test_archive(f: &Fixture, extra: &str) -> PathBuf {
    let app = f.root.join("Claude.app");
    let resources = app.join("Contents/Resources");
    fs::create_dir_all(&resources).unwrap();
    fs::write(
        resources.join("app.asar"),
        super::super::super::storage_contract::synthetic_archive_for_test(extra),
    )
    .unwrap();
    app
}

#[test]
fn settled_binding_cannot_be_renewed_after_a_later_archive_replacement() {
    let f = Fixture::new();
    let app = updater_test_archive(&f, "");
    let initial = super::super::super::storage_contract::inspect(&app).unwrap();
    updater_test_archive(&f, ",futureOptional:state.futureOptional");
    let shutdown = super::super::super::runtime::ShutdownReceipt {
        was_running: true,
        bundled_update_settled: true,
        settled_contract: Some(super::super::super::storage_contract::inspect(&app).unwrap()),
    };
    super::super::super::contract_after_shutdown(&initial, &shutdown, true, &app).unwrap();
    updater_test_archive(&f, ",differentLaterField:state.differentLaterField");
    assert_eq!(
        super::super::super::contract_after_shutdown(&initial, &shutdown, true, &app).unwrap_err(),
        "DESKTOP_CONTRACT_CHANGED"
    );
    assert!(list_runs(&f.roots).unwrap().is_empty());
}

#[test]
fn settled_update_rechecks_storage_before_copying_latest_content_and_preserves_sources() {
    let f = Fixture::new();
    let mut latest = record(1);
    latest["lastActivityAt"] = json!(50);
    latest["completedTurns"] = json!(9);
    f.put(&f.a, &latest, true);
    f.put(&f.b, &record(1), false);
    let source = fs::read(f.path(&f.a, 1)).unwrap();
    let transcript = fs::read(f.transcript(&latest)).unwrap();
    let app = updater_test_archive(&f, "");
    let initial = super::super::super::storage_contract::inspect(&app).unwrap();
    updater_test_archive(&f, ",futureOptional:state.futureOptional");
    assert!(initial.assert_unchanged().is_err());
    let shutdown = super::super::super::runtime::ShutdownReceipt {
        was_running: true,
        bundled_update_settled: true,
        settled_contract: Some(super::super::super::storage_contract::inspect(&app).unwrap()),
    };
    let final_contract =
        super::super::super::contract_after_shutdown(&initial, &shutdown, true, &app).unwrap();
    assert_ne!(initial.fingerprint, final_contract.fingerprint);
    let unknown = unknown_persisted_fields(&final_contract.projected_fields);
    assert!(unknown.contains("futureOptional"));
    let accounts = [f.a.clone(), f.b.clone()];
    let p = preview_continuity_with_fields(&f.roots, &f.a, &f.b, &accounts, &unknown).unwrap();
    assert_eq!(p.missing, 0);
    let run = apply_continuity_with_fields_and_progress(
        &f.roots,
        &f.a,
        &f.b,
        &accounts,
        &unknown,
        &p.fingerprint,
        &f.backup(),
        &mut || final_contract.assert_unchanged(),
        Some(&mut || final_contract.assert_unchanged_fast()),
        &mut |_| {},
        &mut |_, _, _| {},
    )
    .unwrap();
    assert_eq!(run.state, "applied");
    assert_eq!(f.get(&f.b, 1)["completedTurns"], 9);
    assert_eq!(f.get(&f.b, 1)["cliSessionId"], latest["cliSessionId"]);
    assert_eq!(fs::read(f.path(&f.a, 1)).unwrap(), source);
    assert_eq!(fs::read(f.transcript(&latest)).unwrap(), transcript);
    // An unchanged initial approval cannot be reused for the final archive.
    assert!(
        super::super::super::contract_after_shutdown(&initial, &shutdown, false, &app).is_err()
    );
    let no_update = super::super::super::runtime::ShutdownReceipt {
        was_running: true,
        bundled_update_settled: false,
        settled_contract: None,
    };
    assert!(
        super::super::super::contract_after_shutdown(&initial, &no_update, true, &app).is_err()
    );
}

#[test]
fn settled_update_with_meaningful_new_state_blocks_before_any_publication() {
    let f = Fixture::new();
    let mut source = record(1);
    source["futureExecution"] = json!({"pending":true});
    f.put(&f.a, &source, true);
    let bytes = fs::read(f.path(&f.a, 1)).unwrap();
    let app = updater_test_archive(&f, "");
    let initial = super::super::super::storage_contract::inspect(&app).unwrap();
    updater_test_archive(&f, ",futureExecution:state.futureExecution");
    let shutdown = super::super::super::runtime::ShutdownReceipt {
        was_running: true,
        bundled_update_settled: true,
        settled_contract: Some(super::super::super::storage_contract::inspect(&app).unwrap()),
    };
    let final_contract =
        super::super::super::contract_after_shutdown(&initial, &shutdown, true, &app).unwrap();
    let unknown = unknown_persisted_fields(&final_contract.projected_fields);
    let accounts = [f.a.clone(), f.b.clone()];
    let p = preview_continuity_with_fields(&f.roots, &f.a, &f.b, &accounts, &unknown).unwrap();
    assert_eq!(p.missing, 1);
    assert!(p
        .issues
        .iter()
        .any(|issue| issue.reason == "UNSUPPORTED_PERSISTED_FIELD"));
    let mut published = false;
    assert!(apply_continuity_with_fields_and_progress(
        &f.roots,
        &f.a,
        &f.b,
        &accounts,
        &unknown,
        &p.fingerprint,
        &f.backup(),
        &mut || final_contract.assert_unchanged(),
        None,
        &mut |_| published = true,
        &mut |_, _, _| {}
    )
    .is_err());
    assert!(!published);
    assert!(!f.path(&f.b, 1).exists());
    assert_eq!(fs::read(f.path(&f.a, 1)).unwrap(), bytes);
    assert!(list_runs(&f.roots).unwrap().is_empty());
}

#[test]
fn archive_update_after_publication_does_not_refresh_binding_and_is_recoverable() {
    let f = Fixture::new();
    f.put(&f.a, &record(1), true);
    f.put(&f.a, &record(2), true);
    let app = updater_test_archive(&f, "");
    let binding = super::super::super::storage_contract::inspect(&app).unwrap();
    let unknown = unknown_persisted_fields(&binding.projected_fields);
    let accounts = [f.a.clone(), f.b.clone()];
    let p = preview_continuity_with_fields(&f.roots, &f.a, &f.b, &accounts, &unknown).unwrap();
    let mut run_id = None;
    let error = apply_continuity_with_fields_and_progress(
        &f.roots,
        &f.a,
        &f.b,
        &accounts,
        &unknown,
        &p.fingerprint,
        &f.backup(),
        &mut || binding.assert_unchanged(),
        Some(&mut || binding.assert_unchanged_fast()),
        &mut |id| run_id = Some(id.to_owned()),
        &mut |stage, completed, _| {
            if stage == "writing" && completed == 1 {
                updater_test_archive(&f, ",futureOptional:state.futureOptional");
            }
        },
    )
    .unwrap_err();
    assert_eq!(error, "DESKTOP_CONTRACT_CHANGED");
    let run = rollback(&f.roots, &run_id.unwrap(), &mut || Ok(())).unwrap();
    assert_eq!(run.state, "rolled_back");
    assert!(!f.path(&f.b, 1).exists());
    assert!(!f.path(&f.b, 2).exists());
    assert_eq!(f.get(&f.a, 1), record(1));
    assert_eq!(f.get(&f.a, 2), record(2));
}

#[test]
fn continuity_preserves_destination_owned_queued_work_and_quit_markers() {
    let f = Fixture::new();
    f.put(&f.a, &record(1), true);
    let mut target = record(2);
    target["armedWorkAtQuit"] = json!({"kind":"prompt","prompt":"synthetic queued work"});
    target["pendingFirstStart"] = json!(true);
    target["interruptedByQuitAt"] = json!(10);
    target["interruptedUnseenResume"] = json!(true);
    f.put(&f.b, &target, true);
    transfer_all(&f, &f.a, &f.b, &[f.a.clone(), f.b.clone()]);
    assert_eq!(f.get(&f.b, 2), target);
}

#[test]
fn continuity_refuses_workspace_changes_with_destination_owned_queued_work() {
    let f = Fixture::new();
    let mut target = record(1);
    target["armedWorkAtQuit"] = json!({"kind":"prompt","prompt":"synthetic queued work"});
    let mut source = record(1);
    source["lastActivityAt"] = json!(20);
    source["cwd"] = json!("/synthetic/new-workspace");
    f.put(&f.a, &source, true);
    f.put(&f.b, &target, false);
    let p = preview_continuity(&f.roots, &f.a, &f.b, &[f.a.clone(), f.b.clone()]).unwrap();
    assert!(p
        .issues
        .iter()
        .any(|issue| issue.reason == "TARGET_UNFINISHED_SESSION"));
    assert_eq!(f.get(&f.b, 1), target);
}

#[test]
fn continuity_classifies_unstarted_and_cleared_rows_and_keeps_their_drift_witnesses() {
    let f = Fixture::new();
    let c = Identity {
        account: id(7),
        org: id(8),
    };
    Directory::open(&f.dir(&c), true).unwrap();
    f.put(&f.a, &record(1), true);
    let mut pending = record(2);
    pending.as_object_mut().unwrap().remove("cliSessionId");
    pending.as_object_mut().unwrap().remove("cwd");
    pending["pendingFirstStart"] = json!(true);
    let mut cleared = record(3);
    cleared.as_object_mut().unwrap().remove("cliSessionId");
    f.put(&c, &pending, false);
    f.put(&f.b, &cleared, false);
    let mut child = record(4);
    child["forkedFromSessionId"] = json!(sid(2));
    f.put(&c, &child, true);
    let accounts = [f.a.clone(), f.b.clone(), c.clone()];
    let p = preview_continuity(&f.roots, &f.a, &f.b, &accounts).unwrap();
    assert_eq!(p.missing, 0);
    assert_eq!(p.created, 2);
    assert_eq!(
        p.warnings
            .iter()
            .filter(|issue| issue.reason == "NON_RESUMABLE_SESSION")
            .count(),
        2
    );
    transfer_all(&f, &f.a, &f.b, &accounts);
    assert_eq!(f.get(&f.b, 3), cleared);
    assert!(!f.path(&f.b, 2).exists());
    assert_eq!(f.get(&c, 2), pending);
    assert_eq!(f.get(&f.b, 4)["forkedFromSessionId"], sid(2));
    assert!(p
        .warnings
        .iter()
        .any(|issue| issue.reason == "PREEXISTING_MISSING_PARENT"));
    let plan = super::super::continuity::build_plan(&f.roots, &f.a, &f.b, &accounts).unwrap();
    pending["title"] = json!("synthetic changed draft");
    f.put(&c, &pending, false);
    assert!(assert_fast_check(&plan, &f).is_err());
}

#[test]
fn continuity_preserves_native_context_recovery_without_merging_stale_target_state() {
    let f = Fixture::new();
    let mut source = record(1);
    source["lastActivityAt"] = json!(20);
    source["contextRecovery"] =
        json!({"attempts":1,"failures":0,"phase":"compacting","errorUuid":"synthetic-error"});
    source["errorRows"] = json!([{"message":"synthetic error"}]);
    source["gitAnchorsFolderRealpath"] = json!("/synthetic/project");
    let mut target = record(1);
    target["contextRecovery"] = json!({"attempts":2,"failures":2,"phase":"spent"});
    f.put(&f.a, &source, true);
    f.put(&f.b, &target, false);
    transfer_all(&f, &f.a, &f.b, &[f.a.clone(), f.b.clone()]);
    let updated = f.get(&f.b, 1);
    assert_eq!(
        updated["contextRecovery"],
        json!({"attempts":1,"failures":0,"phase":"waiting","errorUuid":"synthetic-error"})
    );
    assert_eq!(updated["errorRows"], source["errorRows"]);
    assert_eq!(updated["gitAnchorsFolderRealpath"], "/synthetic/project");
    source.as_object_mut().unwrap().remove("contextRecovery");
    source["lastActivityAt"] = json!(30);
    f.put(&f.a, &source, false);
    transfer_all(&f, &f.a, &f.b, &[f.a.clone(), f.b.clone()]);
    assert!(f.get(&f.b, 1).get("contextRecovery").is_none());
}

#[test]
fn continuity_refuses_active_descriptors_and_invalid_context_recovery_before_any_write() {
    for recovery in [
        json!({"attempts":1,"failures":0,"phase":"compacting","running":{"uuid":"synthetic-running"}}),
        json!({"attempts":3,"failures":0,"phase":"waiting"}),
        json!({"attempts":1,"failures":1,"phase":"compacting"}),
        json!({"attempts":1,"failures":0,"phase":"unknown"}),
    ] {
        let f = Fixture::new();
        let mut source = record(1);
        source["contextRecovery"] = recovery;
        f.put(&f.a, &source, true);
        let p = preview_continuity(&f.roots, &f.a, &f.b, &[f.a.clone(), f.b.clone()]).unwrap();
        assert!(p
            .issues
            .iter()
            .any(|issue| issue.reason == "UNSUPPORTED_CONTEXT_RECOVERY"));
        assert!(records(&f.dir(&f.b)).unwrap().is_empty());
    }
}

#[test]
fn continuity_validates_retained_destination_recovery_before_restage() {
    for recovery in [
        json!({"attempts":1,"failures":1,"phase":"compacting"}),
        json!({"attempts":1,"failures":0,"phase":"compacting","running":{"uuid":"synthetic-running"}}),
        json!({"attempts":1,"failures":0,"phase":"waiting","unknown":true}),
    ] {
        let f = Fixture::new();
        let source = record(1);
        let mut target = source.clone();
        target["contextRecovery"] = recovery;
        f.put(&f.a, &source, true);
        f.put(&f.b, &target, false);
        let p = preview_continuity(&f.roots, &f.a, &f.b, &[f.a.clone(), f.b.clone()]).unwrap();
        assert!(p
            .issues
            .iter()
            .any(|issue| issue.reason == "UNSUPPORTED_CONTEXT_RECOVERY"));
        assert_eq!(f.get(&f.b, 1), target);
    }
}

#[test]
fn continuity_native_recovery_restage_preserves_compacted_and_exhausted_states() {
    for (attempts, failures, phase, expected) in [
        (2, 1, "compacting", "spent"),
        (2, 1, "waiting", "spent"),
        (1, 0, "compacted", "compacted"),
        (2, 2, "spent", "spent"),
    ] {
        let f = Fixture::new();
        let mut source = record(1);
        source["contextRecovery"] = json!({"attempts":attempts,"failures":failures,"phase":phase});
        f.put(&f.a, &source, true);
        transfer_all(&f, &f.a, &f.b, &[f.a.clone(), f.b.clone()]);
        assert_eq!(f.get(&f.b, 1)["contextRecovery"]["phase"], expected);
    }
}

fn assert_fast_check(plan: &Plan, f: &Fixture) -> Result<()> {
    check_plan(
        &f.roots,
        plan,
        &record_stamps(&plan.target),
        &plan.baseline.as_ref().map(|b| b.stamp.clone()),
        false,
        Some(&record_metadata(&plan.target)),
    )
}

#[test]
fn continuity_backup_verifier_keeps_legacy_source_and_registry_images_readable() {
    let f = Fixture::new();
    f.put(&f.a, &record(1), true);
    let run = transfer_all(&f, &f.a, &f.b, &[f.a.clone(), f.b.clone()]);
    let mut journal = load(&f.roots, &run.id).unwrap();
    let root = Path::new(&run.backup_dir);
    Directory::open(&root.join("source"), true).unwrap();
    for key in [
        format!("source/{}", name(1)),
        "source/scheduled-tasks.json".into(),
        "target/scheduled-tasks.json".into(),
    ] {
        let bytes = json_bytes(&json!({"syntheticLegacyPreimage":true})).unwrap();
        exclusive(&root.join(&key), &bytes).unwrap();
        journal.backups.insert(key, digest(&bytes));
    }
    save(&f.roots, &journal).unwrap();
    let loaded = load(&f.roots, &run.id).unwrap();
    assert_eq!(verify_backups(&loaded).unwrap().len(), 3);
    f.undo(&run);
    assert!(records(&f.dir(&f.b)).unwrap().is_empty());
}

#[test]
fn continuity_backup_verifier_rebinds_children_after_reading_anchored_files() {
    let f = Fixture::new();
    let root = f.backup();
    let opened = Directory::open(&root, true).unwrap();
    let target = Directory::open(&root.join("target"), true).unwrap();
    check_backup_child(&opened, "target", &target).unwrap();
    fs::rename(root.join("target"), f.root.join("old-backup-target")).unwrap();
    Directory::open(&root.join("target"), true).unwrap();
    assert_eq!(
        check_backup_child(&opened, "target", &target).unwrap_err(),
        "BACKUP_REPLACED"
    );
}

#[test]
fn continuity_fast_witness_detects_same_length_in_place_source_edits() {
    let f = Fixture::new();
    f.put(&f.a, &record(1), true);
    let plan = continuity::build_plan(&f.roots, &f.a, &f.b, &[f.a.clone(), f.b.clone()]).unwrap();
    assert_fast_check(&plan, &f).unwrap();
    let bytes = fs::read(f.path(&f.a, 1)).unwrap();
    let text = String::from_utf8(bytes)
        .unwrap()
        .replace("Synthetic 1", "Synthetic 9");
    fs::write(f.path(&f.a, 1), text).unwrap();
    assert_eq!(assert_fast_check(&plan, &f).unwrap_err(), "SNAPSHOT_DRIFT");
}

#[test]
fn continuity_fast_witness_detects_new_source_rows() {
    let f = Fixture::new();
    f.put(&f.a, &record(1), true);
    let plan = continuity::build_plan(&f.roots, &f.a, &f.b, &[f.a.clone(), f.b.clone()]).unwrap();
    f.put(&f.a, &record(2), false);
    assert_eq!(assert_fast_check(&plan, &f).unwrap_err(), "SNAPSHOT_DRIFT");
}

#[test]
fn continuity_fast_target_witness_detects_in_place_edits_and_new_rows() {
    for new_row in [false, true] {
        let f = Fixture::new();
        f.put(&f.a, &record(1), true);
        f.put(&f.b, &record(2), true);
        let plan =
            continuity::build_plan(&f.roots, &f.a, &f.b, &[f.a.clone(), f.b.clone()]).unwrap();
        if new_row {
            f.put(&f.b, &record(3), false);
        } else {
            let value = fs::read_to_string(f.path(&f.b, 2))
                .unwrap()
                .replace("Synthetic 2", "Synthetic 9");
            fs::write(f.path(&f.b, 2), value).unwrap();
        }
        assert_eq!(assert_fast_check(&plan, &f).unwrap_err(), "SNAPSHOT_DRIFT");
    }
}

#[test]
fn continuity_fast_target_witness_rejects_even_empty_namespace_replacement() {
    let f = Fixture::new();
    f.put(&f.a, &record(1), true);
    let plan = continuity::build_plan(&f.roots, &f.a, &f.b, &[f.a.clone(), f.b.clone()]).unwrap();
    fs::rename(f.dir(&f.b), f.root.join("old-target")).unwrap();
    Directory::open(&f.dir(&f.b), true).unwrap();
    assert_eq!(assert_fast_check(&plan, &f).unwrap_err(), "SNAPSHOT_DRIFT");
}

#[test]
fn continuity_commit_still_checks_complete_source_images_after_publication() {
    let f = Fixture::new();
    f.put(&f.a, &record(1), true);
    let accounts = [f.a.clone(), f.b.clone()];
    let preview = preview_continuity(&f.roots, &f.a, &f.b, &accounts).unwrap();
    let result = apply_continuity_with_progress(
        &f.roots,
        &f.a,
        &f.b,
        &accounts,
        &preview.fingerprint,
        &f.backup(),
        &mut || Ok(()),
        None,
        &mut |_| {},
        &mut |stage, _, _| {
            if stage == "verifying" {
                let mut value = record(1);
                value["title"] = json!("Edited at commit");
                f.put(&f.a, &value, false);
            }
        },
    );
    assert_eq!(result.unwrap_err(), "SNAPSHOT_DRIFT");
    let pending = f.pending();
    rollback(&f.roots, &pending.summary.id, &mut || Ok(())).unwrap();
    assert!(records(&f.dir(&f.b)).unwrap().is_empty());
    assert_eq!(f.get(&f.a, 1)["title"], "Edited at commit");
}

#[test]
fn continuity_fast_witness_detects_duplicate_transcripts_in_existing_or_new_projects() {
    for new_project in [false, true] {
        let f = Fixture::new();
        f.put(&f.a, &record(1), true);
        let other = f.roots.pool.join("other");
        if !new_project {
            Directory::open(&other, true).unwrap();
        }
        let plan =
            continuity::build_plan(&f.roots, &f.a, &f.b, &[f.a.clone(), f.b.clone()]).unwrap();
        if new_project {
            Directory::open(&other, true).unwrap();
        }
        fs::copy(
            f.transcript(&record(1)),
            other.join(format!("{}.jsonl", id(101))),
        )
        .unwrap();
        assert_eq!(
            assert_fast_check(&plan, &f).unwrap_err(),
            "TRANSCRIPT_DRIFT"
        );
    }
}

#[test]
fn continuity_fast_witness_detects_in_place_transcript_edits() {
    let f = Fixture::new();
    f.put(&f.a, &record(1), true);
    let plan = continuity::build_plan(&f.roots, &f.a, &f.b, &[f.a.clone(), f.b.clone()]).unwrap();
    fs::write(f.transcript(&record(1)), b"{\"synthetic\":null}\n").unwrap();
    assert_eq!(
        assert_fast_check(&plan, &f).unwrap_err(),
        "TRANSCRIPT_DRIFT"
    );
}

#[test]
fn continuity_fast_witness_rejects_project_directory_swap() {
    let f = Fixture::new();
    f.put(&f.a, &record(1), true);
    let plan = continuity::build_plan(&f.roots, &f.a, &f.b, &[f.a.clone(), f.b.clone()]).unwrap();
    fs::rename(f.roots.pool.join("project"), f.root.join("old-project")).unwrap();
    Directory::open(&f.roots.pool.join("project"), true).unwrap();
    fs::copy(
        f.root
            .join("old-project")
            .join(format!("{}.jsonl", id(101))),
        f.transcript(&record(1)),
    )
    .unwrap();
    assert_eq!(
        assert_fast_check(&plan, &f).unwrap_err(),
        "TRANSCRIPT_DRIFT"
    );
}

#[test]
fn continuity_source_mutation_during_publish_aborts_before_exposing_any_row() {
    let f = Fixture::new();
    f.put(&f.a, &record(1), true);
    let accounts = [f.a.clone(), f.b.clone()];
    let preview = preview_continuity(&f.roots, &f.a, &f.b, &accounts).unwrap();
    let result = apply_continuity_with_progress(
        &f.roots,
        &f.a,
        &f.b,
        &accounts,
        &preview.fingerprint,
        &f.backup(),
        &mut || Ok(()),
        None,
        &mut |_| {},
        &mut |stage, done, _| {
            if stage == "writing" && done == 0 {
                let mut changed = record(1);
                changed["title"] = json!("Changed source");
                f.put(&f.a, &changed, false);
            }
        },
    );
    assert_eq!(result.unwrap_err(), "SNAPSHOT_DRIFT");
    assert!(records(&f.dir(&f.b)).unwrap().is_empty());
    assert!(!continuity::state_path(&f.roots).exists());
}

fn transfer_all(
    f: &Fixture,
    source: &Identity,
    target: &Identity,
    accounts: &[Identity],
) -> RunSummary {
    let preview = preview_continuity(&f.roots, source, target, accounts).unwrap();
    assert_eq!(preview.missing, 0, "{:?}", preview.issues);
    apply_continuity_observed(
        &f.roots,
        source,
        target,
        accounts,
        &preview.fingerprint,
        &f.backup(),
        &mut || Ok(()),
        None,
        &mut |_| {},
    )
    .unwrap()
}

fn active(f: &Fixture, account: &Identity) -> BTreeSet<String> {
    records(&f.dir(account))
        .unwrap()
        .values()
        .map(|r| r.value["cliSessionId"].as_str().unwrap().into())
        .collect()
}

#[test]
fn continuity_preserves_both_divergent_active_branches_without_replacing_target_pointer() {
    let f = Fixture::new();
    let mut a = record(1);
    a["lastActivityAt"] = json!(20);
    let mut b = record(1);
    b["cliSessionId"] = json!(id(500));
    b["lastActivityAt"] = json!(10);
    f.put(&f.a, &a, true);
    f.put(&f.b, &b, true);
    let a_before = fs::read(f.path(&f.a, 1)).unwrap();
    let run = transfer_all(&f, &f.a, &f.b, &[f.a.clone(), f.b.clone()]);
    assert_eq!(run.created, 1);
    assert_eq!(active(&f, &f.b), BTreeSet::from([id(101), id(500)]));
    let target = records(&f.dir(&f.b)).unwrap();
    assert_eq!(target[&name(1)].value["cliSessionId"], id(500));
    assert!(target[&name(1)].value["title"]
        .as_str()
        .unwrap()
        .contains("保留分支"));
    assert!(target
        .values()
        .any(|r| r.value["cliSessionId"] == id(101) && r.value["title"] == "Synthetic 1"));
    assert_eq!(fs::read(f.path(&f.a, 1)).unwrap(), a_before);
    f.undo(&run);
    assert_eq!(active(&f, &f.b), BTreeSet::from([id(500)]));
    assert_eq!(records(&f.dir(&f.b)).unwrap()[&name(1)].value, b);
    assert!(!continuity::state_path(&f.roots).exists());
}

#[test]
fn continuity_three_account_round_trip_is_complete_and_idempotent() {
    let f = Fixture::new();
    let c = Identity {
        account: id(7),
        org: id(8),
    };
    Directory::open(&f.dir(&c), true).unwrap();
    f.put(&f.a, &record(1), true);
    f.put(&f.b, &record(2), true);
    f.put(&c, &record(3), true);
    let identities = [f.a.clone(), f.b.clone(), c.clone()];
    transfer_all(&f, &f.a, &f.b, &identities);
    transfer_all(&f, &f.b, &c, &identities);
    transfer_all(&f, &c, &f.a, &identities);
    for identity in &identities {
        assert_eq!(
            active(&f, identity),
            BTreeSet::from([id(101), id(102), id(103)])
        );
    }
    let p = preview_continuity(&f.roots, &f.a, &f.b, &identities).unwrap();
    assert_eq!((p.created, p.updated, p.missing), (0, 0, 0));
}

#[test]
fn continuity_continued_alias_retains_old_and_new_histories_on_return() {
    let f = Fixture::new();
    let mut b = record(1);
    b["cliSessionId"] = json!(id(500));
    f.put(&f.a, &record(1), true);
    f.put(&f.b, &b, true);
    let identities = [f.a.clone(), f.b.clone()];
    transfer_all(&f, &f.a, &f.b, &identities);
    transfer_all(&f, &f.b, &f.a, &identities);
    let rows = records(&f.dir(&f.b)).unwrap();
    let (alias, r) = rows
        .iter()
        .find(|(n, r)| *n != &name(1) && r.value["cliSessionId"] == id(101))
        .unwrap();
    let mut continued = r.value.clone();
    continued["cliSessionId"] = json!(id(600));
    continued["lastActivityAt"] = json!(100);
    f.put(&f.b, &continued, true);
    assert_eq!(continued["sessionId"], alias.trim_end_matches(".json"));
    transfer_all(&f, &f.b, &f.a, &identities);
    assert_eq!(
        active(&f, &f.a),
        BTreeSet::from([id(101), id(500), id(600)])
    );
    transfer_all(&f, &f.a, &f.b, &identities);
    assert_eq!(active(&f, &f.b), active(&f, &f.a));
    assert!(records(&f.dir(&f.a))
        .unwrap()
        .values()
        .any(|r| r.value["cliSessionId"] == id(600) && r.value["title"] == "Synthetic 1"));
}

#[test]
fn continuity_keeps_destination_grants_and_does_not_import_source_grants_or_queued_work() {
    let f = Fixture::new();
    let mut a = record(1);
    a["lastActivityAt"] = json!(20);
    a["armedWorkAtQuit"] = json!({"kind":"synthetic"});
    a["envScopeId"] = json!("source-scope");
    a["pendingFirstStart"] = json!({"task":"synthetic"});
    a["emailAddress"] = json!("synthetic@example.test");
    a["remoteMcpServersConfig"] = json!([{"sourceOnly":true}]);
    let mut b = record(1);
    b["permissionMode"] = json!("plan");
    b["isStarred"] = json!(true);
    f.put(&f.a, &a, true);
    f.put(&f.b, &b, true);
    f.put(&f.a, &record(2), true);
    transfer_all(&f, &f.a, &f.b, &[f.a.clone(), f.b.clone()]);
    let rows = records(&f.dir(&f.b)).unwrap();
    let r = &rows[&name(1)].value;
    assert_eq!(r["permissionMode"], "plan");
    assert_eq!(r["isStarred"], true);
    for key in [
        "armedWorkAtQuit",
        "pendingFirstStart",
        "envScopeId",
        "emailAddress",
    ] {
        assert!(r.get(key).is_none());
    }
    assert_eq!(rows[&name(2)].value["permissionMode"], "default");
    assert_eq!(rows[&name(2)].value["remoteMcpServersConfig"], json!([]));
}

#[test]
fn continuity_rejects_changed_third_account_before_any_write() {
    let f = Fixture::new();
    let c = Identity {
        account: id(7),
        org: id(8),
    };
    Directory::open(&f.dir(&c), true).unwrap();
    f.put(&f.a, &record(1), true);
    f.put(&c, &record(3), true);
    let accounts = [f.a.clone(), f.b.clone(), c.clone()];
    let p = preview_continuity(&f.roots, &f.a, &f.b, &accounts).unwrap();
    let mut changed = record(3);
    changed["title"] = json!("changed");
    f.put(&c, &changed, false);
    let e = apply_continuity_observed(
        &f.roots,
        &f.a,
        &f.b,
        &accounts,
        &p.fingerprint,
        &f.backup(),
        &mut || Ok(()),
        None,
        &mut |_| {},
    )
    .unwrap_err();
    assert_eq!(e, "PREVIEW_CHANGED");
    assert!(records(&f.dir(&f.b)).unwrap().is_empty());
}

#[test]
fn continuity_missing_transcript_blocks_instead_of_reporting_partial_success() {
    let f = Fixture::new();
    f.put(&f.a, &record(1), false);
    f.put(&f.a, &record(2), true);
    let accounts = [f.a.clone(), f.b.clone()];
    let p = preview_continuity(&f.roots, &f.a, &f.b, &accounts).unwrap();
    assert!(p.missing > 0);
    let e = apply_continuity_observed(
        &f.roots,
        &f.a,
        &f.b,
        &accounts,
        &p.fingerprint,
        &f.backup(),
        &mut || Ok(()),
        None,
        &mut |_| {},
    )
    .unwrap_err();
    assert_eq!(e, "UNRESOLVED_SOURCE_ROWS");
    assert!(records(&f.dir(&f.b)).unwrap().is_empty());
}

#[test]
fn continuity_preserves_source_only_worktree_identity_without_rekeying() {
    let f = Fixture::new();
    let mut a = record(1);
    a["worktreePath"] = json!("/synthetic/worktree");
    a["cwd"] = a["worktreePath"].clone();
    a["worktreeName"] = json!("synthetic-tree");
    f.put(&f.a, &a, true);
    transfer_all(&f, &f.a, &f.b, &[f.a.clone(), f.b.clone()]);
    assert_eq!(
        records(&f.dir(&f.b)).unwrap()[&name(1)].value["worktreePath"],
        a["worktreePath"]
    );
}

#[test]
fn continuity_does_not_fabricate_a_worktree_owner_for_divergent_branch() {
    let f = Fixture::new();
    let mut a = record(1);
    a["worktreePath"] = json!("/synthetic/worktree");
    let mut b = record(1);
    b["cliSessionId"] = json!(id(500));
    f.put(&f.a, &a, true);
    f.put(&f.b, &b, true);
    let p = preview_continuity(&f.roots, &f.a, &f.b, &[f.a.clone(), f.b.clone()]).unwrap();
    assert!(p.missing > 0);
    assert!(p
        .issues
        .iter()
        .any(|i| i.reason == "WORKTREE_BRANCH_REQUIRES_NATIVE_FORK"));
}

#[test]
fn continuity_workspace_change_resets_grants_and_retains_target_annotations() {
    let f = Fixture::new();
    let mut a = record(1);
    a["cwd"] = json!("/synthetic/new-project");
    a["lastActivityAt"] = json!(20);
    let mut b = record(1);
    b["permissionMode"] = json!("bypassPermissions");
    b["targetAnnotation"] = json!("keep");
    f.put(&f.a, &a, true);
    f.put(&f.b, &b, true);
    transfer_all(&f, &f.a, &f.b, &[f.a.clone(), f.b.clone()]);
    let r = records(&f.dir(&f.b)).unwrap()[&name(1)].value.clone();
    assert_eq!(r["cwd"], "/synthetic/new-project");
    assert_eq!(r["permissionMode"], "default");
    assert_eq!(r["targetAnnotation"], "keep");
}

#[test]
fn continuity_later_rollback_restores_owned_record_image_for_earlier_rollback() {
    let f = Fixture::new();
    f.put(&f.a, &record(1), true);
    let accounts = [f.a.clone(), f.b.clone()];
    let first = transfer_all(&f, &f.a, &f.b, &accounts);
    let mut newer = record(1);
    newer["lastActivityAt"] = json!(100);
    newer["title"] = json!("new title");
    f.put(&f.a, &newer, false);
    let second = transfer_all(&f, &f.a, &f.b, &accounts);
    f.undo(&second);
    f.undo(&first);
    assert!(records(&f.dir(&f.b)).unwrap().is_empty());
    assert_eq!(records(&f.dir(&f.a)).unwrap()[&name(1)].value, newer);
}

#[test]
fn continuity_interrupted_publication_restores_target_and_global_manifest() {
    let f = Fixture::new();
    f.put(&f.a, &record(1), true);
    f.put(&f.a, &record(2), true);
    let accounts = [f.a.clone(), f.b.clone()];
    let preview = preview_continuity(&f.roots, &f.a, &f.b, &accounts).unwrap();
    let mut calls = 0;
    let mut published = None;
    let error = apply_continuity_observed(
        &f.roots,
        &f.a,
        &f.b,
        &accounts,
        &preview.fingerprint,
        &f.backup(),
        &mut || {
            calls += 1;
            if calls >= 4 {
                Err("SIMULATED_FAILURE".into())
            } else {
                Ok(())
            }
        },
        None,
        &mut |run| published = Some(run.to_owned()),
    )
    .unwrap_err();
    assert_eq!(error, "SIMULATED_FAILURE");
    let journal = load(&f.roots, &published.unwrap()).unwrap();
    assert_ne!(journal.summary.state, "applied");
    f.undo(&journal.summary);
    assert!(records(&f.dir(&f.b)).unwrap().is_empty());
    assert!(!continuity::state_path(&f.roots).exists());
}

#[test]
fn continuity_same_title_retains_destination_user_title_source() {
    let f = Fixture::new();
    let mut a = record(1);
    a["lastActivityAt"] = json!(20);
    a.as_object_mut().unwrap().remove("titleSource");
    let b = record(1);
    f.put(&f.a, &a, true);
    f.put(&f.b, &b, true);
    transfer_all(&f, &f.a, &f.b, &[f.a.clone(), f.b.clone()]);
    assert_eq!(
        records(&f.dir(&f.b)).unwrap()[&name(1)].value["titleSource"],
        "user"
    );
}

#[test]
fn continuity_quota_source_does_not_clear_destination_permission_error() {
    let f = Fixture::new();
    let mut a = record(1);
    a["error"] = json!("session limit reached; resets soon");
    a["errorAt"] = json!(2);
    let mut b = record(1);
    b["error"] = json!("Permission required");
    b["errorCategory"] = json!("permission");
    b["errorAt"] = json!(2);
    f.put(&f.a, &a, true);
    f.put(&f.b, &b, true);
    transfer_all(&f, &f.a, &f.b, &[f.a.clone(), f.b.clone()]);
    let r = records(&f.dir(&f.b)).unwrap()[&name(1)].value.clone();
    assert_eq!(r["error"], "Permission required");
    assert_eq!(r["errorAt"], 2);
}

#[test]
fn continuity_child_references_the_actual_source_parent_branch_after_projection() {
    let f = Fixture::new();
    let a = record(1);
    let mut b = record(1);
    b["cliSessionId"] = json!(id(500));
    let mut child = record(2);
    child["forkedFromSessionId"] = json!(sid(1));
    child["spawnedFrom"] = json!({"sessionId":sid(1),"title":"synthetic parent"});
    f.put(&f.a, &a, true);
    f.put(&f.a, &child, true);
    f.put(&f.b, &b, true);
    transfer_all(&f, &f.a, &f.b, &[f.a.clone(), f.b.clone()]);
    let rows = records(&f.dir(&f.b)).unwrap();
    let r = &rows[&name(2)].value;
    let parent = &rows[&format!("{}.json", r["forkedFromSessionId"].as_str().unwrap())].value;
    assert_eq!(parent["cliSessionId"], id(101));
    assert_eq!(r["spawnedFrom"]["sessionId"], r["forkedFromSessionId"]);
    assert_eq!(rows[&name(1)].value["cliSessionId"], id(500));
}

#[test]
fn continuity_new_optional_native_fields_do_not_block_compatible_storage() {
    let unknown = unknown_persisted_fields(&BTreeSet::from([
        "sessionId".into(),
        "cliSessionId".into(),
        "futureNativeState".into(),
    ]));
    assert_eq!(unknown, BTreeSet::from(["futureNativeState".into()]));
    for empty in [Value::Null, json!(false), json!(""), json!([]), json!({})] {
        let f = Fixture::new();
        let mut source = record(1);
        source["futureNativeState"] = empty;
        f.put(&f.a, &source, true);
        let accounts = [f.a.clone(), f.b.clone()];
        let p = preview_continuity_with_fields(&f.roots, &f.a, &f.b, &accounts, &unknown).unwrap();
        assert_eq!(p.missing, 0);
        let run = apply_continuity_with_fields_and_progress(
            &f.roots,
            &f.a,
            &f.b,
            &accounts,
            &unknown,
            &p.fingerprint,
            &f.backup(),
            &mut || Ok(()),
            None,
            &mut |_| {},
            &mut |_, _, _| {},
        )
        .unwrap();
        assert_eq!(run.created, 1);
        assert_eq!(f.get(&f.b, 1)["cliSessionId"], source["cliSessionId"]);
    }
}

#[test]
fn continuity_meaningful_new_native_state_reports_the_affected_session_and_writes_nothing() {
    let unknown = BTreeSet::from(["futureNativeState".into()]);
    let f = Fixture::new();
    let mut source = record(1);
    source["futureNativeState"] = json!({"newExecutionRoute":"synthetic"});
    f.put(&f.a, &source, true);
    f.put(&f.a, &record(2), true);
    let accounts = [f.a.clone(), f.b.clone()];
    let p = preview_continuity_with_fields(&f.roots, &f.a, &f.b, &accounts, &unknown).unwrap();
    assert_eq!(p.missing, 1);
    assert_eq!(p.issues.len(), 1);
    assert_eq!(p.issues[0].session_id, sid(1));
    assert_eq!(p.issues[0].reason, "UNSUPPORTED_PERSISTED_FIELD");
    assert_eq!(p.created, 1);
    let result = apply_continuity_with_fields_and_progress(
        &f.roots,
        &f.a,
        &f.b,
        &accounts,
        &unknown,
        &p.fingerprint,
        &f.backup(),
        &mut || Ok(()),
        None,
        &mut |_| {},
        &mut |_, _, _| {},
    );
    assert_eq!(result.unwrap_err(), "UNRESOLVED_SOURCE_ROWS");
    assert!(records(&f.dir(&f.b)).unwrap().is_empty());
    assert_eq!(f.get(&f.a, 1), source);
}

#[test]
fn continuity_rechecks_new_native_state_when_freezing_the_apply_plan() {
    let unknown = BTreeSet::from(["futureNativeState".into()]);
    let f = Fixture::new();
    let mut source = record(1);
    f.put(&f.a, &source, true);
    let accounts = [f.a.clone(), f.b.clone()];
    let p = preview_continuity_with_fields(&f.roots, &f.a, &f.b, &accounts, &unknown).unwrap();
    source["futureNativeState"] = json!("became meaningful after preview");
    f.put(&f.a, &source, false);
    let result = apply_continuity_with_fields_and_progress(
        &f.roots,
        &f.a,
        &f.b,
        &accounts,
        &unknown,
        &p.fingerprint,
        &f.backup(),
        &mut || Ok(()),
        None,
        &mut |_| {},
        &mut |_, _, _| {},
    );
    assert_eq!(result.unwrap_err(), "PREVIEW_CHANGED");
    assert!(records(&f.dir(&f.b)).unwrap().is_empty());
}

#[test]
fn continuity_storage_contract_drift_at_commit_leaves_recoverable_receipt() {
    let f = Fixture::new();
    f.put(&f.a, &record(1), true);
    let accounts = [f.a.clone(), f.b.clone()];
    let p = preview_continuity(&f.roots, &f.a, &f.b, &accounts).unwrap();
    let changed = std::cell::Cell::new(false);
    let result = apply_continuity_with_progress(
        &f.roots,
        &f.a,
        &f.b,
        &accounts,
        &p.fingerprint,
        &f.backup(),
        &mut || {
            if changed.get() {
                Err("DESKTOP_CONTRACT_CHANGED".into())
            } else {
                Ok(())
            }
        },
        Some(&mut || Ok(())),
        &mut |_| {},
        &mut |stage, _, _| {
            if stage == "verifying" {
                changed.set(true);
            }
        },
    );
    assert_eq!(result.unwrap_err(), "DESKTOP_CONTRACT_CHANGED");
    let pending = f.pending();
    rollback(&f.roots, &pending.summary.id, &mut || Ok(())).unwrap();
    assert!(records(&f.dir(&f.b)).unwrap().is_empty());
    assert_eq!(f.get(&f.a, 1), record(1));
}

#[test]
fn continuity_reports_unknown_native_state_in_older_source_or_retained_target() {
    let unknown = BTreeSet::from(["futureNativeState".into()]);
    for in_source in [true, false] {
        let f = Fixture::new();
        let mut source = record(1);
        let mut target = record(1);
        if in_source {
            source["futureNativeState"] = json!({"unknownRoute":"synthetic"});
            target["lastActivityAt"] = json!(20);
        } else {
            target["futureNativeState"] = json!({"unknownGrant":"synthetic"});
            source["lastActivityAt"] = json!(20);
            source["cwd"] = json!("/synthetic/changed-workspace");
        }
        f.put(&f.a, &source, true);
        f.put(&f.b, &target, false);
        let accounts = [f.a.clone(), f.b.clone()];
        let p = preview_continuity_with_fields(&f.roots, &f.a, &f.b, &accounts, &unknown).unwrap();
        assert_eq!(p.missing, 1);
        assert_eq!(p.issues[0].reason, "UNSUPPORTED_PERSISTED_FIELD");
        assert_eq!(p.created + p.updated, 0);
        assert_eq!(f.get(&f.a, 1), source);
        assert_eq!(f.get(&f.b, 1), target);
    }
}
