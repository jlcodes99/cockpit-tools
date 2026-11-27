use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const MAX_ASAR_HEADER_BYTES: u32 = 4 * 1024 * 1024;
const MAX_PACKAGE_JSON_BYTES: u64 = 64 * 1024;

pub(super) fn desktop_version_from_asar(path: &Path) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    let mut prefix = [0; 16];
    file.read_exact(&mut prefix).ok()?;
    let word = |start| u32::from_le_bytes(prefix[start..start + 4].try_into().unwrap());
    // Electron ASAR stores two Chromium Pickles, followed by packed file data.
    let header_size = word(4);
    let header_payload_size = word(8);
    let json_size = word(12);
    let data_start = 8 + u64::from(header_size);
    if word(0) != 4
        || header_size > MAX_ASAR_HEADER_BYTES
        || header_payload_size.checked_add(4)? != header_size
        || json_size.checked_add(4)? > header_payload_size
        || data_start > length
    {
        return None;
    }

    let mut header = vec![0; json_size as usize];
    file.read_exact(&mut header).ok()?;
    let metadata: serde_json::Value = serde_json::from_slice(&header).ok()?;
    let entry = metadata.get("files")?.get("package.json")?;
    let size = entry.get("size")?.as_u64()?;
    let offset = entry.get("offset")?.as_str()?.parse::<u64>().ok()?;
    let start = data_start.checked_add(offset)?;
    if size > MAX_PACKAGE_JSON_BYTES || start.checked_add(size)? > length {
        return None;
    }

    file.seek(SeekFrom::Start(start)).ok()?;
    let mut package = vec![0; size as usize];
    file.read_exact(&mut package).ok()?;
    let package: serde_json::Value = serde_json::from_slice(&package).ok()?;
    let name = package
        .get("productName")
        .or_else(|| package.get("name"))?
        .as_str()?;
    if !name.eq_ignore_ascii_case("Antigravity") {
        return None;
    }

    let version = package.get("version")?.as_str()?.trim();
    (!version.is_empty()).then(|| version.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_archive(path: &Path, package: &str, offset: &str) {
        let header = serde_json::json!({
            "files": {"package.json": {"size": package.len(), "offset": offset}}
        })
        .to_string();
        let padded_len = (header.len() + 3) & !3;
        let mut bytes = [
            4,
            padded_len as u32 + 8,
            padded_len as u32 + 4,
            header.len() as u32,
        ]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect::<Vec<_>>();
        bytes.extend_from_slice(header.as_bytes());
        bytes.resize(16 + padded_len, 0);
        bytes.extend_from_slice(b"other");
        bytes.extend_from_slice(package.as_bytes());
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn reads_the_desktop_version_from_packed_package_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("app.asar");
        write_archive(
            &path,
            r#"{"name":"antigravity","productName":"Antigravity","version":"2.32.1"}"#,
            "5",
        );
        assert_eq!(desktop_version_from_asar(&path).as_deref(), Some("2.32.1"));
    }

    #[test]
    fn packed_desktop_version_wins_over_stale_unpacked_product_json() {
        let temp = tempfile::tempdir().unwrap();
        let resources = temp.path().join("resources");
        std::fs::create_dir_all(resources.join("app")).unwrap();
        // Leftover from an Antigravity 1.x install in the same directory.
        std::fs::write(
            resources.join("app").join("product.json"),
            r#"{"nameShort":"Antigravity","ideVersion":"1.23.2","version":"1.107.0"}"#,
        )
        .unwrap();
        write_archive(
            &resources.join("app.asar"),
            r#"{"name":"antigravity","productName":"Antigravity","version":"2.35.0"}"#,
            "5",
        );

        let desktop =
            super::super::read_antigravity_root_file_metadata(temp.path(), Some("antigravity"))
                .unwrap();
        assert_eq!(desktop.version, "2.35.0");
        assert_eq!(desktop.source, "app.asar/package.json");

        // The IDE target keeps reading its unpacked product.json.
        let ide = super::super::read_antigravity_root_file_metadata(
            temp.path(),
            Some("antigravity_ide"),
        )
        .unwrap();
        assert_eq!(ide.version, "1.23.2");
    }

    #[test]
    fn ignores_an_unrelated_product_and_out_of_bounds_entries() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("app.asar");
        write_archive(
            &path,
            r#"{"name":"antigravity","productName":"Antigravity IDE","version":"2.6.0"}"#,
            "5",
        );
        assert_eq!(desktop_version_from_asar(&path), None);
        write_archive(
            &path,
            r#"{"name":"antigravity","version":"2.32.1"}"#,
            "18446744073709551614",
        );
        assert_eq!(desktop_version_from_asar(&path), None);
    }
}
