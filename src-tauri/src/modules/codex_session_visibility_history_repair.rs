// Manual, isolated history-index recovery. Automatic launch/visibility repair never enters here.
fn history_repair_message(key: &str, args: &[(&str, &str)]) -> String {
    modules::i18n::translate(
        &modules::config::get_user_config().language,
        &format!("codex.sessionManager.repairModal.{key}"),
        args,
    )
}

fn history_repair_error(error: String) -> String {
    if error.starts_with("CODEX_HISTORY_REPAIR_RUNNING") {
        history_repair_message("historyRunning", &[])
    } else {
        history_repair_message("historyFailed", &[("error", &error)])
    }
}

fn repair_history_projections_for_instances(
    options: CodexSessionVisibilityRepairOptions,
    run_id: Option<String>,
    progress_reporter: ProgressReporter<'_>,
    selection: RepairTargetSelection,
    instances: Vec<CodexSyncInstance>,
) -> Result<CodexSessionVisibilityRepairSummary, String> {
    use modules::{
        codex_history_projection_guard as guard, codex_history_projection_repair as history,
    };

    if instances.is_empty() {
        return Err(history_repair_error(
            "CODEX_HISTORY_REPAIR_NO_INSTANCE".into(),
        ));
    }
    let mut summary = CodexSessionVisibilityRepairSummary {
        instance_count: instances.len(),
        history_projection_count: Some(0),
        ..Default::default()
    };
    // Validate every selected instance before the first mutation. A malformed
    // chain in a later instance must not turn a failed preview into a partial repair.
    let mut plans = Vec::with_capacity(instances.len());
    for (index, instance) in instances.iter().enumerate() {
        let progress = |current: usize, total: usize| {
            report_repair_progress(
                progress_reporter,
                &run_id,
                options,
                "scan_history_index",
                instance_progress_percent(index, instances.len(), 0, 4),
                current,
                total,
                Some(instance),
            );
        };
        let plan = history::scan(&instance.data_dir, selection.session_ids(), &progress)
            .map_err(history_repair_error)?;
        let stopped = if plan.candidate_count() == 0 {
            Ok(())
        } else {
            guard::ensure_stopped(&instance.data_dir)
        };
        let running = match stopped {
            Ok(()) => false,
            Err(error) if error.starts_with("CODEX_HISTORY_REPAIR_RUNNING") => true,
            Err(error) => return Err(history_repair_error(error)),
        };
        if running && !options.dry_run && plan.candidate_count() > 0 {
            return Err(history_repair_message("historyRunning", &[]));
        }
        plans.push((plan, running));
    }
    let mut total = 0;
    for (index, (instance, (plan, running))) in instances.iter().zip(plans).enumerate() {
        let candidates = plan.candidate_count();
        let mut backup_dir = None;
        if candidates > 0 && !options.dry_run {
            guard::ensure_stopped(&instance.data_dir).map_err(history_repair_error)?;
            let scope = modules::backup_storage::scope_for_path(&instance.data_dir);
            let backup = modules::backup_storage::behavior_backup_dir(
                "codex",
                &scope,
                &format!(
                    "history-index-{}-{}",
                    Utc::now().format("%Y%m%d-%H%M%S"),
                    uuid::Uuid::new_v4()
                ),
            )
            .map_err(history_repair_error)?;
            report_repair_progress(
                progress_reporter,
                &run_id,
                options,
                "reset_history_index",
                instance_progress_percent(index, instances.len(), 2, 4),
                index + 1,
                instances.len(),
                Some(instance),
            );
            let repaired = history::apply(&instance.data_dir, &plan, &backup, &|| {
                guard::ensure_stopped(&instance.data_dir)
            })
            .map_err(|error| {
                // A later instance can change after preflight. Keep completed
                // repairs and their backups observable rather than claiming an
                // all-or-nothing outcome across independent SQLite databases.
                let context = format!(
                    "{error}; repaired_projections={total}; completed_backups={:?}; backup={}",
                    summary.backup_dirs, backup.display(),
                );
                history_repair_error(if total > 0 {
                    format!("CODEX_HISTORY_REPAIR_PARTIAL: {context}")
                } else {
                    context
                })
            })?;
            total += repaired;
            let path = backup.to_string_lossy().into_owned();
            modules::logger::log_info(&format!(
                "[Codex History Index] backup={}, reset_projections={repaired}", path,
            ));
            summary.backup_dirs.push(path.clone());
            backup_dir = Some(path);
        } else {
            total += candidates;
        }
        if candidates > 0 {
            summary.mutated_instance_count += 1;
        }
        summary.items.push(CodexSessionVisibilityRepairItem {
            instance_id: instance.id.clone(),
            instance_name: instance.name.clone(),
            running,
            backup_dir,
            ..Default::default()
        });
        modules::logger::log_info(&format!(
            "[Codex History Index] instance_id={}, preview={}, scanned_files={}, affected_projections={candidates}",
            instance.id, options.dry_run, plan.scanned_files(),
        ));
    }
    summary.history_projection_count = Some(total);
    summary.message = history_repair_message(
        if total == 0 {
            "historyNoChanges"
        } else if options.dry_run {
            "historyPreview"
        } else {
            "historySuccess"
        },
        &[("count", &total.to_string())],
    );
    report_repair_progress(
        progress_reporter,
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
