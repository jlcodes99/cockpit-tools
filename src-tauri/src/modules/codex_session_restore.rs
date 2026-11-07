// Restore durable history first, then append official-compatible index entries.
fn restore_trashed_session_entry(
    entry: &TrashedSessionEntry,
) -> Result<RestoreTrashedSessionOutcome, String> {
    restore_trashed_session_entry_with_metadata_rebuild(entry, true)
}

fn restore_trashed_session_entry_with_metadata_rebuild(
    entry: &TrashedSessionEntry,
    rebuild_metadata: bool,
) -> Result<RestoreTrashedSessionOutcome, String> {
    if !entry.trashed_rollout_path.exists() {
        return Err(format!(
            "废纸篓中的会话文件不存在，无法恢复 ({}): {}",
            entry.manifest.session_id,
            entry.trashed_rollout_path.display()
        ));
    }

    let session_id = entry.manifest.session_id.clone();
    if let Some(trashed_session_id) = rollout_session_id(&entry.trashed_rollout_path)? {
        if trashed_session_id != session_id {
            return Err(format!(
                "废纸篓中的会话文件与清单不一致，无法恢复 (清单: {}, 文件: {}): {}",
                session_id,
                trashed_session_id,
                entry.trashed_rollout_path.display()
            ));
        }
    }

    let target_rollout_path = entry.manifest.original_rollout_path.clone();
    let published = restore_publish::copy_if_absent(
        &entry.trashed_rollout_path,
        &target_rollout_path,
        |staged| {
            if rollout_session_id(staged)?.as_deref() != Some(session_id.as_str()) {
                return Err(format!(
                    "废纸篓中的会话文件与清单不一致，无法恢复 ({}): {}",
                    session_id,
                    entry.trashed_rollout_path.display()
                ));
            }
            Ok(())
        },
    )
    .map_err(|error| {
        format!(
            "恢复会话文件失败 ({} -> {}): {}",
            entry.trashed_rollout_path.display(),
            target_rollout_path.display(),
            error
        )
    })?;

    if !published {
        match rollout_session_id(&target_rollout_path)? {
            Some(existing_session_id) if existing_session_id == session_id => {}
            Some(existing_session_id) => {
                return Err(format!(
                    "目标位置已存在不同会话文件，为避免覆盖，无法恢复 (待恢复: {}, 已存在: {}): {}",
                    session_id,
                    existing_session_id,
                    target_rollout_path.display()
                ));
            }
            None => {
                return Err(format!(
                    "目标位置已存在无法确认会话 ID 的文件，为避免覆盖，无法恢复 ({}): {}",
                    session_id,
                    target_rollout_path.display()
                ));
            }
        }
    }

    let session_index_entry = build_restored_session_index_entry(entry, &target_rollout_path);
    upsert_session_index_with_entry(
        &entry.manifest.instance_root,
        &session_id,
        &session_index_entry,
    )
    .map_err(|error| {
        modules::i18n::translate(
            &modules::config::get_user_config().language,
            "codex.sessionManager.messages.restoreIndexRetry",
            &[("error", &error)],
        )
    })?;
    // Publication is intentionally monotonic. An index failure leaves both the
    // durable target and the trash original available for an idempotent retry.

    let mut metadata_rebuild_failed = false;
    if rebuild_metadata {
        if let Err(error) = modules::codex_official_app_server::rebuild_thread_metadata(
            &entry.manifest.instance_root,
        ) {
            metadata_rebuild_failed = true;
            modules::logger::log_warn(&format!(
                "会话已恢复，但官方 Codex 重建会话索引失败 ({}): {}",
                entry.manifest.instance_name, error
            ));
        }
    }

    if let Err(error) = fs::remove_dir_all(&entry.entry_dir) {
        modules::logger::log_warn(&format!(
            "会话已恢复，但清理废纸篓条目失败 ({}): {}",
            entry.entry_dir.display(),
            error
        ));
    } else {
        cleanup_empty_trash_ancestors(&entry.entry_dir);
    }

    Ok(RestoreTrashedSessionOutcome {
        metadata_rebuild_failed,
    })
}
