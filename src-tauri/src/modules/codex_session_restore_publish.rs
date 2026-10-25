//! Publish restored history without ever overwriting a competing durable file.
use std::{fs, io, path::Path};

pub(super) fn copy_if_absent(
    source: &Path,
    target: &Path,
    validate_staged: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<bool, String> {
    copy_if_absent_with(source, target, validate_staged, || Ok(()))
}

fn copy_if_absent_with(
    source: &Path,
    target: &Path,
    validate_staged: impl FnOnce(&Path) -> Result<(), String>,
    before_publish: impl FnOnce() -> Result<(), String>,
) -> Result<bool, String> {
    if target.symlink_metadata().is_ok() {
        return Ok(false);
    }
    let source_metadata = fs::symlink_metadata(source).map_err(|e| e.to_string())?;
    if !source_metadata.is_file() || source_metadata.file_type().is_symlink() {
        return Err("restored rollout source is not a regular file".into());
    }
    let parent = target
        .parent()
        .ok_or("restored rollout target has no parent")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let temp = parent.join(format!(
        ".cockpit-session-restore-{}.tmp",
        uuid::Uuid::new_v4()
    ));
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut output = options.open(&temp).map_err(|e| e.to_string())?;
        let mut input = fs::File::open(source).map_err(|e| e.to_string())?;
        let copied = io::copy(&mut input, &mut output).map_err(|e| e.to_string())?;
        let after = input.metadata().map_err(|e| e.to_string())?;
        if copied != source_metadata.len()
            || after.len() != source_metadata.len()
            || after.modified().ok() != source_metadata.modified().ok()
        {
            return Err("restored rollout source changed while copying".into());
        }
        if let Ok(modified) = source_metadata.modified() {
            output.set_modified(modified).map_err(|e| e.to_string())?;
        }
        output.sync_all().map_err(|e| e.to_string())?;
        drop(output);
        validate_staged(&temp)?;
        before_publish()?;
        match fs::hard_link(&temp, target) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(false),
            Err(error) => Err(error.to_string()),
        }
    })();
    // Only this private staging name is removed. Published history is monotonic,
    // even if a later index update fails or another writer appends to the target.
    let _ = fs::remove_file(&temp);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_restore_publish_never_overwrites_a_target_created_after_preflight() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.jsonl");
        let target = directory.path().join("sessions/target.jsonl");
        fs::write(&source, b"original trash history\n").unwrap();
        let published = copy_if_absent_with(
            &source,
            &target,
            |_| Ok(()),
            || {
                fs::write(&target, b"new concurrent history\n").unwrap();
                Ok(())
            },
        )
        .unwrap();
        assert!(!published);
        assert_eq!(fs::read(&target).unwrap(), b"new concurrent history\n");
        assert_eq!(fs::read(&source).unwrap(), b"original trash history\n");
        assert_eq!(fs::read_dir(target.parent().unwrap()).unwrap().count(), 1);
    }

    #[test]
    fn session_restore_publish_validates_before_publication_and_preserves_source() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.jsonl");
        let target = directory.path().join("sessions/target.jsonl");
        fs::write(&source, b"original history\n").unwrap();
        assert!(copy_if_absent(&source, &target, |_| Err("identity mismatch".into())).is_err());
        assert!(!target.exists());
        assert_eq!(fs::read(&source).unwrap(), b"original history\n");
        assert_eq!(fs::read_dir(target.parent().unwrap()).unwrap().count(), 0);
        assert!(copy_if_absent(&source, &target, |_| Ok(())).unwrap());
        assert_eq!(fs::read(&target).unwrap(), fs::read(&source).unwrap());
        assert!(!copy_if_absent(&source, &target, |_| panic!(
            "existing target is never staged"
        ))
        .unwrap());
    }
}
