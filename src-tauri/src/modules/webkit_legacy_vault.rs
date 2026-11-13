//! Read exact, app-owned localStorage keys from the former macOS dev origin.
//! The WebKit origin directory is SHA-256(origin + salt), encoded as base64url.
//! See WebKit's NetworkStorageManager and SQLiteStorageArea for the layout.
use serde::Deserialize;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LegacyVault {
    Saved,
    History,
    ApiKeyFun,
}

#[cfg(any(target_os = "macos", test))]
mod webkit {
    use std::{fs, io::ErrorKind, path::Path, time::Duration};

    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use rusqlite::{Connection, OpenFlags, OptionalExtension};
    use serde_json::Value;
    use sha2::{Digest, Sha256};

    use super::LegacyVault;

    const MAX_VALUE_BYTES: i64 = 10 * 1024 * 1024;

    fn keys(vault: LegacyVault) -> &'static [&'static str] {
        match vault {
            LegacyVault::Saved => &[
                "agtools.mfa.vault.v3",
                "agtools.mfa.vault.v2",
                "agtools.mfa.vault.v1",
                "agtools.two_factor_auth.saved.v2",
            ],
            LegacyVault::History => &[
                "agtools.2fa.query.history.v1",
                "agtools.two_factor_auth.history.v2",
            ],
            LegacyVault::ApiKeyFun => &["apikey_fun_managed_keys"],
        }
    }

    fn origin_directory(salt: &[u8]) -> String {
        let mut hash = Sha256::new();
        hash.update(b"http://localhost:1420");
        hash.update(salt);
        URL_SAFE_NO_PAD.encode(hash.finalize())
    }

    fn read_database(path: &Path, vault: LegacyVault) -> Result<Option<Vec<Value>>, String> {
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => return Err("旧 WebView 密钥库路径无效".into()),
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err("无法读取旧 WebView 密钥库".into()),
        }
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|_| "无法打开旧 WebView 密钥库".to_string())?;
        connection
            .busy_timeout(Duration::from_millis(150))
            .map_err(|_| "无法读取旧 WebView 密钥库".to_string())?;
        let table: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='ItemTable')",
            [], |row| row.get(0),
        ).map_err(|_| "无法读取旧 WebView 密钥库".to_string())?;
        if !table {
            return Ok(None);
        }

        let mut values = Vec::new();
        for key in keys(vault) {
            let found = connection.query_row(
                "SELECT length(value), CASE WHEN length(value) <= ?2 THEN CAST(value AS BLOB) END
                 FROM ItemTable WHERE key=?1",
                rusqlite::params![key, MAX_VALUE_BYTES],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<Vec<u8>>>(1)?)),
            ).optional().map_err(|_| "无法读取旧 WebView 密钥库".to_string())?;
            let Some((size, bytes)) = found else { continue };
            let bytes = bytes.ok_or("旧 WebView 密钥数据超过读取预算")?;
            if size % 2 != 0 {
                return Err("旧 WebView 密钥数据编码无效".into());
            }
            let units = bytes
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]));
            let text = char::decode_utf16(units)
                .collect::<Result<String, _>>()
                .map_err(|_| "旧 WebView 密钥数据编码无效".to_string())?;
            let records: Vec<Value> = serde_json::from_str(&text)
                .map_err(|_| "旧 WebView 密钥数据格式无效".to_string())?;
            if *key == "agtools.mfa.vault.v3" {
                return Ok(Some(records));
            }
            values.extend(records);
        }
        Ok((!values.is_empty()).then_some(values))
    }

    pub(super) fn read_at(root: &Path, vault: LegacyVault) -> Result<Option<Vec<Value>>, String> {
        for directory in [root.join("Default"), root.to_path_buf()] {
            use std::io::Read;
            let mut salt_file = match fs::File::open(directory.join("salt")) {
                Ok(file) => file,
                Err(error) if error.kind() == ErrorKind::NotFound => continue,
                Err(_) => return Err("无法读取旧 WebView 的存储标识".into()),
            };
            let mut salt = [0; 8];
            salt_file
                .read_exact(&mut salt)
                .map_err(|_| "无法读取旧 WebView 的存储标识".to_string())?;
            let origin = origin_directory(&salt);
            let database = directory
                .join(&origin)
                .join(&origin)
                .join("LocalStorage/localstorage.sqlite3");
            if let Some(records) = read_database(&database, vault)? {
                return Ok(Some(records));
            }
        }
        read_database(
            &root.join("LocalStorage/http_localhost_1420.localstorage"),
            vault,
        )
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn reads_only_known_keys_from_the_exact_historical_origin() {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path();
            let base = root.join("Default");
            let salt = b"fixture!";
            let origin = origin_directory(salt);
            let directory = base.join(&origin).join(origin).join("LocalStorage");
            fs::create_dir_all(&directory).unwrap();
            fs::write(base.join("salt"), salt).unwrap();
            let database = directory.join("localstorage.sqlite3");
            let conn = Connection::open(&database).unwrap();
            conn.execute_batch("CREATE TABLE ItemTable(key TEXT UNIQUE,value BLOB NOT NULL);")
                .unwrap();
            for (key, text) in [
                (
                    "agtools.mfa.vault.v2",
                    r#"[{"accountName":"测试","secret":"EXAMPLE"}]"#,
                ),
                ("unrelated", r#"["unrelated"]"#),
            ] {
                let bytes: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
                conn.execute(
                    "INSERT INTO ItemTable VALUES (?1,?2)",
                    rusqlite::params![key, bytes],
                )
                .unwrap();
            }
            drop(conn);
            let before = fs::read(&database).unwrap();
            let records = read_at(root, LegacyVault::Saved).unwrap().unwrap();
            assert_eq!(records[0]["accountName"], "测试");
            assert!(read_at(root, LegacyVault::History).unwrap().is_none());
            assert!(read_at(root, LegacyVault::ApiKeyFun).unwrap().is_none());
            assert_eq!(fs::read(database).unwrap(), before);
        }
    }
}

pub fn read(
    identifier: &str,
    vault: LegacyVault,
) -> Result<Option<Vec<serde_json::Value>>, String> {
    #[cfg(target_os = "macos")]
    if identifier == "com.jlcodes.cockpit-tools.dev" {
        let home = dirs::home_dir().ok_or("无法获取旧 WebView 的数据目录")?;
        let root = home
            .join("Library/WebKit")
            .join(identifier)
            .join("WebsiteData");
        return webkit::read_at(&root, vault);
    }
    let _ = (identifier, vault);
    Ok(None)
}
