fn prepare_export_session_entries(
    mut entries: Vec<(CodexSyncInstance, ThreadSnapshot)>,
) -> Result<(tempfile::TempDir, Vec<(CodexSyncInstance, ThreadSnapshot)>), String> {
    let staging = tempfile::tempdir().map_err(|error| history_read_error(&error.to_string()))?;
    let mut catalog = portable_history::Catalog::default();
    for (number, (_, snapshot)) in entries.iter_mut().enumerate() {
        prepare_export_session_entry(
            snapshot,
            staging.path().join(number.to_string()),
            &mut catalog,
        )?;
    }
    Ok((staging, entries))
}

fn prepare_export_session_entry(
    snapshot: &mut ThreadSnapshot,
    root: PathBuf,
    catalog: &mut portable_history::Catalog,
) -> Result<(), String> {
    let mut prepare = || -> Result<PathBuf, String> {
        let source = &snapshot.rollout_path;
        let relative = PathBuf::from(snapshot_relative_rollout_path(snapshot));
        let mut relative = if rollout_reader::is_compressed(source) {
            relative.with_extension("")
        } else {
            relative
        };
        let before = fs::metadata(source).map_err(|e| e.to_string())?;
        let metadata = rollout_reader::metadata(source)
            .map_err(|e| e.to_string())?
            .ok_or("missing session metadata")?;
        if portable_history::is_paginated(&metadata) {
            // Use the logical ID in the standalone filename, including after revert.
            if let Some((logical, physical)) =
                modules::codex_history_projection_repair::rollout_file_ids(&relative)
            {
                if logical != physical {
                    let filename = relative
                        .file_name()
                        .and_then(|name| name.to_str())
                        .ok_or("invalid rollout filename")?;
                    relative.set_file_name(format!("{}-{logical}.jsonl", &filename[..27]));
                }
            }
        }
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("missing staging directory")?)
            .map_err(|e| e.to_string())?;
        if portable_history::is_paginated(&metadata) {
            portable_history::write_portable_rollout(
                &snapshot.source_root,
                source,
                metadata,
                &path,
                catalog,
            )?;
        } else if rollout_reader::is_compressed(source) {
            rollout_reader::materialize_bounded(source, &path).map_err(|e| e.to_string())?;
        } else {
            let mut input = File::open(source)
                .map_err(|e| e.to_string())?
                .take(before.len());
            let mut output = File::create(&path).map_err(|e| e.to_string())?;
            if std::io::copy(&mut input, &mut output).map_err(|e| e.to_string())? != before.len() {
                return Err("rollout changed during export".into());
            }
        }
        let after = fs::metadata(source).map_err(|e| e.to_string())?;
        if after.len() != before.len() || after.modified().ok() != before.modified().ok() {
            return Err("rollout changed during export".into());
        }
        modules::codex_session_import_paths::ensure_standalone_rollout_reader(BufReader::new(
            File::open(&path).map_err(|e| e.to_string())?,
        ))?;
        Ok(path)
    };
    let path = prepare().map_err(|error| history_read_error(&error))?;
    snapshot.rollout_path = path;
    snapshot.source_root = root;
    Ok(())
}
