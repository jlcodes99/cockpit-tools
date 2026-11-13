use std::{
    fs,
    io::{ErrorKind, Read, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use super::{
    account, atomic_write, background_maintenance, secure_account_storage::LocalVaultCipher,
};

const FILE: &str = "apikey_fun_vault.json";
const CHECKPOINT: &str = "apikey_fun_vault.recovery.json";
const CHECKPOINT_REQUIRED: &str = "apikey_fun_vault_checkpoint_required";
const KIND: &str = "apikey-fun-vault";
const VERSION: u32 = 1;
const CHECKPOINT_BYTES: u64 = background_maintenance::BATCH_BYTES;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedApiKey {
    id: String,
    key: String,
    name: String,
    created_at: u64,
    last_used_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_remaining: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    added_to_codex_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    codex_provider_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    added_to_claude_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    claude_account_name: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct ApiKeyFunVault {
    version: u32,
    revision: u64,
    keys: Vec<ManagedApiKey>,
}

fn decode(cipher: &LocalVaultCipher, content: &str) -> Result<ApiKeyFunVault, String> {
    let snapshot: ApiKeyFunVault = cipher.unseal(KIND, content)?;
    if snapshot.version != VERSION {
        return Err("APIKEY.FUN 密钥库版本不受支持".into());
    }
    Ok(snapshot)
}

fn read_at(root: &Path) -> Result<Option<ApiKeyFunVault>, String> {
    let content = match fs::read_to_string(root.join(FILE)) {
        Ok(content) => content,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return if root
                .join(CHECKPOINT)
                .try_exists()
                .map_err(|error| error.to_string())?
            {
                Err(CHECKPOINT_REQUIRED.into())
            } else {
                Ok(None)
            };
        }
        Err(error) => return Err(format!("读取 APIKEY.FUN 密钥库失败: {error}")),
    };
    decode(&LocalVaultCipher::open(root, false)?, &content).map(Some)
}

fn lock_file(root: &Path, name: &str) -> Result<fs::File, String> {
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join(name))
        .map_err(|error| format!("打开 APIKEY.FUN 密钥库锁失败: {error}"))?;
    fs2::FileExt::try_lock_exclusive(&lock)
        .map_err(|_| "APIKEY.FUN 密钥库正在使用，请重试".to_string())?;
    Ok(lock)
}

fn read_checkpoint_content(path: &Path) -> Result<Option<String>, String> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("读取 APIKEY.FUN 恢复备份失败: {error}")),
    };
    let mut content = String::new();
    file.take(CHECKPOINT_BYTES + 1)
        .read_to_string(&mut content)
        .map_err(|error| format!("读取 APIKEY.FUN 恢复备份失败: {error}"))?;
    if content.len() as u64 > CHECKPOINT_BYTES {
        return Err("APIKEY.FUN 恢复备份超过处理预算".into());
    }
    Ok(Some(content))
}

fn checkpoint_at(root: &Path) -> Result<bool, String> {
    let _lock = lock_file(root, "apikey_fun_vault.recovery.lock")?;
    let Some(content) = read_checkpoint_content(&root.join(FILE))? else {
        return Ok(true);
    };
    let cipher = LocalVaultCipher::open(root, false)?;
    let live = decode(&cipher, &content)?;
    let path = root.join(CHECKPOINT);
    if let Ok(Some(previous)) = read_checkpoint_content(&path) {
        if previous == content {
            return Ok(true);
        }
        if decode(&cipher, &previous).is_ok_and(|snapshot| snapshot.revision > live.revision) {
            return Err("已有 APIKEY.FUN 恢复备份比当前密钥库更新".into());
        }
    }
    atomic_write::write_secret_string_atomic(&path, &content)?;
    // Check the live source again in the next maintenance batch. A save may
    // have completed while this snapshot was being written.
    Ok(false)
}

fn schedule_checkpoint(root: PathBuf) {
    let key = format!("apikey-fun-recovery:{}", root.display());
    background_maintenance::schedule(key, move || checkpoint_at(&root));
}

