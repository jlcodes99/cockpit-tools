use chrono::{DateTime, Duration as ChronoDuration, Utc};
use reqwest_dav::types::list_cmd::ListEntity;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::modules::config::UserConfig;

#[derive(Debug, Clone)]
pub struct WebdavConnectionSettings {
    pub base_url: String,
    pub username: String,
    pub password: String,
    pub remote_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebdavBackupFileEntry {
    pub file_name: String,
    pub file_kind: String,
    pub size_bytes: u64,
    pub modified_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebdavTestResult {
    pub ok: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebdavUploadResult {
    pub uploaded_files: Vec<WebdavBackupFileEntry>,
    pub deleted_files: Vec<String>,
    pub uploaded_at: String,
    pub remote_dir: String,
}

pub struct WebdavSyncClient {
    pub client: reqwest_dav::Client,
    pub remote_dir: String,
}

impl WebdavSyncClient {
    pub fn new(settings: &WebdavConnectionSettings) -> Result<Self, String> {
        let auth = reqwest_dav::Auth::Basic(settings.username.clone(), settings.password.clone());
        let client = reqwest_dav::ClientBuilder::new()
            .set_host(settings.base_url.clone())
            .set_auth(auth)
            .build()
            .map_err(|err| format!("创建 WebDAV 客户端失败: {:?}", err))?;

        Ok(Self {
            client,
            remote_dir: settings.remote_dir.clone(),
        })
    }

    pub async fn check_dir_exists(&self, path: &str) -> bool {
        match self.client.list(path, reqwest_dav::Depth::Number(0)).await {
            Ok(_) => true,
            Err(reqwest_dav::Error::Decode(reqwest_dav::DecodeError::StatusMismatched(err))) => {
                err.response_code != 404
            }
            Err(_) => false,
        }
    }

    pub async fn ensure_remote_dir(&self) -> Result<(), String> {
        let mut current_dir = String::new();
        for part in self.remote_dir.split('/') {
            if part.is_empty() {
                continue;
            }
            if !current_dir.is_empty() {
                current_dir.push('/');
            }
            current_dir.push_str(part);
            if self.check_dir_exists(&current_dir).await {
                continue;
            }
            if let Err(err) = self.client.mkcol(&current_dir).await {
                match &err {
                    reqwest_dav::Error::Decode(reqwest_dav::DecodeError::StatusMismatched(
                        status_err,
                    )) => {
                        if status_err.response_code == 405 {
                            continue;
                        }
                    }
                    _ => {}
                }
                return Err(format!("创建 WebDAV 远端目录失败: {:?}", err));
            }
        }
        Ok(())
    }

    pub async fn list_remote_backups(&self) -> Result<Vec<WebdavBackupFileEntry>, String> {
        let mut files = Vec::new();

        if !self.check_dir_exists(&self.remote_dir).await {
            return Ok(files);
        }

        let entities = self
            .client
            .list(&self.remote_dir, reqwest_dav::Depth::Number(1))
            .await
            .map_err(|err| format!("读取 WebDAV 备份列表失败: {:?}", err))?;

        for entity in entities {
            match entity {
                ListEntity::File(file) => {
                    let Some(raw_name) = file.href.rsplit('/').find(|value| !value.is_empty())
                    else {
                        continue;
                    };
                    let file_name = match urlencoding::decode(raw_name) {
                        Ok(decoded) => decoded.to_string(),
                        Err(err) => {
                            tracing::warn!("跳过无法解码的 WebDAV 文件名 [{}]: {}", raw_name, err);
                            continue;
                        }
                    };

                    if !is_backup_file_name(&file_name) {
                        continue;
                    }

                    files.push(WebdavBackupFileEntry {
                        file_kind: file_kind(&file_name).to_string(),
                        file_name,
                        size_bytes: file.content_length as u64,
                        modified_at: Some(file.last_modified.to_rfc3339()),
                    });
                }
                ListEntity::Folder(_) => {}
            }
        }

        files.sort_by(|left, right| {
            modified_sort_key(right)
                .cmp(&modified_sort_key(left))
                .then_with(|| right.file_name.cmp(&left.file_name))
        });

        Ok(files)
    }

    pub async fn upload_backup_bytes(
        &self,
        file_name: &str,
        bytes: Vec<u8>,
    ) -> Result<WebdavBackupFileEntry, String> {
        if !is_backup_file_name(file_name) {
            return Err("WebDAV 只允许上传 Cockpit 备份文件".to_string());
        }
        self.ensure_remote_dir().await?;

        let path = format!("{}/{}", self.remote_dir, file_name);
        let size_bytes = bytes.len() as u64;

        self.client
            .put(&path, bytes)
            .await
            .map_err(|err| format!("上传 WebDAV 备份失败: {:?}", err))?;

        Ok(WebdavBackupFileEntry {
            file_name: file_name.to_string(),
            file_kind: file_kind(file_name).to_string(),
            size_bytes,
            modified_at: Some(Utc::now().to_rfc3339()),
        })
    }

    pub async fn read_remote_backup(&self, file_name: &str) -> Result<String, String> {
        if !is_backup_file_name(file_name) || !file_name.ends_with(".json") {
            return Err("只能从 WebDAV 恢复 JSON 备份文件".to_string());
        }
        let path = format!("{}/{}", self.remote_dir, file_name);
        let response = self
            .client
            .get(&path)
            .await
            .map_err(|err| format!("读取 WebDAV 备份失败: {:?}", err))?;

        response
            .text()
            .await
            .map_err(|err| format!("读取 WebDAV 备份内容失败: {}", err))
    }

    pub async fn read_remote_backup_bytes(&self, file_name: &str) -> Result<Vec<u8>, String> {
        if !is_backup_file_name(file_name) {
            return Err("WebDAV 只允许读取 Cockpit 备份文件".to_string());
        }
        let path = format!("{}/{}", self.remote_dir, file_name);
        let response = self
            .client
            .get(&path)
            .await
            .map_err(|err| format!("读取 WebDAV 备份失败: {:?}", err))?;

        let bytes = response
            .bytes()
            .await
            .map_err(|err| format!("读取 WebDAV 备份内容失败: {}", err))?;
        Ok(bytes.to_vec())
    }

    pub async fn delete_remote_backup(&self, file_name: &str) -> Result<(), String> {
        if !is_backup_file_name(file_name) {
            return Err("WebDAV 只允许删除 Cockpit 备份文件".to_string());
        }
        let path = format!("{}/{}", self.remote_dir, file_name);

        if let Err(err) = self.client.delete(&path).await {
            match &err {
                reqwest_dav::Error::Decode(reqwest_dav::DecodeError::StatusMismatched(
                    status_err,
                )) => {
                    if status_err.response_code == 404 {
                        return Ok(());
                    }
                }
                _ => {}
            }
            return Err(format!("删除 WebDAV 备份失败: {:?}", err));
        }
        Ok(())
    }

    pub async fn cleanup_remote_backups(&self, retention_days: i32) -> Result<Vec<String>, String> {
        let mut deleted = Vec::new();

        if !self.check_dir_exists(&self.remote_dir).await {
            return Ok(deleted);
        }

        let entities = self
            .client
            .list(&self.remote_dir, reqwest_dav::Depth::Number(1))
            .await
            .map_err(|err| format!("读取 WebDAV 备份列表失败: {:?}", err))?;

        let cutoff = Utc::now() - ChronoDuration::days(retention_days.max(1) as i64);

        for entity in entities {
            match entity {
                ListEntity::File(file) => {
                    let Some(raw_name) = file.href.rsplit('/').find(|value| !value.is_empty())
                    else {
                        continue;
                    };
                    let file_name = match urlencoding::decode(raw_name) {
                        Ok(decoded) => decoded.to_string(),
                        Err(err) => {
                            tracing::warn!("跳过无法解码的 WebDAV 文件名 [{}]: {}", raw_name, err);
                            continue;
                        }
                    };

                    if !is_backup_file_name(&file_name) {
                        continue;
                    }

                    if file.last_modified >= cutoff {
                        continue;
                    }

                    let path = format!("{}/{}", self.remote_dir, file_name);
                    if let Err(err) = self.client.delete(&path).await {
                        match &err {
                            reqwest_dav::Error::Decode(
                                reqwest_dav::DecodeError::StatusMismatched(status_err),
                            ) => {
                                if status_err.response_code == 404 {
                                    continue;
                                }
                            }
                            _ => {}
                        }
                        // 局部错误记录并继续，不中断整个清理流程
                        tracing::error!("删除过期 WebDAV 备份 [{}] 失败: {:?}", file_name, err);
                        continue;
                    }
                    deleted.push(file_name);
                }
                ListEntity::Folder(_) => {}
            }
        }

        deleted.sort();
        Ok(deleted)
    }
}
pub fn normalize_base_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("WebDAV 地址不能为空".to_string());
    }

    let mut url = Url::parse(trimmed).map_err(|err| format!("WebDAV 地址无效: {}", err))?;
    match url.scheme() {
        "http" | "https" => {}
        _ => return Err("WebDAV 地址必须以 http 或 https 开头".to_string()),
    }
    url.set_query(None);
    url.set_fragment(None);

    let mut value = url.to_string();
    if !value.ends_with('/') {
        value.push('/');
    }
    Ok(value)
}

pub fn normalize_remote_dir(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim().trim_matches('/');
    if trimmed.is_empty() {
        return Err("WebDAV 远端目录不能为空".to_string());
    }
    if trimmed.contains('\\') {
        return Err("WebDAV 远端目录不能包含反斜杠".to_string());
    }

    let mut parts = Vec::new();
    for part in trimmed.split('/') {
        let normalized = part.trim();
        if normalized.is_empty() {
            return Err("WebDAV 远端目录不能包含空路径段".to_string());
        }
        let decoded = urlencoding::decode(normalized)
            .map_err(|err| format!("WebDAV 远端目录编码无效: {}", err))?;
        if decoded == "." || decoded == ".." || decoded.contains('\\') {
            return Err("WebDAV 远端目录不能包含路径穿越片段".to_string());
        }
        parts.push(normalized.to_string());
    }

    Ok(parts.join("/"))
}

pub fn is_backup_file_name(file_name: &str) -> bool {
    let trimmed = file_name.trim();
    if trimmed != file_name || trimmed.contains('/') || trimmed.contains('\\') {
        return false;
    }
    let matches_prefix = trimmed.starts_with("cockpit_auto_backup_")
        || trimmed.starts_with("cockpit_manual_backup_");
    let matches_suffix = trimmed.ends_with(".json") || trimmed.ends_with(".zip");
    matches_prefix && matches_suffix
}

pub fn connection_from_config(config: &UserConfig) -> Result<WebdavConnectionSettings, String> {
    if !config.external_network_enabled {
        return Err("已关闭外连：WebDAV 同步不可用".to_string());
    }
    crate::modules::webdav_domain::validate_webdav_url(
        &config.webdav_sync_url,
        &config.webdav_allowed_domains,
    )?;
    connection_from_parts(
        &config.webdav_sync_url,
        &config.webdav_sync_username,
        &config.webdav_sync_password,
        &config.webdav_sync_remote_dir,
    )
}

pub fn connection_from_parts(
    base_url: &str,
    username: &str,
    password: &str,
    remote_dir: &str,
) -> Result<WebdavConnectionSettings, String> {
    let normalized_base_url = normalize_base_url(base_url)?;
    let normalized_remote_dir = normalize_remote_dir(remote_dir)?;
    let normalized_username = username.trim().to_string();
    if normalized_username.is_empty() {
        return Err("WebDAV 账号不能为空".to_string());
    }
    if password.is_empty() {
        return Err("WebDAV 应用密码不能为空".to_string());
    }

    Ok(WebdavConnectionSettings {
        base_url: normalized_base_url,
        username: normalized_username,
        password: password.to_string(),
        remote_dir: normalized_remote_dir,
    })
}
pub async fn test_connection(
    settings: &WebdavConnectionSettings,
) -> Result<WebdavTestResult, String> {
    let client = WebdavSyncClient::new(settings)?;
    client.ensure_remote_dir().await?;
    let _ = client
        .client
        .list(&settings.remote_dir, reqwest_dav::Depth::Number(1))
        .await
        .map_err(|err| format!("连接测试失败: {:?}", err))?;
    Ok(WebdavTestResult {
        ok: true,
        message: "WebDAV 连接成功".to_string(),
    })
}

pub async fn list_remote_backups(
    settings: &WebdavConnectionSettings,
) -> Result<Vec<WebdavBackupFileEntry>, String> {
    let client = WebdavSyncClient::new(settings)?;
    client.list_remote_backups().await
}

pub async fn read_remote_backup(
    settings: &WebdavConnectionSettings,
    file_name: &str,
) -> Result<String, String> {
    let client = WebdavSyncClient::new(settings)?;
    client.read_remote_backup(file_name).await
}

pub async fn read_remote_backup_bytes(
    settings: &WebdavConnectionSettings,
    file_name: &str,
) -> Result<Vec<u8>, String> {
    let client = WebdavSyncClient::new(settings)?;
    client.read_remote_backup_bytes(file_name).await
}

pub async fn delete_remote_backup(
    settings: &WebdavConnectionSettings,
    file_name: &str,
) -> Result<(), String> {
    let client = WebdavSyncClient::new(settings)?;
    client.delete_remote_backup(file_name).await
}

fn modified_sort_key(file: &WebdavBackupFileEntry) -> i64 {
    file.modified_at
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.timestamp())
        .unwrap_or_default()
}

fn file_kind(file_name: &str) -> &str {
    if file_name.ends_with(".zip") {
        "zip"
    } else {
        "json"
    }
}

#[cfg(test)]
mod tests {
    use super::{normalize_base_url, normalize_remote_dir};

    #[test]
    fn normalize_webdav_target_rejects_invalid_values() {
        assert!(normalize_base_url("").is_err());
        assert!(normalize_base_url("ftp://dav.example.com/dav/").is_err());
        assert!(normalize_remote_dir("../backups").is_err());
        assert!(normalize_remote_dir("CockpitTools\\backups").is_err());
    }

    #[test]
    fn normalize_webdav_target_trims_valid_values() {
        assert_eq!(
            normalize_base_url(" https://dav.jianguoyun.com/dav/ ").unwrap(),
            "https://dav.jianguoyun.com/dav/"
        );
        assert_eq!(
            normalize_remote_dir(" /cockpit-tools/ ").unwrap(),
            "cockpit-tools"
        );
    }
}
