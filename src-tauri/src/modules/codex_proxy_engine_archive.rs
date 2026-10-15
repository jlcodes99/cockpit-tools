//! Bounded extraction of pinned official engine archives into a private staging directory.
use super::codex_proxy_engine_install::InstallControl;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

const COMPRESSED_LIMIT: u64 = 128 * 1024 * 1024;
const EXTRACTED_LIMIT: u64 = 512 * 1024 * 1024;
const ENTRY_LIMIT: usize = 128;
const CHUNK: usize = 64 * 1024;
const ARCHIVE: &str = "ENGINE_INSTALL_ARCHIVE";
const IO: &str = "ENGINE_INSTALL_IO";
const TOO_LARGE: &str = "ENGINE_INSTALL_TOO_LARGE";

fn open_regular(path: &Path) -> Result<File, String> {
    if !fs::symlink_metadata(path).map_err(|_| IO)?.is_file() {
        return Err(ARCHIVE.into());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    let file = options.open(path).map_err(|_| IO)?;
    if !file.metadata().map_err(|_| IO)?.is_file() {
        return Err(ARCHIVE.into());
    }
    Ok(file)
}

fn digest(file: &mut File, limit: u64, control: &InstallControl) -> Result<String, String> {
    if file.metadata().map_err(|_| IO)?.len() > limit {
        return Err(TOO_LARGE.into());
    }
    let mut hasher = Sha256::new();
    let mut buffer = [0; CHUNK];
    let mut total = 0u64;
    loop {
        control.check()?;
        let count = file.read(&mut buffer).map_err(|_| IO)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > limit {
            return Err(TOO_LARGE.into());
        }
        hasher.update(&buffer[..count]);
    }
    control.check()?;
    Ok(format!("{:x}", hasher.finalize()))
}

pub(crate) fn file_sha256(path: &Path, control: &InstallControl) -> Result<String, String> {
    digest(&mut open_regular(path)?, EXTRACTED_LIMIT, control)
}

/// Hash before creating any output, then extract using the same open file handle.
/// The caller owns the private staging directory and removes it on any failure.
pub(crate) fn prepare(
    archive: &Path,
    output: &Path,
    asset_name: &str,
    expected_sha256: &str,
    control: &InstallControl,
) -> Result<BTreeMap<String, String>, String> {
    control.check()?;
    control.phase("verifying");
    let mut file = open_regular(archive)?;
    let actual = digest(&mut file, COMPRESSED_LIMIT, control)?;
    if expected_sha256.len() != 64 || !actual.eq_ignore_ascii_case(expected_sha256) {
        return Err("ENGINE_INSTALL_CHECKSUM".into());
    }
    if !fs::symlink_metadata(output).map_err(|_| IO)?.is_dir() {
        return Err(ARCHIVE.into());
    }
    file.seek(SeekFrom::Start(0)).map_err(|_| IO)?;
    control.phase("extracting");
    let windows = asset_name.ends_with(".zip");
    let mut state = Extraction {
        output,
        control,
        windows,
        upstream_executable: asset_name
            .strip_suffix(".zip")
            .and_then(|name| name.rsplit_once("-v"))
            .map(|(name, _)| format!("{name}.exe")),
        paths: HashSet::new(),
        files: BTreeMap::new(),
        total: 0,
    };
    if windows {
        extract_zip(file, &mut state)?;
    } else if asset_name.ends_with(".tar.gz") {
        extract_tar(file, &mut state)?;
    } else if asset_name.ends_with(".gz") {
        extract_gzip(file, &mut state)?;
    } else {
        return Err(ARCHIVE.into());
    }
    control.check()?;
    let executable = if windows { "mihomo.exe" } else { "mihomo" };
    if !state.files.contains_key(executable) {
        return Err(ARCHIVE.into());
    }
    Ok(state.files)
}

fn safe_path(name: &str, directory: bool) -> Result<&str, String> {
    let name = if directory {
        name.strip_suffix('/').unwrap_or(name)
    } else {
        name
    };
    if name.is_empty() || name.contains(['\\', ':', '\0']) || name.starts_with('/') {
        return Err(ARCHIVE.into());
    }
    for part in name.split('/') {
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.ends_with(['.', ' '])
            || part.chars().any(char::is_control)
        {
            return Err(ARCHIVE.into());
        }
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && matches!(stem.as_bytes()[3], b'1'..=b'9'))
        {
            return Err(ARCHIVE.into());
        }
    }
    Ok(name)
}