fn restore_at(root: &Path) -> Result<ApiKeyFunVault, String> {
    let _live_lock = lock_file(root, "apikey_fun_vault.lock")?;
    let path = root.join(FILE);
    if path.try_exists().map_err(|error| error.to_string())? {
        return Err("apikey_fun_vault_already_exists".into());
    }
    let _checkpoint_lock = lock_file(root, "apikey_fun_vault.recovery.lock")?;
    let content =
        read_checkpoint_content(&root.join(CHECKPOINT))?.ok_or("APIKEY.FUN 恢复备份缺失")?;
    let snapshot = decode(&LocalVaultCipher::open(root, false)?, &content)?;
    let mut temp = tempfile::NamedTempFile::new_in(root).map_err(|error| error.to_string())?;
    temp.write_all(content.as_bytes())
        .and_then(|_| temp.as_file().sync_all())
        .map_err(|error| error.to_string())?;
    temp.persist_noclobber(&path)
        .map_err(|error| error.error.to_string())?;
    Ok(snapshot)
}

fn write_at(
    root: &Path,
    expected_revision: Option<u64>,
    keys: Vec<ManagedApiKey>,
) -> Result<ApiKeyFunVault, String> {
    let _lock = lock_file(root, "apikey_fun_vault.lock")?;

    let current = read_at(root)?;
    let revision = match (current, expected_revision) {
        (Some(snapshot), None) => return Ok(snapshot),
        (Some(snapshot), Some(expected)) if snapshot.revision == expected => snapshot
            .revision
            .checked_add(1)
            .ok_or("APIKEY.FUN 密钥库版本溢出")?,
        (None, None) => 0,
        _ => return Err("APIKEY.FUN 密钥库已更新，请重试".into()),
    };
    let snapshot = ApiKeyFunVault {
        version: VERSION,
        revision,
        keys,
    };
    let cipher = LocalVaultCipher::open(root, revision == 0)?;
    let content = cipher.seal(KIND, &snapshot)?;
    atomic_write::write_secret_string_atomic(&root.join(FILE), &content)?;
    Ok(snapshot)
}

pub fn load() -> Result<Option<ApiKeyFunVault>, String> {
    read_at(&account::get_data_dir()?)
}

pub fn write(
    expected_revision: Option<u64>,
    keys: Vec<ManagedApiKey>,
) -> Result<ApiKeyFunVault, String> {
    let root = account::get_data_dir()?;
    let snapshot = write_at(&root, expected_revision, keys)?;
    schedule_checkpoint(root);
    Ok(snapshot)
}

pub fn schedule_recovery_checkpoint() -> Result<(), String> {
    schedule_checkpoint(account::get_data_dir()?);
    Ok(())
}

pub fn restore() -> Result<ApiKeyFunVault, String> {
    restore_at(&account::get_data_dir()?)
}

pub fn export_legacy(path: &Path) -> Result<(), String> {
    export_legacy_at(&account::get_data_dir()?, path)
}

