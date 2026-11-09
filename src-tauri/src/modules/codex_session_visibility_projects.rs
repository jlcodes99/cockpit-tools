// Project recovery composes with the existing quick/deep pipeline. There is no
// separate repair mode or switch-time history scan.
const QUICK_PROJECT_REPAIR_BUDGET: Duration = Duration::from_millis(500);

fn project_recovery_message(key: &str, args: &[(&str, &str)]) -> String {
    modules::i18n::translate(
        &modules::config::get_user_config().language,
        &format!("codex.sessionManager.repairModal.{key}"),
        args,
    )
}

fn project_recovery_error(error: String) -> String {
    modules::logger::log_warn(&format!("[Codex Local Projects] {error}"));
    project_recovery_message(
        if error.contains("CODEX_PROJECT_PENDING_RECOVERY") {
            "projectsPending"
        } else if error.contains("CODEX_PROJECT_CHANGED") {
            "projectsChanged"
        } else {
            "projectsFailed"
        },
        &[("error", &error)],
    )
}

fn verify_project_recovery_stopped(instance: &CodexSyncInstance) -> Result<(), String> {
    // Default instances deliberately use None through is_instance_running, so
    // official macOS/Windows desktops without CODEX_HOME are still detected.
    let entries = modules::process::collect_codex_launch_maintenance_process_entries()?;
    if is_instance_running(instance, &entries) {
        return Err(project_recovery_message("stopInstanceHint", &[]));
    }
    Ok(())
}

fn visibility_item_changed(item: &CodexSessionVisibilityRepairItem) -> bool {
    item.backup_dir.is_some()
        || item.changed_rollout_file_count > 0
        || item.updated_sqlite_row_count > 0
        || item.updated_sqlite_timestamp_row_count > 0
        || item.added_session_index_entry_count > 0
        || item.updated_session_index_entry_count > 0
        || item.inserted_catalog_row_count > 0
        || item.removed_catalog_row_count > 0
        || item.updated_global_state_entry_count > 0
        || item.history_projection_count.unwrap_or(0) > 0
}

fn combined_visibility_message(
    summary: &CodexSessionVisibilityRepairSummary,
    dry_run: bool,
) -> String {
    let running = summary
        .items
        .iter()
        .filter(|item| item.running && visibility_item_changed(item))
        .count();
    if dry_run {
        build_dry_run_summary_message(
            summary.mutated_instance_count,
            summary.changed_rollout_file_count,
            summary.updated_sqlite_row_count,
            summary.updated_sqlite_timestamp_row_count,
            summary.added_session_index_entry_count,
            summary.updated_session_index_entry_count,
            summary.inserted_catalog_row_count,
            summary.removed_catalog_row_count,
            summary.updated_global_state_entry_count,
            summary.skipped_rollout_file_count,
            running,
        )
    } else {
        build_summary_message(
            summary.mutated_instance_count,
            summary.changed_rollout_file_count,
            summary.updated_sqlite_row_count,
            summary.updated_sqlite_timestamp_row_count,
            summary.added_session_index_entry_count,
            summary.updated_session_index_entry_count,
            summary.inserted_catalog_row_count,
            summary.removed_catalog_row_count,
            summary.updated_global_state_entry_count,
            summary.skipped_rollout_file_count,
            running,
            summary.skipped_sqlite_file_count,
            summary.metadata_rebuild_failed_count,
        )
    }
}