fn allowed_name(name: &str, windows: bool) -> bool {
    let engine = if windows { "mihomo.exe" } else { "mihomo" };
    name == engine
        || ["LICENSE", "NOTICE"].iter().any(|prefix| {
            name == *prefix
                || name.strip_prefix(*prefix).is_some_and(|tail| {
                    matches!(tail.chars().next(), Some('.' | '-'))
                        && tail.len() <= 64
                        && tail
                            .chars()
                            .all(|ch| ch.is_ascii_alphanumeric() || ".-_".contains(ch))
                })
        })
}

struct Extraction<'a> {
    output: &'a Path,
    control: &'a InstallControl,
    windows: bool,
    upstream_executable: Option<String>,
    paths: HashSet<String>,
    files: BTreeMap<String, String>,
    total: u64,
}

impl Extraction<'_> {
    fn entry(
        &mut self,
        name: &str,
        directory: bool,
        size: u64,
        reader: &mut impl Read,
    ) -> Result<(), String> {
        self.control.check()?;
        let path = safe_path(name, directory)?;
        if self.paths.len() >= ENTRY_LIMIT {
            return Err(TOO_LARGE.into());
        }
        if !self.paths.insert(path.to_ascii_lowercase()) {
            return Err(ARCHIVE.into());
        }
        if directory {
            return if size == 0 {
                Ok(())
            } else {
                Err(ARCHIVE.into())
            };
        }
        if size > EXTRACTED_LIMIT - self.total {
            return Err(TOO_LARGE.into());
        }
        let basename = path.rsplit('/').next().ok_or(ARCHIVE)?;
        let basename = if self.windows && self.upstream_executable.as_deref() == Some(basename) {
            "mihomo.exe"
        } else {
            basename
        };
        let mut destination = if allowed_name(basename, self.windows) {
            if self
                .files
                .keys()
                .any(|key| key.eq_ignore_ascii_case(basename))
            {
                return Err(ARCHIVE.into());
            }
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(if basename == "mihomo" { 0o700 } else { 0o600 });
            }
            Some(options.open(self.output.join(basename)).map_err(|_| IO)?)
        } else {
            None
        };
        let mut hasher = Sha256::new();
        let mut buffer = [0; CHUNK];
        let mut copied = 0u64;
        loop {
            self.control.check()?;
            let count = reader.read(&mut buffer).map_err(archive_error)?;
            if count == 0 {
                break;
            }
            copied += count as u64;
            self.total += count as u64;
            if self.total > EXTRACTED_LIMIT || copied > size {
                return Err(TOO_LARGE.into());
            }
            if let Some(file) = destination.as_mut() {
                file.write_all(&buffer[..count]).map_err(|_| IO)?;
                hasher.update(&buffer[..count]);
            }
        }
        if copied != size {
            return Err(ARCHIVE.into());
        }
        if let Some(file) = destination {
            self.control.check()?;
            file.sync_all().map_err(|_| IO)?;
            self.files
                .insert(basename.into(), format!("{:x}", hasher.finalize()));
        }
        Ok(())
    }
}

fn archive_error(error: impl std::fmt::Display) -> String {
    let message = error.to_string();
    for code in [
        TOO_LARGE,
        "ENGINE_INSTALL_CANCELLED",
        "ENGINE_INSTALL_TIMEOUT",
    ] {
        if message.contains(code) {
            return code.into();
        }
    }
    ARCHIVE.into()
}

fn extract_zip(file: File, state: &mut Extraction<'_>) -> Result<(), String> {
    let mut zip = zip::ZipArchive::new(file).map_err(archive_error)?;
    if zip.len() > ENTRY_LIMIT {
        return Err(TOO_LARGE.into());
    }
    for index in 0..zip.len() {
        state.control.check()?;
        let mut entry = zip.by_index(index).map_err(archive_error)?;
        let kind = entry.unix_mode().unwrap_or(0) & 0o170000;
        if entry.is_symlink() || !matches!(kind, 0 | 0o100000 | 0o040000) {
            return Err(ARCHIVE.into());
        }
        let name = std::str::from_utf8(entry.name_raw())
            .map_err(|_| ARCHIVE)?
            .to_owned();
        let directory = entry.is_dir();
        if (kind == 0o040000 && !directory) || (kind == 0o100000 && directory) {
            return Err(ARCHIVE.into());
        }
        let size = entry.size();
        state.entry(&name, directory, size, &mut entry)?;
    }
    Ok(())
}

struct BoundedReader<'a, R> {
    inner: R,
    control: &'a InstallControl,
    total: u64,
}
impl<R: Read> Read for BoundedReader<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.control.check().map_err(io::Error::other)?;
        let count = self.inner.read(buffer)?;
        self.total += count as u64;
        if self.total > EXTRACTED_LIMIT {
            return Err(io::Error::other(TOO_LARGE));
        }
        Ok(count)
    }
}