fn export_legacy_at(root: &Path, path: &Path) -> Result<(), String> {
    let directory = path.parent().ok_or("无法定位导出目录")?;
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("无效的导出文件名")?;
    let protected = [
        FILE,
        CHECKPOINT,
        "secure-account-storage.key",
        "apikey_fun_vault.lock",
        "apikey_fun_vault.recovery.lock",
    ];
    if fs::canonicalize(directory).map_err(|error| error.to_string())?
        == fs::canonicalize(root).map_err(|error| error.to_string())?
        && protected.iter().any(|name| {
            filename
                .trim_end_matches([' ', '.'])
                .eq_ignore_ascii_case(name)
        })
    {
        return Err("APIKEY.FUN 导出不能覆盖密钥库、恢复备份或密钥文件".into());
    }
    let snapshot = read_at(root)?.ok_or("APIKEY.FUN 密钥库尚未初始化")?;
    let content = serde_json::to_string_pretty(&snapshot.keys)
        .map_err(|_| "序列化 APIKEY.FUN 旧版密钥列表失败")?;
    atomic_write::write_secret_string_atomic(path, &content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::secure_account_storage::local_vault_key_path;

    fn key() -> ManagedApiKey {
        ManagedApiKey {
            id: "synthetic-id".into(),
            key: "synthetic-credential".into(),
            name: "Synthetic".into(),
            created_at: 10,
            last_used_at: 15,
            last_status: None,
            last_remaining: None,
            added_to_codex_at: None,
            codex_provider_name: None,
            added_to_claude_at: None,
            claude_account_name: None,
        }
    }

    #[test]
    fn initialization_and_updates_preserve_authoritative_encrypted_state() {
        let dir = tempfile::tempdir().unwrap();
        let key = key();
        let root = dir.path();
        assert!(read_at(root).unwrap().is_none());
        assert_eq!(write_at(root, None, vec![key.clone()]).unwrap().revision, 0);
        let content = fs::read_to_string(root.join(FILE)).unwrap();
        assert!(!content.contains(&key.key));
        assert_eq!(read_at(root).unwrap().unwrap().keys[0].key, key.key);
        assert_eq!(write_at(root, Some(0), Vec::new()).unwrap().revision, 1);
        assert!(write_at(root, Some(0), vec![key.clone()]).is_err());
        assert!(write_at(root, None, vec![key]).unwrap().keys.is_empty());
        let key_path = local_vault_key_path(root);
        fs::remove_file(&key_path).unwrap();
        assert!(write_at(root, None, Vec::new()).is_err());
        assert!(!key_path.exists());
    }

    #[test]
    fn protected_checkpoint_preserves_an_authoritative_clear_when_live_file_is_lost() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let key = key();
        write_at(root, None, vec![key.clone()]).unwrap();
        assert!(!checkpoint_at(root).unwrap());
        let backup = fs::read_to_string(root.join(CHECKPOINT)).unwrap();
        assert!(!backup.contains(&key.key));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(root.join(CHECKPOINT))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        assert!(checkpoint_at(root).unwrap());
        write_at(root, Some(0), Vec::new()).unwrap();
        assert!(!checkpoint_at(root).unwrap());
        assert!(restore_at(root).is_err());
        assert!(read_at(root).unwrap().unwrap().keys.is_empty());
        fs::remove_file(root.join(FILE)).unwrap();
        assert_eq!(read_at(root).err().as_deref(), Some(CHECKPOINT_REQUIRED));
        assert_eq!(
            write_at(root, None, vec![key]).err().as_deref(),
            Some(CHECKPOINT_REQUIRED)
        );
        let restored = restore_at(root).unwrap();
        assert_eq!(restored.revision, 1);
        assert!(restored.keys.is_empty());
        assert!(checkpoint_at(root).unwrap());
    }

    #[test]
    fn corrupt_live_files_and_unverifiable_backups_are_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_at(root, None, vec![key()]).unwrap();
        checkpoint_at(root).unwrap();
        let checkpoint = fs::read(root.join(CHECKPOINT)).unwrap();
        fs::write(root.join(FILE), b"invalid live data").unwrap();
        assert!(checkpoint_at(root).is_err());
        assert!(restore_at(root).is_err());
        assert_eq!(fs::read(root.join(FILE)).unwrap(), b"invalid live data");
        assert_eq!(fs::read(root.join(CHECKPOINT)).unwrap(), checkpoint);
        fs::remove_file(root.join(FILE)).unwrap();
        fs::write(root.join(CHECKPOINT), b"invalid backup").unwrap();
        assert!(restore_at(root).is_err());
        assert!(!root.join(FILE).exists());
    }

    #[test]
    fn legacy_export_is_a_fresh_array_and_protects_the_encrypted_source() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let fixture = key();
        write_at(root, None, vec![fixture.clone()]).unwrap();
        checkpoint_at(root).unwrap();
        let path = root.join("export.json");
        export_legacy_at(root, &path).unwrap();
        let exported: Vec<ManagedApiKey> =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(exported[0].key, fixture.key);
        assert!(export_legacy_at(root, &root.join(FILE)).is_err());
        assert!(export_legacy_at(root, &root.join(CHECKPOINT)).is_err());
        assert!(export_legacy_at(root, &local_vault_key_path(root)).is_err());
        write_at(root, Some(0), Vec::new()).unwrap();
        export_legacy_at(root, &path).unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), "[]");
    }
}
