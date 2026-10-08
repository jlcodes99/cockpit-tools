//! Copy client profile content without making NTFS EFS metadata a prerequisite.
use std::fs;
use std::io;
use std::path::Path;

pub(crate) fn copy_profile_file(source: &Path, target: &Path) -> io::Result<u64> {
    let result = fs::copy(source, target);
    #[cfg(windows)]
    return recover_efs_copy(source, target, result);
    #[cfg(not(windows))]
    result
}

#[cfg(any(windows, test))]
fn recover_efs_copy(source: &Path, target: &Path, result: io::Result<u64>) -> io::Result<u64> {
    match result {
        // CopyFile may fail to reproduce Application Protected EFS attributes
        // outside AppData even though this user can read the source content.
        // Sharing violations and access denied must continue to fail normally.
        Err(error) if error.raw_os_error() == Some(6000) => {
            if source == target
                || fs::canonicalize(source)
                    .ok()
                    .zip(fs::canonicalize(target).ok())
                    .is_some_and(|(source, target)| source == target)
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "profile copy source equals target",
                ));
            }
            let mut input = fs::File::open(source)?;
            let mut output = fs::File::create(target)?;
            // io::copy streams bounded chunks; it does not load a profile DB into memory.
            let copied = io::copy(&mut input, &mut output)?;
            output.sync_all()?;
            Ok(copied)
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovers_encryption_attribute_failure_without_changing_source_content() {
        let root = std::env::temp_dir().join(format!("cockpit-efs-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let source = root.join("Local State");
        let target = root.join("backup");
        let content = vec![123; 1024 * 1024];
        fs::write(&source, &content).unwrap();
        assert_eq!(
            recover_efs_copy(&source, &target, Err(io::Error::from_raw_os_error(6000))).unwrap(),
            content.len() as u64
        );
        assert_eq!(fs::read(&source).unwrap(), content);
        assert_eq!(fs::read(&target).unwrap(), content);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn does_not_bypass_sharing_or_permission_failures() {
        for code in [5, 32, 33] {
            let result = recover_efs_copy(
                Path::new("missing-source"),
                Path::new("missing-target"),
                Err(io::Error::from_raw_os_error(code)),
            );
            assert_eq!(result.unwrap_err().raw_os_error(), Some(code));
        }
    }

    #[test]
    fn rejects_fallback_copy_to_source_itself() {
        let source = Path::new("same-file");
        let result = recover_efs_copy(source, source, Err(io::Error::from_raw_os_error(6000)));
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }
}