fn extract_gzip(file: File, state: &mut Extraction<'_>) -> Result<(), String> {
    let mut reader = flate2::read::MultiGzDecoder::new(file);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o700);
    }
    let mut output = options.open(state.output.join("mihomo")).map_err(|_| IO)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; CHUNK];
    loop {
        state.control.check()?;
        let count = reader.read(&mut buffer).map_err(archive_error)?;
        if count == 0 {
            break;
        }
        state.total += count as u64;
        if state.total > EXTRACTED_LIMIT {
            return Err(TOO_LARGE.into());
        }
        output.write_all(&buffer[..count]).map_err(|_| IO)?;
        hasher.update(&buffer[..count]);
    }
    if state.total == 0 {
        return Err(ARCHIVE.into());
    }
    state.control.check()?;
    output.sync_all().map_err(|_| IO)?;
    state
        .files
        .insert("mihomo".into(), format!("{:x}", hasher.finalize()));
    Ok(())
}

fn extract_tar(file: File, state: &mut Extraction<'_>) -> Result<(), String> {
    let reader = BoundedReader {
        inner: flate2::read::GzDecoder::new(file),
        control: state.control,
        total: 0,
    };
    let mut tar = tar::Archive::new(reader);
    // Raw mode avoids allocating/processing attacker-controlled long-name or PAX metadata.
    for entry in tar.entries().map_err(archive_error)?.raw(true) {
        state.control.check()?;
        let mut entry = entry.map_err(archive_error)?;
        let kind = entry.header().entry_type();
        if !kind.is_file() && !kind.is_dir() {
            return Err(ARCHIVE.into());
        }
        let name = std::str::from_utf8(&entry.path_bytes())
            .map_err(|_| ARCHIVE)?
            .to_owned();
        let size = entry.size();
        state.entry(&name, kind.is_dir(), size, &mut entry)?;
    }
    // Consume padding and gzip trailer as well, enforcing the bound and CRC validation.
    let mut reader = tar.into_inner();
    let mut buffer = [0; CHUNK];
    while reader.read(&mut buffer).map_err(archive_error)? != 0 {}
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("engine-archive-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            fs::create_dir(path.join("out")).unwrap();
            Self(path)
        }
        fn zip(&self, entries: &[(&str, &[u8])]) -> std::path::PathBuf {
            let path = self.0.join("engine.zip");
            let mut writer = zip::ZipWriter::new(File::create(&path).unwrap());
            for (name, data) in entries {
                writer
                    .start_file(*name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                writer.write_all(data).unwrap();
            }
            writer.finish().unwrap();
            path
        }
        fn tar(&self, name: &str, link: bool) -> std::path::PathBuf {
            let path = self.0.join("engine.tar.gz");
            let encoder = flate2::write::GzEncoder::new(
                File::create(&path).unwrap(),
                flate2::Compression::fast(),
            );
            let mut writer = tar::Builder::new(encoder);
            let mut header = tar::Header::new_gnu();
            header.set_mode(0o755);
            header.set_size(if link { 0 } else { 6 });
            if link {
                header.set_entry_type(tar::EntryType::Symlink);
                header.set_link_name("outside").unwrap();
            }
            writer
                .append_data(
                    &mut header,
                    name,
                    if link { &b""[..] } else { &b"engine"[..] },
                )
                .unwrap();
            writer.into_inner().unwrap().finish().unwrap();
            path
        }
        fn prepare(&self, path: &Path) -> Result<BTreeMap<String, String>, String> {
            let control = InstallControl::for_test();
            let hash = file_sha256(path, &control).unwrap();
            prepare(
                path,
                &self.0.join("out"),
                path.file_name().unwrap().to_str().unwrap(),
                &hash,
                &control,
            )
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn official_zip_preserves_runtime_and_licenses() {
        let fixture = Fixture::new();
        let path = fixture.zip(&[
            ("package/mihomo.exe", b"exe"),
            ("package/libcronet.dll", b"dll"),
            ("package/LICENSE", b"license"),
            ("package/NOTICE.txt", b"notice"),
            ("package/README.md", b"skip"),
        ]);
        let files = fixture.prepare(&path).unwrap();
        assert_eq!(files.len(), 3);
        assert_eq!(files["mihomo.exe"], format!("{:x}", Sha256::digest(b"exe")));
        assert!(!fixture.0.join("out/README.md").exists());
    }

    #[test]
    fn official_tar_extracts_flat_and_private() {
        let fixture = Fixture::new();
        let path = fixture.tar("package/mihomo", false);
        assert!(fixture.prepare(&path).unwrap().contains_key("mihomo"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(fixture.0.join("out/mihomo"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
    }

    #[test]
    fn official_gzip_extracts_binary_and_checks_trailer() {
        for corrupt in [false, true] {
            let fixture = Fixture::new();
            let path = fixture.0.join("mihomo-darwin-arm64-v1.19.31.gz");
            let mut encoder =
                flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
            encoder.write_all(b"official-binary-fixture").unwrap();
            let mut bytes = encoder.finish().unwrap();
            if corrupt {
                let crc = bytes.len() - 8;
                bytes[crc] ^= 1;
            }
            fs::write(&path, bytes).unwrap();
            let result = fixture.prepare(&path);
            if corrupt {
                assert_eq!(result.unwrap_err(), ARCHIVE);
            } else {
                assert_eq!(
                    result.unwrap()["mihomo"],
                    format!("{:x}", Sha256::digest(b"official-binary-fixture"))
                );
                assert_eq!(
                    fs::read(fixture.0.join("out/mihomo")).unwrap(),
                    b"official-binary-fixture"
                );
            }
        }
    }

    #[test]
    fn official_windows_asset_name_is_normalized() {
        let fixture = Fixture::new();
        let path = fixture.zip(&[("mihomo-windows-amd64-v1.exe", b"exe")]);
        let control = InstallControl::for_test();
        let hash = file_sha256(&path, &control).unwrap();
        let files = prepare(
            &path,
            &fixture.0.join("out"),
            "mihomo-windows-amd64-v1-v1.19.31.zip",
            &hash,
            &control,
        )
        .unwrap();
        assert_eq!(files.len(), 1);
        assert!(files.contains_key("mihomo.exe"));
    }

    #[test]
    fn wrong_checksum_writes_nothing() {
        let fixture = Fixture::new();
        let path = fixture.zip(&[("mihomo.exe", b"exe")]);
        assert_eq!(
            prepare(
                &path,
                &fixture.0.join("out"),
                "engine.zip",
                &"0".repeat(64),
                &InstallControl::for_test()
            )
            .unwrap_err(),
            "ENGINE_INSTALL_CHECKSUM"
        );
        assert_eq!(fs::read_dir(fixture.0.join("out")).unwrap().count(), 0);
    }

    #[test]
    fn rejects_unsafe_paths_even_for_ignored_files() {
        for name in [
            "../ignored",
            "/ignored",
            "C:/ignored",
            "dir\\ignored",
            "dir/../ignored",
            "CON.txt",
        ] {
            let fixture = Fixture::new();
            let path = fixture.zip(&[(name, b"bad")]);
            assert_eq!(fixture.prepare(&path).unwrap_err(), ARCHIVE, "{name}");
        }
    }

    #[test]
    fn rejects_tar_symlinks() {
        let fixture = Fixture::new();
        let path = fixture.tar("mihomo", true);
        assert_eq!(fixture.prepare(&path).unwrap_err(), ARCHIVE);
    }

    #[test]
    fn windows_needs_only_the_engine_binary() {
        let fixture = Fixture::new();
        let path = fixture.zip(&[("mihomo.exe", b"exe")]);
        assert!(fixture.prepare(&path).unwrap().contains_key("mihomo.exe"));
    }

    #[test]
    fn rejects_flattened_duplicate_files() {
        let fixture = Fixture::new();
        let path = fixture.zip(&[("a/mihomo.exe", b"one"), ("b/mihomo.exe", b"two")]);
        assert_eq!(fixture.prepare(&path).unwrap_err(), ARCHIVE);
    }

    #[test]
    fn interrupted_install_writes_nothing() {
        let fixture = Fixture::new();
        let path = fixture.zip(&[("mihomo.exe", b"exe"), ("libcronet.dll", b"dll")]);
        let hash = file_sha256(&path, &InstallControl::for_test()).unwrap();
        for (control, error) in [
            (
                InstallControl::for_test_cancelled(),
                "ENGINE_INSTALL_CANCELLED",
            ),
            (
                InstallControl::for_test_timed_out(),
                "ENGINE_INSTALL_TIMEOUT",
            ),
        ] {
            assert_eq!(
                prepare(&path, &fixture.0.join("out"), "engine.zip", &hash, &control).unwrap_err(),
                error
            );
            assert_eq!(fs::read_dir(fixture.0.join("out")).unwrap().count(), 0);
        }
    }
}
