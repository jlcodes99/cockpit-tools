// Keep compressed source history intact. Packages always contain verified plain
// JSONL with size/hash calculated after decompression, never compressed bytes
// hidden behind a .jsonl entry name.
fn prepare_export_session_entries(
    mut entries: Vec<(CodexSyncInstance, ThreadSnapshot)>,
) -> Result<(tempfile::TempDir, Vec<(CodexSyncInstance, ThreadSnapshot)>), String> {
    let staging = tempfile::tempdir().map_err(|error| history_read_error(&error.to_string()))?;
    for (number, (_, snapshot)) in entries.iter_mut().enumerate() {
        if rollout_reader::is_compressed(&snapshot.rollout_path) {
            let root = staging.path().join(number.to_string());
            let relative = snapshot_relative_rollout_path(snapshot);
            let relative = PathBuf::from(relative).with_extension("");
            let path = root.join(relative);
            fs::create_dir_all(path.parent().ok_or("export staging has no parent")?)
                .map_err(|error| history_read_error(&error.to_string()))?;
            rollout_reader::materialize_bounded(&snapshot.rollout_path, &path)
                .map_err(|error| history_read_error(&error.to_string()))?;
            snapshot.rollout_path = path;
            snapshot.source_root = root;
        }
        modules::codex_session_import_paths::ensure_standalone_rollout_reader(BufReader::new(
            File::open(&snapshot.rollout_path).map_err(|error| error.to_string())?,
        ))?;
    }
    Ok((staging, entries))
}