// This is shared by manual visibility repair and the one bounded startup repair
// used by default instances, managed instances and Switch and Start.
fn repair_session_visibility_for_instances_with_options(
    options: CodexSessionVisibilityRepairOptions,
    run_id: Option<String>,
    reporter: ProgressReporter<'_>,
    selection: RepairTargetSelection,
    instances: Vec<CodexSyncInstance>,
) -> Result<CodexSessionVisibilityRepairSummary, String> {
    if options.mode == CodexSessionVisibilityRepairMode::History {
        return repair_history_projections_for_instances(
            options, run_id, reporter, selection, instances,
        );
    }
    let deep = options.mode == CodexSessionVisibilityRepairMode::Deep;
    let automatic_start = launch_visibility_budget().is_some();
    let _leases = if deep && !options.dry_run {
        let leases = instances
            .iter()
            .map(|instance| {
                modules::codex_account::try_acquire_profile_mutation_lease(
                    &instance.data_dir,
                    "deep-session-visibility-repair",
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        for instance in &instances {
            verify_project_recovery_stopped(instance)?;
        }
        // Recover unfinished project writes before provider repair can prune any
        // backup or change another field in those same databases.
        for instance in &instances {
            if local_projects::has_pending(&instance.data_dir) {
                local_projects::recover_pending(&instance.data_dir)
                    .map_err(project_recovery_error)?;
            }
        }
        leases
    } else {
        Vec::new()
    };
    let base_reporter = |mut progress: CodexSessionVisibilityRepairProgress| {
        progress.percent = (u16::from(progress.percent) * 60 / 100) as u8;
        if progress.stage == "done" {
            progress.stage = "scan_local_projects".into();
        }
        if let Some(report) = reporter {
            report(progress);
        }
    };
    // Launch selects a single stopped profile. Prepare its account-scoped
    // project registry before optional session work can exhaust the launch budget.
    let mut desktop_preparation = if selection.prepare_desktop_projects {
        instances.first().map(prepare_local_project_account_state)
    } else {
        None
    };
    let mut summary = repair_provider_visibility_for_instances_with_options(
        options,
        run_id.clone(),
        Some(&base_reporter),
        selection.clone(),
        instances.clone(),
    )?;
    if matches!(&desktop_preparation, Some(Ok(true))) {
        summary.updated_global_state_entry_count += 1;
        if let Some(item) = summary.items.first_mut() {
            item.updated_global_state_entry_count += 1;
        }
    }
    let mut totals = local_projects::Counts::default();
    for (index, instance) in instances.iter().enumerate() {
        report_repair_progress(
            reporter,
            &run_id,
            options,
            "scan_local_projects",
            60 + (index * 25 / instances.len().max(1)) as u8,
            index + 1,
            instances.len(),
            Some(instance),
        );
        let project_budget = if deep {
            Duration::from_secs(30)
        } else {
            launch_visibility_budget()
                .map(|budget| {
                    budget
                        .deadline
                        .saturating_duration_since(std::time::Instant::now())
                        .min(QUICK_PROJECT_REPAIR_BUDGET)
                })
                .unwrap_or(QUICK_PROJECT_REPAIR_BUDGET)
        };
        let result = with_launch_visibility_budget(project_budget, || {
            if local_projects::has_pending(&instance.data_dir) {
                return Err("CODEX_PROJECT_PENDING_RECOVERY".into());
            }
            desktop_preparation.take().transpose()?;
            let Some(plan) = local_projects::scan_for_repair(
                &instance.data_dir,
                selection.session_ids(),
                !deep,
            )?
            else {
                return Ok(None);
            };
            let changed = plan.changed();
            let state_changes = plan.state_changes();
            let mut backup_dir = None;
            if changed && !options.dry_run {
                // A launch already owns the profile lease. A manual quick repair
                // takes one only for its small write, without queuing behind a start.
                let _quick_lease = if !deep && automatic_start {
                    None
                } else if !deep {
                    Some(modules::codex_account::try_acquire_profile_mutation_lease(
                        &instance.data_dir,
                        "quick-project-recovery",
                    )?)
                } else {
                    None
                };
                verify_project_recovery_stopped(instance)?;
                let scope = modules::backup_storage::scope_for_path(&instance.data_dir);
                let backup = modules::backup_storage::behavior_backup_dir(
                    "codex",
                    &scope,
                    &format!("local-projects-{}", uuid::Uuid::new_v4()),
                )?;
                report_repair_progress(
                    reporter,
                    &run_id,
                    options,
                    "write_local_projects",
                    60 + (index * 25 / instances.len().max(1)) as u8,
                    index + 1,
                    instances.len(),
                    Some(instance),
                );
                local_projects::apply(&plan, &backup)?;
                backup_dir = Some(backup.to_string_lossy().into_owned());
            }
            Ok(Some((plan.counts, state_changes, changed, backup_dir)))
        });
        let outcome = match result {
            Ok(outcome) => outcome,
            Err(error)
                if !deep
                    && !error.contains("ROLLBACK_FAILED")
                    && !local_projects::has_pending(&instance.data_dir) =>
            {
                // Current metadata may be large, busy or newer than the plan.
                // Its read-only scan or rolled-back write must not delay startup.
                modules::logger::log_warn(&format!("[Codex Local Projects] bounded quick repair deferred: instance_id={}, error={error}",instance.id));
                summary.local_project_recovery_deferred = true;
                continue;
            }
            Err(error) if !deep && error.contains("CODEX_PROJECT_PENDING_RECOVERY") => {
                summary.local_project_recovery_deferred = true;
                continue;
            }
            Err(error) => {
                return Err(format!(
                    "{}; completed_backups={:?}",
                    project_recovery_error(error),
                    summary.backup_dirs
                ))
            }
        };
        let Some((counts, state_changes, changed, backup)) = outcome else {
            continue;
        };
        totals.projects += counts.projects;
        totals.assignments += counts.assignments;
        totals.thread_rows += counts.thread_rows;
        totals.catalog_rows += counts.catalog_rows;
        totals.skipped_projects += counts.skipped_projects;
        totals.ambiguous_threads += counts.ambiguous_threads;
        if changed {
            summary.updated_sqlite_row_count += counts.thread_rows + counts.catalog_rows;
            summary.updated_global_state_entry_count += state_changes;
            if let Some(item) = summary
                .items
                .iter_mut()
                .find(|item| item.instance_id == instance.id)
            {
                item.updated_sqlite_row_count += counts.thread_rows + counts.catalog_rows;
                item.updated_global_state_entry_count += state_changes;
                if backup.is_some() {
                    item.backup_dir = backup.clone();
                }
            }
        }
        if let Some(backup) = backup {
            summary.backup_dirs.push(backup);
        }
    }
    let mut history_message = None;
    if deep {
        let history_reporter = |mut progress: CodexSessionVisibilityRepairProgress| {
            progress.percent = 85 + (u16::from(progress.percent) * 14 / 100) as u8;
            if progress.stage == "done" {
                progress.stage = "reset_history_index".into();
            }
            if let Some(report) = reporter {
                report(progress);
            }
        };
        let history = repair_history_projections_for_instances(
            options,
            run_id.clone(),
            Some(&history_reporter),
            selection,
            instances.clone(),
        )
        .map_err(|error| format!("{error}; completed_backups={:?}", summary.backup_dirs))?;
        summary.history_projection_count = history.history_projection_count;
        summary.history_repair_warning = history.history_repair_warning;
        for source in history.items {
            if let Some(item) = summary
                .items
                .iter_mut()
                .find(|item| item.instance_id == source.instance_id)
            {
                item.history_projection_count = source.history_projection_count;
                if source.backup_dir.is_some() {
                    item.backup_dir = source.backup_dir;
                }
            }
        }
        summary.backup_dirs.extend(history.backup_dirs);
        if history.history_projection_count.unwrap_or(0) > 0 {
            history_message = Some(history.message);
        }
    }
    summary.mutated_instance_count = summary
        .items
        .iter()
        .filter(|item| visibility_item_changed(item))
        .count();
    summary.message = combined_visibility_message(&summary, options.dry_run);
    if totals.projects > 0
        || totals.assignments > 0
        || totals.thread_rows > 0
        || totals.catalog_rows > 0
        || totals.skipped_projects > 0
        || totals.ambiguous_threads > 0
    {
        summary.message.push_str("; ");
        summary.message.push_str(&project_recovery_message(
            if options.dry_run {
                "projectsPreview"
            } else {
                "projectsSuccess"
            },
            &[
                ("projects", &totals.projects.to_string()),
                ("assignments", &totals.assignments.to_string()),
                ("skipped", &totals.skipped_projects.to_string()),
                ("ambiguous", &totals.ambiguous_threads.to_string()),
            ],
        ));
    }
    if summary.local_project_recovery_deferred {
        summary.message.push_str("; ");
        summary
            .message
            .push_str(&project_recovery_message("projectsDeferred", &[]));
    }
    if let Some(message) = history_message {
        summary.message.push_str("; ");
        summary.message.push_str(&message);
    }
    summary.local_project_recovery = Some(totals);
    report_repair_progress(
        reporter,
        &run_id,
        options,
        "done",
        100,
        instances.len(),
        instances.len(),
        None,
    );
    Ok(summary)
}

// Switch and launch both hold the profile mutation lease. Resolve the owner
// from the committed credentials and only write while the Desktop is stopped.
fn prepare_local_project_account_state(instance: &CodexSyncInstance) -> Result<bool, String> {
    let budget = launch_visibility_budget().map_or(QUICK_PROJECT_REPAIR_BUDGET, |parent| {
        parent
            .deadline
            .saturating_duration_since(std::time::Instant::now())
            .min(QUICK_PROJECT_REPAIR_BUDGET)
    });
    with_launch_visibility_budget(budget, || {
        check_launch_visibility_budget("read_project_owner")?;
        if local_projects::has_pending(&instance.data_dir) {
            return Err("CODEX_PROJECT_PENDING_RECOVERY".into());
        }
        let Some(owner) = modules::codex_account::desktop_renderer_project_owner(&instance.data_dir)? else {
            return Ok(false);
        };
        verify_project_recovery_stopped(instance)?;
        let prepared = local_projects::account_state::prepare(&instance.data_dir, owner.as_deref())?;
        if prepared {
            modules::logger::log_info(&format!(
                "[Codex Local Projects] prepared account-scoped local groups: instance_id={}",
                instance.id,
            ));
        }
        Ok(prepared)
    })
}

pub(crate) fn prepare_desktop_project_state_after_switch(
    instance_id: &str,
    data_dir: &Path,
) -> Result<bool, String> {
    prepare_local_project_account_state(&CodexSyncInstance {
        id: instance_id.to_string(),
        name: instance_id.to_string(),
        data_dir: data_dir.to_path_buf(),
        last_pid: None,
    })
}
