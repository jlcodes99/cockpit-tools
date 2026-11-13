// Project recovery can run independently of the provider and history repair.
const QUICK_PROJECT_REPAIR_BUDGET: Duration = Duration::from_millis(500);

fn quick_project_repair_budget(data_dir: &Path) -> Duration {
    let large_state = [GLOBAL_STATE_FILE.to_string(), format!("{GLOBAL_STATE_FILE}.bak")]
        .iter()
        .any(|name| std::fs::metadata(data_dir.join(name)).is_ok_and(|m| m.len() > 2_000_000));
    // Multi-account archives need more time to parse and atomically preserve.
    // Startup still caps this work at the remaining five-second launch budget.
    if large_state {
        Duration::from_secs(2)
    } else {
        QUICK_PROJECT_REPAIR_BUDGET
    }
}

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

fn project_deferral_message(data_dir: &Path, error: &str) -> String {
    let reason = if error.contains("CODEX_PROJECT_STATE_TOO_LARGE") {
        project_recovery_message("projectsStateTooLarge", &[])
    } else if error.contains("CODEX_PROJECT_QUICK_LIMIT")
        || error.contains(LAUNCH_VISIBILITY_TIMEOUT)
    {
        project_recovery_message("projectsQuickBudget", &[])
    } else {
        project_recovery_error(error.to_owned())
    };
    let size = [GLOBAL_STATE_FILE.to_string(), format!("{GLOBAL_STATE_FILE}.bak")]
        .into_iter()
        .filter_map(|name| {
            std::fs::metadata(data_dir.join(&name))
                .ok()
                .map(|metadata| (name, metadata.len()))
        })
        .max_by_key(|(_, bytes)| *bytes)
        .map(|(name, bytes)| project_recovery_message(
            "projectsStateSize",
            &[("file", &name), ("bytes", &bytes.to_string())],
        ));
    [Some(reason), size, Some(project_recovery_message("projectsDeferredAction", &[]))]
        .into_iter().flatten().collect::<Vec<_>>().join(" ")
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
    let project_only = options.mode == CodexSessionVisibilityRepairMode::Projects;
    let thorough_projects = deep || project_only;
    let automatic_start = launch_visibility_budget().is_some();
    if instances.is_empty() {
        return Err("未找到要修复的 Codex 实例".to_string());
    }
    let _leases = if thorough_projects && !options.dry_run {
        let leases = instances
            .iter()
            .map(|instance| {
                modules::codex_account::try_acquire_profile_mutation_lease(
                    &instance.data_dir,
                    if project_only { "local-project-recovery" } else { "deep-session-visibility-repair" },
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        for instance in &instances {
            verify_project_recovery_stopped(instance)?;
        }
        leases
    } else {
        Vec::new()
    };
    let history_preflight_reporter = |mut progress: CodexSessionVisibilityRepairProgress| {
        progress.percent = 2;
        if let Some(report) = reporter {
            report(progress);
        }
    };
    let mut history_plans = if deep {
        let plans = scan_history_projections_for_instances(
            options, &run_id, Some(&history_preflight_reporter), &selection, &instances,
        )?;
        if !options.dry_run {
            if let Some(warning) = history_duplicate_ordinals_warning(&instances, &plans) {
                return Err(warning);
            }
        }
        Some(plans)
    } else {
        None
    };
    if thorough_projects && !options.dry_run {
        // Preflight finished; recover pending project writes before provider repair.
        for instance in &instances {
            if local_projects::has_pending(&instance.data_dir) {
                local_projects::recover_pending(&instance.data_dir)
                    .map_err(project_recovery_error)?;
            }
        }
    }
    let base_reporter = |mut progress: CodexSessionVisibilityRepairProgress| {
        progress.percent = (u16::from(progress.percent) * 60 / 100) as u8;
        if progress.stage == "done" {
            progress.stage = "scan_local_projects".into();
        }
        if let Some(report) = reporter {
            report(progress);
        }
    };
    let mut summary = if project_only {
        CodexSessionVisibilityRepairSummary {
            instance_count: instances.len(),
            items: instances.iter().map(|instance| CodexSessionVisibilityRepairItem {
                instance_id: instance.id.clone(),
                instance_name: instance.name.clone(),
                ..Default::default()
            }).collect(),
            ..Default::default()
        }
    } else {
        repair_provider_visibility_for_instances_with_options(
            options,
            run_id.clone(),
            Some(&base_reporter),
            selection.clone(),
            instances.clone(),
        )?
    };
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
        let project_budget = if thorough_projects {
            Duration::from_secs(30)
        } else {
            let quick_budget = quick_project_repair_budget(&instance.data_dir);
            launch_visibility_budget()
                .map(|budget| {
                    budget
                        .deadline
                        .saturating_duration_since(std::time::Instant::now())
                        .min(quick_budget)
                })
                .unwrap_or(quick_budget)
        };
        let result = with_launch_visibility_budget(project_budget, || {
            if local_projects::has_pending(&instance.data_dir) {
                return Err("CODEX_PROJECT_PENDING_RECOVERY".into());
            }
            let Some(plan) = local_projects::scan_for_repair(
                &instance.data_dir,
                selection.session_ids(),
                !thorough_projects,
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
                let _quick_lease = if !thorough_projects && automatic_start {
                    None
                } else if !thorough_projects {
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
                if !thorough_projects
                    && !error.contains("ROLLBACK_FAILED")
                    && !local_projects::has_pending(&instance.data_dir) =>
            {
                // Current metadata may be large, busy or newer than the plan.
                // Its read-only scan or rolled-back write must not delay startup.
                modules::logger::log_warn(&format!("[Codex Local Projects] bounded quick repair deferred: instance_id={}, error={error}",instance.id));
                summary.local_project_recovery_deferred = true;
                summary.local_project_recovery_deferred_message
                    .get_or_insert_with(|| project_deferral_message(&instance.data_dir, &error));
                continue;
            }
            Err(error) if !thorough_projects && error.contains("CODEX_PROJECT_PENDING_RECOVERY") => {
                summary.local_project_recovery_deferred = true;
                summary.local_project_recovery_deferred_message
                    .get_or_insert_with(|| project_deferral_message(&instance.data_dir, &error));
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
        // Provider migration can rewrite rollout bytes. Otherwise the original
        // evidence remains usable and apply checks it again before deleting any index.
        let plans = if options.dry_run || summary.changed_rollout_file_count == 0 {
            history_plans.take().expect("full repair history was preflighted")
        } else {
            scan_history_projections_for_instances(
                options, &run_id, Some(&history_reporter), &selection, &instances,
            )
            .map_err(|error| format!("{error}; completed_backups={:?}", summary.backup_dirs))?
        };
        let history = apply_history_projection_plans(
            options,
            run_id.clone(),
            Some(&history_reporter),
            &instances,
            plans,
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
    if !project_only {
        summary.message = combined_visibility_message(&summary, options.dry_run);
    }
    if project_only || totals.projects > 0
        || totals.assignments > 0
        || totals.thread_rows > 0
        || totals.catalog_rows > 0
        || totals.skipped_projects > 0
        || totals.ambiguous_threads > 0
    {
        if !summary.message.is_empty() {
            summary.message.push_str("; ");
        }
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
    if let Some(detail) = &summary.local_project_recovery_deferred_message {
        summary.message.push_str("; ");
        summary.message.push_str(detail);
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
}

pub(crate) fn prepare_desktop_project_state_after_switch(
    instance_id: &str,
    data_dir: &Path,
) -> Result<bool, String> {
    let instance = CodexSyncInstance {
        id: instance_id.to_string(),
        name: instance_id.to_string(),
        data_dir: data_dir.to_path_buf(),
        last_pid: None,
    };
    with_launch_visibility_budget(quick_project_repair_budget(data_dir), || {
        prepare_local_project_account_state(&instance)
    })
}
