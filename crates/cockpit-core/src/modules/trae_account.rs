use aes::Aes128;
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use cbc::cipher::block_padding::Pkcs7;
use cbc::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use rand::RngCore;
use reqwest::{Method, Url};
use ring::rand::SystemRandom;
use ring::signature::{EcdsaKeyPair, ECDSA_P256_SHA256_ASN1_SIGNING};
use serde_json::{Map, Value};
use sha2::{Digest, Sha512};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::models::trae::{TraeAccount, TraeAccountIndex, TraeImportPayload};
use crate::modules::{account, config, logger};

const ACCOUNTS_INDEX_FILE: &str = "trae_accounts.json";
const ACCOUNTS_DIR: &str = "trae_accounts";
const TRAE_DEFAULT_AUTH_PROVIDER_ID: &str = "icube.cloudide";
const TRAE_STORAGE_AUTH_KEY_PREFIX: &str = "iCubeAuthInfo://";
const TRAE_STORAGE_SERVER_KEY_PREFIX: &str = "iCubeServerData://";
const TRAE_STORAGE_ENTITLEMENT_KEY_PREFIX: &str = "iCubeEntitlementInfo://";
const TRAE_STORAGE_DEVICE_KEY_PREFIX: &str = "iCubeAuthInfo://icube-dc:";
const TRAE_STORAGE_AUTH_KEY: &str = "iCubeAuthInfo://icube.cloudide";
const TRAE_STORAGE_ENTITLEMENT_KEY: &str = "iCubeEntitlementInfo://icube.cloudide";
const TRAE_STORAGE_SERVER_KEY: &str = "iCubeServerData://icube.cloudide";
const TRAE_STORAGE_USERTAG_KEY: &str = "iCubeAuthInfo://usertag";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraePlatformKind {
    Trae,
    TraeSolo,
    TraeCn,
    TraeSoloCn,
}

impl TraePlatformKind {
    pub fn parse(raw: Option<&str>) -> Result<Self, String> {
        let normalized = raw
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("trae")
            .to_ascii_lowercase()
            .replace('-', "_");
        match normalized.as_str() {
            "trae" => Ok(Self::Trae),
            "trae_solo" => Ok(Self::TraeSolo),
            "trae_cn" => Ok(Self::TraeCn),
            "trae_solo_cn" => Ok(Self::TraeSoloCn),
            other => Err(format!("不支持的 Trae 平台: {}", other)),
        }
    }

    pub fn provider_key(self) -> &'static str {
        match self {
            Self::Trae => "trae",
            Self::TraeSolo => "trae_solo",
            Self::TraeCn => "trae_cn",
            Self::TraeSoloCn => "trae_solo_cn",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Trae => "Trae",
            Self::TraeSolo => "TRAE SOLO",
            Self::TraeCn => "Trae CN",
            Self::TraeSoloCn => "TRAE SOLO CN",
        }
    }

    pub fn is_cn(self) -> bool {
        matches!(self, Self::TraeCn | Self::TraeSoloCn)
    }

    pub fn is_solo(self) -> bool {
        matches!(self, Self::TraeSolo | Self::TraeSoloCn)
    }

    pub fn auth_client_id(self) -> &'static str {
        if self.is_solo() {
            TRAE_SOLO_AUTH_CLIENT_ID
        } else {
            TRAE_AUTH_CLIENT_ID
        }
    }

    pub fn auth_domain(self) -> &'static str {
        if self.is_cn() {
            TRAE_CN_AUTH_DOMAIN
        } else {
            TRAE_AUTH_DOMAIN
        }
    }

    pub fn default_login_host(self) -> String {
        format!("https://{}", self.auth_domain())
    }

    pub fn app_support_dir_name(self) -> &'static str {
        self.display_name()
    }

    #[cfg(target_os = "macos")]
    pub fn macos_app_name(self) -> &'static str {
        match self {
            Self::Trae => "Trae.app",
            Self::TraeSolo => "TRAE SOLO.app",
            Self::TraeCn => "Trae CN.app",
            Self::TraeSoloCn => "TRAE SOLO CN.app",
        }
    }
}

fn all_trae_platform_kinds() -> [TraePlatformKind; 4] {
    [
        TraePlatformKind::Trae,
        TraePlatformKind::TraeSolo,
        TraePlatformKind::TraeCn,
        TraePlatformKind::TraeSoloCn,
    ]
}

pub(crate) fn trae_configured_app_path(platform: TraePlatformKind) -> String {
    let current = config::get_user_config();
    match platform {
        TraePlatformKind::Trae => current.trae_app_path,
        TraePlatformKind::TraeSolo => current.trae_solo_app_path,
        TraePlatformKind::TraeCn => current.trae_cn_app_path,
        TraePlatformKind::TraeSoloCn => current.trae_solo_cn_app_path,
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn trae_configured_app_scan_roots(platform: TraePlatformKind) -> String {
    let current = config::get_user_config();
    match platform {
        TraePlatformKind::Trae => current.trae_app_scan_roots,
        TraePlatformKind::TraeSolo => current.trae_solo_app_scan_roots,
        TraePlatformKind::TraeCn => current.trae_cn_app_scan_roots,
        TraePlatformKind::TraeSoloCn => current.trae_solo_cn_app_scan_roots,
    }
}

const BYTE_CRYPTO_BLOCK_SIZE: usize = 16;
const BYTE_CRYPTO_HEADER_LEN: usize = 6;
const BYTE_CRYPTO_SHA512_LEN: usize = 64;
const BYTE_CRYPTO_RANDOM_KEY_LEN: usize = 32;
const BYTE_CRYPTO_PREFIX_AES: [u8; BYTE_CRYPTO_HEADER_LEN] = [116, 99, 5, 16, 0, 0];
const BYTE_CRYPTO_PREFIX_AES_PRIVATE: [u8; BYTE_CRYPTO_HEADER_LEN] = [18, 57, 32, 32, 2, 3];

const BYTE_CRYPTO_AES_PRIVATE_A: [u8; BYTE_CRYPTO_SHA512_LEN] = [
    191, 192, 216, 250, 122, 246, 220, 97, 31, 254, 98, 27, 8, 72, 71, 176, 135, 99, 96, 18, 127,
    101, 203, 104, 211, 102, 191, 125, 37, 72, 150, 156, 51, 229, 121, 35, 17, 153, 141, 177, 110,
    131, 150, 128, 172, 255, 254, 6, 18, 140, 55, 62, 236, 249, 135, 64, 135, 12, 117, 4, 89, 149,
    168, 209,
];
const BYTE_CRYPTO_AES_PRIVATE_B: [u8; BYTE_CRYPTO_SHA512_LEN] = [
    246, 204, 26, 232, 232, 70, 129, 109, 223, 146, 169, 242, 23, 241, 105, 145, 50, 196, 165, 42,
    254, 120, 3, 54, 244, 207, 209, 85, 53, 6, 138, 106, 175, 148, 31, 204, 186, 186, 165, 182, 87,
    142, 49, 10, 39, 110, 26, 154, 86, 56, 173, 125, 18, 64, 198, 225, 99, 99, 83, 82, 191, 134,
    76, 170,
];
const BYTE_CRYPTO_AES_A: [u8; BYTE_CRYPTO_SHA512_LEN] = [
    82, 9, 106, 213, 48, 54, 165, 56, 191, 64, 163, 158, 129, 243, 215, 251, 124, 227, 57, 130,
    155, 47, 255, 135, 52, 142, 67, 68, 196, 222, 233, 203, 84, 123, 148, 50, 166, 194, 35, 61,
    238, 76, 149, 11, 66, 250, 195, 78, 8, 46, 161, 102, 40, 217, 36, 178, 118, 91, 162, 73, 109,
    139, 209, 37,
];
const BYTE_CRYPTO_AES_B: [u8; BYTE_CRYPTO_SHA512_LEN] = [
    31, 221, 168, 51, 136, 7, 199, 49, 177, 18, 16, 89, 39, 128, 236, 95, 96, 81, 127, 169, 25,
    181, 74, 13, 45, 229, 122, 159, 147, 201, 156, 239, 160, 224, 59, 77, 174, 42, 245, 176, 200,
    235, 187, 60, 131, 83, 153, 97, 23, 43, 4, 126, 186, 119, 214, 38, 225, 105, 20, 99, 85, 33,
    12, 125,
];

type Aes128CbcEnc = cbc::Encryptor<Aes128>;
type Aes128CbcDec = cbc::Decryptor<Aes128>;

const TRAE_ACCOUNT_API_ORIGIN_NORMAL: &str = "https://grow-normal.trae.ai";
const TRAE_ACCOUNT_API_ORIGIN_SG: &str = "https://growsg-normal.trae.ai";
const TRAE_ACCOUNT_API_ORIGIN_US: &str = "https://growsg-normal.trae.ai";
const TRAE_ACCOUNT_API_ORIGIN_USTTP: &str = "https://grow-normal.traeapi.us";
const TRAE_ACCOUNT_API_ORIGIN_CN: &str = "https://api.trae.cn";
const TRAE_ACCOUNT_API_ORIGIN_CN_ICUBE: &str = "https://api.trae.com.cn";
const TRAE_EXCHANGE_TOKEN_PATH: &str = "/cloudide/api/v3/trae/oauth/ExchangeToken";
const TRAE_AUTH_CODE_EXCHANGE_TOKEN_PATH: &str = "/trae/api/v3/oauth/ExchangeToken";
const TRAE_GET_USER_INFO_PATH: &str = "/cloudide/api/v3/trae/GetUserInfo";
const TRAE_CHECK_LOGIN_PATH: &str = "/cloudide/api/v3/trae/CheckLogin";
const TRAE_PAY_STATUS_PATH: &str = "/trae/api/v1/pay/ide_user_pay_status";
const TRAE_ENT_USAGE_PATH: &str = "/trae/api/v1/pay/ide_user_ent_usage";
const TRAE_AUTH_DOMAIN: &str = "www.trae.ai";
const TRAE_CN_AUTH_DOMAIN: &str = "www.trae.cn";
const TRAE_AUTH_CLIENT_ID: &str = "ono9krqynydwx5";
const TRAE_SOLO_AUTH_CLIENT_ID: &str = "en1oxy7wnw8j9n";
const TRAE_EXCHANGE_CLIENT_SECRET: &str = "-";
const TRAE_IDE_VERSION: &str = "3.5.66";

lazy_static::lazy_static! {
    static ref TRAE_ACCOUNT_INDEX_LOCK: Mutex<()> = Mutex::new(());
}

#[derive(Clone, Debug)]
struct TraeRefreshRoutingContext {
    platform: TraePlatformKind,
    client_id: String,
    login_host: String,
    login_region: Option<String>,
    store_region: Option<String>,
    ai_region: Option<String>,
}

fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

fn normalize_non_empty(value: Option<&str>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn normalize_email(value: Option<&str>) -> Option<String> {
    normalize_non_empty(value).and_then(|raw| {
        if raw.contains('@') {
            Some(raw.to_lowercase())
        } else {
            None
        }
    })
}

fn normalize_identity_email(value: Option<&str>) -> Option<String> {
    normalize_email(value).and_then(|email| {
        if email == "unknown" {
            None
        } else {
            Some(email)
        }
    })
}

fn account_matches_import_identity(
    account: &TraeAccount,
    normalized_user_id: Option<&str>,
    normalized_email: Option<&str>,
) -> bool {
    let existing_user_id = normalize_non_empty(account.user_id.as_deref());

    if let (Some(left), Some(right)) = (existing_user_id.as_deref(), normalized_user_id) {
        return left == right;
    }

    if normalized_user_id.is_some() && existing_user_id.is_some() {
        return false;
    }

    matches!(
        (
            normalize_identity_email(Some(account.email.as_str())).as_deref(),
            normalized_email
        ),
        (Some(left), Some(right)) if left == right
    )
}

fn normalize_timestamp(raw: Option<i64>) -> Option<i64> {
    let value = raw?;
    if value <= 0 {
        return None;
    }
    if value > 10_000_000_000 {
        return Some(value / 1000);
    }
    Some(value)
}

fn ensure_https_url(raw: &str) -> Result<Url, String> {
    let normalized = normalize_non_empty(Some(raw)).ok_or_else(|| "Trae 域名为空".to_string())?;
    let with_scheme = if normalized.starts_with("http://") || normalized.starts_with("https://") {
        normalized
    } else {
        format!("https://{}", normalized.trim_start_matches('/'))
    };
    Url::parse(with_scheme.as_str()).map_err(|e| format!("解析 Trae 域名失败: {}", e))
}

fn normalize_origin(raw: &str) -> Option<String> {
    let url = ensure_https_url(raw).ok()?;
    let host = url.host_str()?;
    Some(format!("{}://{}", url.scheme(), host))
}

fn is_official_trae_account_api_origin(origin: &str) -> bool {
    let normalized = origin.trim_end_matches('/');
    [
        TRAE_ACCOUNT_API_ORIGIN_NORMAL,
        TRAE_ACCOUNT_API_ORIGIN_SG,
        TRAE_ACCOUNT_API_ORIGIN_US,
        TRAE_ACCOUNT_API_ORIGIN_USTTP,
        TRAE_ACCOUNT_API_ORIGIN_CN,
        TRAE_ACCOUNT_API_ORIGIN_CN_ICUBE,
    ]
    .iter()
    .any(|candidate| normalized == *candidate)
}

fn official_trae_account_api_origin_for_region(
    platform: TraePlatformKind,
    store_region: Option<&str>,
    ai_region: Option<&str>,
    login_region: Option<&str>,
) -> String {
    if platform.is_cn() {
        return TRAE_ACCOUNT_API_ORIGIN_CN.to_string();
    }

    let normalized_region = store_region
        .or(ai_region)
        .map(|value| to_store_region(value))
        .or_else(|| {
            login_region.map(|value| match value.trim().to_ascii_lowercase().as_str() {
                "sg" => "SG".to_string(),
                "us" => "US".to_string(),
                "usttp" => "USTTP".to_string(),
                _ => "CN".to_string(),
            })
        })
        .unwrap_or_else(|| "CN".to_string());

    match normalized_region.as_str() {
        "SG" => TRAE_ACCOUNT_API_ORIGIN_SG.to_string(),
        "US" => TRAE_ACCOUNT_API_ORIGIN_US.to_string(),
        "USTTP" => TRAE_ACCOUNT_API_ORIGIN_USTTP.to_string(),
        _ => TRAE_ACCOUNT_API_ORIGIN_NORMAL.to_string(),
    }
}

fn resolve_trae_account_api_origin(
    platform: TraePlatformKind,
    host: Option<&str>,
    store_region: Option<&str>,
    ai_region: Option<&str>,
    login_region: Option<&str>,
) -> String {
    if let Some(origin) = host.and_then(normalize_origin) {
        if is_official_trae_account_api_origin(origin.as_str()) {
            return origin;
        }
    }

    official_trae_account_api_origin_for_region(platform, store_region, ai_region, login_region)
}

fn resolve_trae_auth_storage_origin(
    platform: TraePlatformKind,
    host: Option<&str>,
    store_region: Option<&str>,
    ai_region: Option<&str>,
    login_region: Option<&str>,
) -> String {
    host.and_then(normalize_origin).unwrap_or_else(|| {
        official_trae_account_api_origin_for_region(platform, store_region, ai_region, login_region)
    })
}

fn build_api_urls(origin: &str, path: &str) -> Vec<String> {
    vec![format!("{}{}", origin.trim_end_matches('/'), path)]
}

fn get_data_dir() -> Result<PathBuf, String> {
    account::get_data_dir()
}

fn get_accounts_dir() -> Result<PathBuf, String> {
    let base = get_data_dir()?;
    let dir = base.join(ACCOUNTS_DIR);
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|e| format!("创建 Trae 账号目录失败: {}", e))?;
    }
    Ok(dir)
}

fn get_accounts_index_path() -> Result<PathBuf, String> {
    Ok(get_data_dir()?.join(ACCOUNTS_INDEX_FILE))
}

pub fn accounts_index_path_string() -> Result<String, String> {
    Ok(get_accounts_index_path()?.to_string_lossy().to_string())
}

fn normalize_account_id(account_id: &str) -> Result<String, String> {
    let trimmed = account_id.trim();
    if trimmed.is_empty() {
        return Err("账号 ID 不能为空".to_string());
    }

    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains("..") {
        return Err("账号 ID 非法，包含路径字符".to_string());
    }

    let valid = trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.');
    if !valid {
        return Err("账号 ID 非法，仅允许字母/数字/._-".to_string());
    }

    Ok(trimmed.to_string())
}

fn resolve_account_file_path(account_id: &str) -> Result<PathBuf, String> {
    let normalized = normalize_account_id(account_id)?;
    Ok(get_accounts_dir()?.join(format!("{}.json", normalized)))
}

pub fn load_account(account_id: &str) -> Option<TraeAccount> {
    let account_path = resolve_account_file_path(account_id).ok()?;
    if !account_path.exists() {
        return None;
    }
    let content = fs::read_to_string(&account_path).ok()?;
    crate::modules::atomic_write::parse_json_with_auto_restore(&account_path, &content).ok()
}

fn save_account_file(account: &TraeAccount) -> Result<(), String> {
    let path = resolve_account_file_path(account.id.as_str())?;
    let content = serde_json::to_string_pretty(account)
        .map_err(|e| format!("序列化 Trae 账号失败: {}", e))?;
    crate::modules::atomic_write::write_string_atomic(&path, &content)
        .map_err(|e| format!("保存 Trae 账号失败: {}", e))
}

fn delete_account_file(account_id: &str) -> Result<(), String> {
    let path = resolve_account_file_path(account_id)?;
    if path.exists() {
        fs::remove_file(path).map_err(|e| format!("删除 Trae 账号文件失败: {}", e))?;
    }
    Ok(())
}

fn load_account_index() -> TraeAccountIndex {
    let path = match get_accounts_index_path() {
        Ok(path) => path,
        Err(_) => return TraeAccountIndex::new(),
    };

    if !path.exists() {
        return repair_account_index_from_details("索引文件不存在")
            .unwrap_or_else(TraeAccountIndex::new);
    }

    match fs::read_to_string(&path) {
        Ok(content) if content.trim().is_empty() => {
            repair_account_index_from_details("索引文件为空").unwrap_or_else(TraeAccountIndex::new)
        }
        Ok(content) => match crate::modules::atomic_write::parse_json_with_auto_restore::<
            TraeAccountIndex,
        >(&path, &content)
        {
            Ok(index) if !index.accounts.is_empty() => index,
            Ok(_) => repair_account_index_from_details("索引账号列表为空")
                .unwrap_or_else(TraeAccountIndex::new),
            Err(err) => {
                logger::log_warn(&format!(
                    "[Trae Account] 账号索引解析失败，尝试按详情文件自动修复: path={}, error={}",
                    path.display(),
                    err
                ));
                repair_account_index_from_details("索引文件损坏")
                    .unwrap_or_else(TraeAccountIndex::new)
            }
        },
        Err(_) => TraeAccountIndex::new(),
    }
}

fn load_account_index_checked() -> Result<TraeAccountIndex, String> {
    let path = get_accounts_index_path()?;
    if !path.exists() {
        if let Some(index) = repair_account_index_from_details("索引文件不存在") {
            return Ok(index);
        }
        return Ok(TraeAccountIndex::new());
    }

    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(err) => {
            if let Some(index) = repair_account_index_from_details("索引文件读取失败") {
                return Ok(index);
            }
            return Err(format!("读取账号索引失败: {}", err));
        }
    };

    if content.trim().is_empty() {
        if let Some(index) = repair_account_index_from_details("索引文件为空") {
            return Ok(index);
        }
        return Ok(TraeAccountIndex::new());
    }

    match crate::modules::atomic_write::parse_json_with_auto_restore::<TraeAccountIndex>(
        &path, &content,
    ) {
        Ok(index) if !index.accounts.is_empty() => Ok(index),
        Ok(index) => {
            if let Some(repaired) = repair_account_index_from_details("索引账号列表为空") {
                return Ok(repaired);
            }
            Ok(index)
        }
        Err(err) => {
            if let Some(index) = repair_account_index_from_details("索引文件损坏") {
                return Ok(index);
            }
            Err(crate::error::file_corrupted_error(
                ACCOUNTS_INDEX_FILE,
                &path.to_string_lossy(),
                &err.to_string(),
            ))
        }
    }
}

fn save_account_index(index: &TraeAccountIndex) -> Result<(), String> {
    let path = get_accounts_index_path()?;
    let content = serde_json::to_string_pretty(index)
        .map_err(|e| format!("序列化 Trae 账号索引失败: {}", e))?;
    crate::modules::atomic_write::write_string_atomic(&path, &content)
        .map_err(|e| format!("写入 Trae 账号索引失败: {}", e))
}

fn repair_account_index_from_details(reason: &str) -> Option<TraeAccountIndex> {
    let index_path = get_accounts_index_path().ok()?;
    let accounts_dir = get_accounts_dir().ok()?;
    let mut accounts = crate::modules::account_index_repair::load_accounts_from_details(
        &accounts_dir,
        |account_id| load_account(account_id),
    )
    .ok()?;

    if accounts.is_empty() {
        return None;
    }

    crate::modules::account_index_repair::sort_accounts_by_recency(
        &mut accounts,
        |account| account.last_used,
        |account| account.created_at,
        |account| account.id.as_str(),
    );

    let mut index = TraeAccountIndex::new();
    index.accounts = accounts.iter().map(|account| account.summary()).collect();

    let backup_path = crate::modules::account_index_repair::backup_existing_index(&index_path)
        .unwrap_or_else(|err| {
            logger::log_warn(&format!(
                "[Trae Account] 自动修复前备份索引失败，继续尝试重建: path={}, error={}",
                index_path.display(),
                err
            ));
            None
        });

    if let Err(err) = save_account_index(&index) {
        logger::log_warn(&format!(
            "[Trae Account] 自动修复索引保存失败，将以内存结果继续运行: reason={}, recovered_accounts={}, error={}",
            reason,
            index.accounts.len(),
            err
        ));
    }

    logger::log_warn(&format!(
        "[Trae Account] 检测到账号索引异常，已根据详情文件自动重建: reason={}, recovered_accounts={}, backup_path={}",
        reason,
        index.accounts.len(),
        backup_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "-".to_string())
    ));

    Some(index)
}

fn refresh_summary(index: &mut TraeAccountIndex, account: &TraeAccount) {
    if let Some(summary) = index.accounts.iter_mut().find(|item| item.id == account.id) {
        *summary = account.summary();
        return;
    }
    index.accounts.push(account.summary());
}

fn upsert_account_record(account: TraeAccount) -> Result<TraeAccount, String> {
    let _lock = TRAE_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 Trae 账号锁失败".to_string())?;
    let mut index = load_account_index();
    save_account_file(&account)?;
    refresh_summary(&mut index, &account);
    save_account_index(&index)?;
    Ok(account)
}

fn persist_quota_query_error(account_id: &str, message: &str) {
    let Some(mut account) = load_account(account_id) else {
        return;
    };
    account.quota_query_last_error = Some(message.to_string());
    account.quota_query_last_error_at = Some(chrono::Utc::now().timestamp_millis());
    let _ = upsert_account_record(account);
}

fn extract_json_value(root: Option<&Value>, path: &[&str]) -> Option<Value> {
    let mut current = root?;
    for key in path {
        current = current.as_object()?.get(*key)?;
    }
    Some(current.clone())
}

fn pick_string(root: Option<&Value>, paths: &[&[&str]]) -> Option<String> {
    for path in paths {
        if let Some(value) = extract_json_value(root, path) {
            if let Some(text) = value.as_str() {
                if let Some(normalized) = normalize_non_empty(Some(text)) {
                    return Some(normalized);
                }
            }
            if let Some(num) = value.as_i64() {
                return Some(num.to_string());
            }
            if let Some(num) = value.as_u64() {
                return Some(num.to_string());
            }
        }
    }
    None
}

fn pick_i64(root: Option<&Value>, paths: &[&[&str]]) -> Option<i64> {
    for path in paths {
        if let Some(value) = extract_json_value(root, path) {
            if let Some(num) = value.as_i64() {
                return Some(num);
            }
            if let Some(num) = value.as_u64() {
                if num <= i64::MAX as u64 {
                    return Some(num as i64);
                }
            }
            if let Some(text) = value.as_str() {
                let trimmed = text.trim();
                if let Ok(parsed) = trimmed.parse::<i64>() {
                    return Some(parsed);
                }
                if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(trimmed) {
                    return Some(parsed.timestamp());
                }
            }
        }
    }
    None
}

fn parse_value_or_json_string(value: Option<&Value>) -> Option<Value> {
    let value = value?;
    if value.is_object() || value.is_array() {
        return Some(value.clone());
    }
    if let Some(text) = value.as_str() {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return None;
        }
        if let Ok(parsed) = serde_json::from_str::<Value>(trimmed) {
            return Some(parsed);
        }
    }
    None
}

fn parse_value_or_json_string_or_icube_cipher(value: Option<&Value>) -> Option<Value> {
    let value = value?;
    if value.is_object() || value.is_array() {
        return Some(value.clone());
    }
    let text = value.as_str()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(parsed) = serde_json::from_str::<Value>(trimmed) {
        return Some(parsed);
    }
    let decoded = BASE64_STANDARD.decode(trimmed.as_bytes()).ok()?;
    let decrypted = byte_crypto_decrypt(&decoded)?;
    let decrypted_text = String::from_utf8(decrypted).ok()?;
    serde_json::from_str::<Value>(decrypted_text.as_str()).ok()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ByteCryptoVersion {
    Aes,
    AesPrivate,
    Unknown,
}

fn sha512_bytes(data: &[u8]) -> [u8; BYTE_CRYPTO_SHA512_LEN] {
    let mut hasher = Sha512::new();
    hasher.update(data);
    let digest = hasher.finalize();
    let mut out = [0u8; BYTE_CRYPTO_SHA512_LEN];
    out.copy_from_slice(digest.as_slice());
    out
}

fn byte_crypto_version_from_header(header: &[u8]) -> ByteCryptoVersion {
    if header == BYTE_CRYPTO_PREFIX_AES {
        return ByteCryptoVersion::Aes;
    }
    if header == BYTE_CRYPTO_PREFIX_AES_PRIVATE {
        return ByteCryptoVersion::AesPrivate;
    }
    ByteCryptoVersion::Unknown
}

fn byte_crypto_salt(version: ByteCryptoVersion) -> [u8; BYTE_CRYPTO_SHA512_LEN] {
    let mut salt = [0u8; BYTE_CRYPTO_SHA512_LEN];
    let (left, right) = match version {
        ByteCryptoVersion::AesPrivate => (BYTE_CRYPTO_AES_PRIVATE_A, BYTE_CRYPTO_AES_PRIVATE_B),
        _ => (BYTE_CRYPTO_AES_A, BYTE_CRYPTO_AES_B),
    };
    for idx in 0..BYTE_CRYPTO_SHA512_LEN {
        salt[idx] = left[idx] ^ right[idx];
    }
    salt
}

fn byte_crypto_derive_key_iv(
    key_material: &[u8],
    version: ByteCryptoVersion,
) -> Option<([u8; 16], [u8; 16])> {
    if key_material.len() != BYTE_CRYPTO_RANDOM_KEY_LEN {
        return None;
    }

    let mut merge = [0u8; BYTE_CRYPTO_SHA512_LEN * 2];
    let key_hash = sha512_bytes(key_material);
    merge[..BYTE_CRYPTO_SHA512_LEN].copy_from_slice(&key_hash);
    merge[BYTE_CRYPTO_SHA512_LEN..].copy_from_slice(&byte_crypto_salt(version));

    let merged_hash = sha512_bytes(&merge);
    let mut aes_key = [0u8; 16];
    let mut iv = [0u8; 16];
    aes_key.copy_from_slice(&merged_hash[..16]);
    iv.copy_from_slice(&merged_hash[16..32]);
    Some((aes_key, iv))
}

fn byte_crypto_encrypt_v1(plaintext: &[u8]) -> Result<Vec<u8>, String> {
    let mut random_key = [0u8; BYTE_CRYPTO_RANDOM_KEY_LEN];
    rand::rngs::OsRng.fill_bytes(&mut random_key);

    let (aes_key, iv) = byte_crypto_derive_key_iv(&random_key, ByteCryptoVersion::Aes)
        .ok_or_else(|| "生成 Trae usertag 密钥失败".to_string())?;

    let mut payload = Vec::with_capacity(BYTE_CRYPTO_SHA512_LEN + plaintext.len());
    payload.extend_from_slice(&sha512_bytes(plaintext));
    payload.extend_from_slice(plaintext);

    let mut padded = payload;
    let msg_len = padded.len();
    let pad_len = BYTE_CRYPTO_BLOCK_SIZE - (msg_len % BYTE_CRYPTO_BLOCK_SIZE);
    padded.resize(msg_len + pad_len, 0);
    let cipher = Aes128CbcEnc::new_from_slices(&aes_key, &iv)
        .map_err(|e| format!("初始化 Trae usertag 加密器失败: {}", e))?;
    let encrypted = cipher
        .encrypt_padded_mut::<Pkcs7>(&mut padded, msg_len)
        .map_err(|e| format!("加密 Trae usertag 失败: {}", e))?
        .to_vec();

    let mut out =
        Vec::with_capacity(BYTE_CRYPTO_HEADER_LEN + BYTE_CRYPTO_RANDOM_KEY_LEN + encrypted.len());
    out.extend_from_slice(&BYTE_CRYPTO_PREFIX_AES);
    out.extend_from_slice(&random_key);
    out.extend_from_slice(&encrypted);
    Ok(out)
}

fn byte_crypto_decrypt(raw: &[u8]) -> Option<Vec<u8>> {
    if raw.len() <= BYTE_CRYPTO_HEADER_LEN + BYTE_CRYPTO_RANDOM_KEY_LEN {
        return None;
    }

    let version = byte_crypto_version_from_header(&raw[..BYTE_CRYPTO_HEADER_LEN]);
    if version == ByteCryptoVersion::Unknown {
        return None;
    }

    let key_start = BYTE_CRYPTO_HEADER_LEN;
    let key_end = key_start + BYTE_CRYPTO_RANDOM_KEY_LEN;
    let key_material = &raw[key_start..key_end];
    let ciphertext = &raw[key_end..];
    if ciphertext.is_empty() || ciphertext.len() % BYTE_CRYPTO_BLOCK_SIZE != 0 {
        return None;
    }

    let (aes_key, iv) = byte_crypto_derive_key_iv(key_material, version)?;
    let mut buffer = ciphertext.to_vec();
    let cipher = Aes128CbcDec::new_from_slices(&aes_key, &iv).ok()?;
    let decrypted = cipher
        .decrypt_padded_mut::<Pkcs7>(&mut buffer)
        .ok()?
        .to_vec();
    if decrypted.len() < BYTE_CRYPTO_SHA512_LEN {
        return None;
    }

    let digest = sha512_bytes(&decrypted[BYTE_CRYPTO_SHA512_LEN..]);
    if digest.as_slice() != &decrypted[..BYTE_CRYPTO_SHA512_LEN] {
        return None;
    }

    Some(decrypted[BYTE_CRYPTO_SHA512_LEN..].to_vec())
}

fn parse_usertag_map_from_json(text: &str) -> Option<BTreeMap<String, String>> {
    let value = serde_json::from_str::<Value>(text).ok()?;
    let obj = value.as_object()?;
    let mut map = BTreeMap::new();
    for (key, value) in obj {
        let user_id = normalize_non_empty(Some(key.as_str()))?;
        let usertag = value
            .as_str()
            .and_then(|item| normalize_non_empty(Some(item)))
            .map(|item| item.to_ascii_lowercase())?;
        map.insert(user_id, usertag);
    }
    Some(map)
}

fn decode_usertag_map(raw: &str) -> Option<BTreeMap<String, String>> {
    let text = normalize_non_empty(Some(raw))?;
    if let Some(map) = parse_usertag_map_from_json(text.as_str()) {
        return Some(map);
    }

    let decoded = BASE64_STANDARD.decode(text.as_bytes()).ok()?;
    let decrypted = byte_crypto_decrypt(&decoded)?;
    let decrypted_text = String::from_utf8(decrypted).ok()?;
    parse_usertag_map_from_json(decrypted_text.as_str())
}

fn encode_usertag_map(map: &BTreeMap<String, String>) -> Result<String, String> {
    let payload =
        serde_json::to_string(map).map_err(|e| format!("序列化 usertag 映射失败: {}", e))?;
    let encrypted = byte_crypto_encrypt_v1(payload.as_bytes())?;
    Ok(BASE64_STANDARD.encode(encrypted))
}

fn normalize_usertag_value(raw: Option<&str>) -> Option<String> {
    normalize_non_empty(raw).map(|value| value.to_ascii_lowercase())
}

fn find_storage_key_by_prefix(
    root_obj: &Map<String, Value>,
    prefix: &str,
    exclude_key: Option<&str>,
) -> Option<String> {
    for key in root_obj.keys() {
        if !key.starts_with(prefix) {
            continue;
        }
        if let Some(exclude) = exclude_key {
            if key == exclude {
                continue;
            }
        }
        return Some(key.clone());
    }
    None
}

fn provider_id_from_storage_key(key: &str, prefix: &str) -> Option<String> {
    key.strip_prefix(prefix)
        .and_then(|suffix| normalize_non_empty(Some(suffix)))
}

fn is_trae_device_key_storage_key(key: &str) -> bool {
    key.starts_with(TRAE_STORAGE_DEVICE_KEY_PREFIX)
}

fn is_trae_device_provider_id(provider_id: &str) -> bool {
    provider_id.trim().starts_with("icube-dc:")
}

fn is_trae_user_auth_storage_key(key: &str) -> bool {
    key.starts_with(TRAE_STORAGE_AUTH_KEY_PREFIX)
        && key != TRAE_STORAGE_USERTAG_KEY
        && !is_trae_device_key_storage_key(key)
}

fn find_user_auth_storage_key(root_obj: &Map<String, Value>) -> Option<String> {
    for key in root_obj.keys() {
        if is_trae_user_auth_storage_key(key) {
            return Some(key.clone());
        }
    }
    None
}

fn build_auth_storage_key(provider_id: &str) -> String {
    format!("{}{}", TRAE_STORAGE_AUTH_KEY_PREFIX, provider_id)
}

fn build_device_key_storage_key(device_id: &str) -> String {
    format!("{}{}", TRAE_STORAGE_DEVICE_KEY_PREFIX, device_id)
}

fn build_server_storage_key(provider_id: &str) -> String {
    format!("{}{}", TRAE_STORAGE_SERVER_KEY_PREFIX, provider_id)
}

fn build_entitlement_storage_key(provider_id: &str) -> String {
    format!("{}{}", TRAE_STORAGE_ENTITLEMENT_KEY_PREFIX, provider_id)
}

fn resolve_storage_provider_id(root_obj: &Map<String, Value>) -> String {
    if let Some(key) = find_user_auth_storage_key(root_obj) {
        if let Some(provider) = provider_id_from_storage_key(&key, TRAE_STORAGE_AUTH_KEY_PREFIX) {
            return provider;
        }
    }
    if let Some(key) = find_storage_key_by_prefix(root_obj, TRAE_STORAGE_SERVER_KEY_PREFIX, None) {
        if let Some(provider) = provider_id_from_storage_key(&key, TRAE_STORAGE_SERVER_KEY_PREFIX) {
            if !is_trae_device_provider_id(provider.as_str()) {
                return provider;
            }
        }
    }
    if let Some(key) =
        find_storage_key_by_prefix(root_obj, TRAE_STORAGE_ENTITLEMENT_KEY_PREFIX, None)
    {
        if let Some(provider) =
            provider_id_from_storage_key(&key, TRAE_STORAGE_ENTITLEMENT_KEY_PREFIX)
        {
            if !is_trae_device_provider_id(provider.as_str()) {
                return provider;
            }
        }
    }
    TRAE_DEFAULT_AUTH_PROVIDER_ID.to_string()
}

fn has_trae_auth_storage_key(root_obj: &Map<String, Value>) -> bool {
    find_user_auth_storage_key(root_obj).is_some()
}

fn resolve_usertag_from_storage(
    root_obj: Option<&Map<String, Value>>,
    user_id: Option<&str>,
    auth_raw: Option<&Value>,
    server_raw: Option<&Value>,
) -> Option<String> {
    if let Some(obj) = root_obj {
        if let Some(raw_text) = obj
            .get(TRAE_STORAGE_USERTAG_KEY)
            .and_then(|value| value.as_str())
            .and_then(|value| normalize_non_empty(Some(value)))
        {
            if let Some(map) = decode_usertag_map(raw_text.as_str()) {
                if let Some(uid) = user_id.and_then(|value| normalize_non_empty(Some(value))) {
                    if let Some(tag) = map.get(&uid) {
                        return Some(tag.clone());
                    }
                }
                if map.len() == 1 {
                    if let Some(tag) = map.values().next() {
                        return Some(tag.clone());
                    }
                }
            }

            if let Some(tag) = normalize_usertag_value(Some(raw_text.as_str())) {
                return Some(tag);
            }
        }
    }

    normalize_usertag_value(
        pick_string(
            auth_raw,
            &[
                &["account", "userTag"],
                &["userTag"],
                &["callbackQuery", "userTag"],
                &["rawQuery", "userTag"],
                &["data", "userTag"],
            ],
        )
        .as_deref(),
    )
    .or_else(|| {
        normalize_usertag_value(
            pick_string(
                server_raw,
                &[&["account", "userTag"], &["userTag"], &["data", "userTag"]],
            )
            .as_deref(),
        )
    })
}

fn resolve_account_user_id_for_inject(account: &TraeAccount) -> Option<String> {
    normalize_non_empty(account.user_id.as_deref()).or_else(|| {
        pick_string(
            account.trae_auth_raw.as_ref(),
            &[
                &["userId"],
                &["user_id"],
                &["uid"],
                &["id"],
                &["account", "uid"],
            ],
        )
    })
}

fn merge_auth_fields(auth_raw: Option<&Value>, payload: &TraeImportPayload) -> Option<Value> {
    let mut merged = match auth_raw {
        Some(Value::Object(obj)) => obj.clone(),
        _ => Map::new(),
    };

    merged.insert(
        "accessToken".to_string(),
        Value::String(payload.access_token.clone()),
    );
    if let Some(refresh) = payload.refresh_token.as_ref() {
        merged.insert("refreshToken".to_string(), Value::String(refresh.clone()));
    }
    merged.insert("email".to_string(), Value::String(payload.email.clone()));
    if let Some(user_id) = payload.user_id.as_ref() {
        merged.insert("userId".to_string(), Value::String(user_id.clone()));
    }
    if let Some(token_type) = payload.token_type.as_ref() {
        merged.insert("tokenType".to_string(), Value::String(token_type.clone()));
    }
    if let Some(expires_at) = payload.expires_at {
        merged.insert(
            "expiresAt".to_string(),
            Value::Number(serde_json::Number::from(expires_at)),
        );
    }
    Some(Value::Object(merged))
}

fn normalize_email_from_payload(payload: &TraeImportPayload) -> String {
    if let Some(email) = normalize_email(Some(payload.email.as_str())) {
        return email;
    }
    if let Some(user_id) = normalize_non_empty(payload.user_id.as_deref()) {
        if user_id.contains('@') {
            return user_id.to_lowercase();
        }
    }
    if let Some(name) = normalize_non_empty(payload.nickname.as_deref()) {
        if name.contains('@') {
            return name.to_lowercase();
        }
    }
    "unknown".to_string()
}

fn resolve_payload_identity(payload: &TraeImportPayload) -> String {
    normalize_non_empty(payload.user_id.as_deref())
        .or_else(|| normalize_email(Some(payload.email.as_str())))
        .or_else(|| normalize_non_empty(Some(payload.access_token.as_str())))
        .unwrap_or_else(|| "trae_user".to_string())
}

fn resolve_payload_platform_kind(payload: &TraeImportPayload) -> TraePlatformKind {
    let profile_root = profile_payload_root(payload.trae_profile_raw.as_ref());
    let roots = [
        payload.trae_auth_raw.as_ref(),
        profile_root,
        payload.trae_server_raw.as_ref(),
        payload.trae_entitlement_raw.as_ref(),
        payload.trae_usage_raw.as_ref(),
    ];
    resolve_platform_from_roots(&roots)
}

fn resolve_platform_scoped_payload_identity(
    platform: TraePlatformKind,
    payload: &TraeImportPayload,
) -> String {
    format!(
        "{}:{}",
        platform.provider_key(),
        resolve_payload_identity(payload)
    )
}

fn apply_payload(account: &mut TraeAccount, payload: TraeImportPayload) {
    let merged_auth_raw = merge_auth_fields(payload.trae_auth_raw.as_ref(), &payload);
    account.email = normalize_email_from_payload(&payload);
    account.user_id = normalize_non_empty(payload.user_id.as_deref());
    account.nickname = normalize_non_empty(payload.nickname.as_deref());
    account.access_token = payload.access_token;
    account.refresh_token = normalize_non_empty(payload.refresh_token.as_deref());
    account.token_type = normalize_non_empty(payload.token_type.as_deref());
    account.expires_at = normalize_timestamp(payload.expires_at);
    account.plan_type = normalize_non_empty(payload.plan_type.as_deref());
    account.plan_reset_at = normalize_timestamp(payload.plan_reset_at);
    account.trae_auth_raw = merged_auth_raw;
    account.trae_profile_raw = payload.trae_profile_raw;
    account.trae_entitlement_raw = payload.trae_entitlement_raw;
    account.trae_usage_raw = payload.trae_usage_raw;
    account.trae_server_raw = payload.trae_server_raw;
    account.trae_usertag_raw = normalize_non_empty(payload.trae_usertag_raw.as_deref());
    account.status = normalize_non_empty(payload.status.as_deref());
    account.status_reason = normalize_non_empty(payload.status_reason.as_deref());
    account.last_used = now_ts();
}

pub fn list_accounts() -> Vec<TraeAccount> {
    let index = load_account_index();
    index
        .accounts
        .iter()
        .filter_map(|item| load_account(item.id.as_str()))
        .collect()
}

pub fn list_accounts_checked() -> Result<Vec<TraeAccount>, String> {
    let index = load_account_index_checked()?;
    Ok(index
        .accounts
        .iter()
        .filter_map(|item| load_account(item.id.as_str()))
        .collect())
}

pub fn upsert_account(payload: TraeImportPayload) -> Result<TraeAccount, String> {
    let _lock = TRAE_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 Trae 账号锁失败".to_string())?;

    let now = now_ts();
    let mut index = load_account_index();
    let normalized_user_id = normalize_non_empty(payload.user_id.as_deref());
    let normalized_email = normalize_email(Some(payload.email.as_str()));
    let incoming_platform = resolve_payload_platform_kind(&payload);

    let identity = resolve_platform_scoped_payload_identity(incoming_platform, &payload);
    let generated_id = format!("trae_{:x}", md5::compute(identity.as_bytes()));

    let account_id = index
        .accounts
        .iter()
        .filter_map(|summary| load_account(summary.id.as_str()))
        .find(|account| {
            if resolve_account_platform_kind(account) != incoming_platform {
                return false;
            }

            if let (Some(left), Some(right)) = (
                normalize_non_empty(account.user_id.as_deref()),
                normalized_user_id.clone(),
            ) {
                if left == right {
                    return true;
                }
            }

            if let (Some(left), Some(right)) = (
                normalize_email(Some(account.email.as_str())),
                normalized_email.clone(),
            ) {
                return left == right;
            }
            false
        })
        .map(|account| account.id)
        .unwrap_or(generated_id);

    let existing = load_account(&account_id);
    let tags = existing.as_ref().and_then(|item| item.tags.clone());
    let created_at = existing.as_ref().map(|item| item.created_at).unwrap_or(now);

    let mut account = existing.unwrap_or(TraeAccount {
        id: account_id.clone(),
        email: normalize_email_from_payload(&payload),
        user_id: normalized_user_id,
        nickname: normalize_non_empty(payload.nickname.as_deref()),
        tags: tags.clone(),
        access_token: payload.access_token.clone(),
        refresh_token: normalize_non_empty(payload.refresh_token.as_deref()),
        token_type: normalize_non_empty(payload.token_type.as_deref()),
        expires_at: normalize_timestamp(payload.expires_at),
        plan_type: normalize_non_empty(payload.plan_type.as_deref()),
        plan_reset_at: normalize_timestamp(payload.plan_reset_at),
        trae_auth_raw: merge_auth_fields(payload.trae_auth_raw.as_ref(), &payload),
        trae_profile_raw: payload.trae_profile_raw.clone(),
        trae_entitlement_raw: payload.trae_entitlement_raw.clone(),
        trae_usage_raw: payload.trae_usage_raw.clone(),
        trae_server_raw: payload.trae_server_raw.clone(),
        trae_usertag_raw: normalize_non_empty(payload.trae_usertag_raw.as_deref()),
        status: normalize_non_empty(payload.status.as_deref()),
        status_reason: normalize_non_empty(payload.status_reason.as_deref()),
        quota_query_last_error: None,
        quota_query_last_error_at: None,
        usage_updated_at: None,
        created_at,
        last_used: now,
    });

    account.tags = tags;
    apply_payload(&mut account, payload);
    account.id = account_id;
    account.created_at = created_at;
    account.quota_query_last_error = None;
    account.quota_query_last_error_at = None;
    account.last_used = now;

    save_account_file(&account)?;
    refresh_summary(&mut index, &account);
    save_account_index(&index)?;

    logger::log_info(&format!(
        "[Trae Account] 账号已保存: id={}, email={}",
        account.id, account.email
    ));
    Ok(account)
}

pub fn remove_account(account_id: &str) -> Result<(), String> {
    let _lock = TRAE_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 Trae 账号锁失败".to_string())?;
    let mut index = load_account_index();
    index.accounts.retain(|item| item.id != account_id);
    save_account_index(&index)?;
    delete_account_file(account_id)?;
    Ok(())
}

pub fn remove_accounts(account_ids: &[String]) -> Result<(), String> {
    for id in account_ids {
        remove_account(id)?;
    }
    Ok(())
}

pub fn update_account_tags(account_id: &str, tags: Vec<String>) -> Result<TraeAccount, String> {
    let mut account = load_account(account_id).ok_or_else(|| "账号不存在".to_string())?;
    account.tags = Some(tags);
    account.last_used = now_ts();
    let updated = account.clone();
    upsert_account_record(account)?;
    Ok(updated)
}

fn storage_object_value(root: &Value, key: &str) -> Option<Value> {
    root.as_object()
        .and_then(|obj| parse_value_or_json_string(obj.get(key)))
}

fn storage_object_value_auth(root: &Value, key: &str) -> Option<Value> {
    root.as_object()
        .and_then(|obj| parse_value_or_json_string_or_icube_cipher(obj.get(key)))
}

fn payload_from_storage_root(storage_root: &Value) -> Result<TraeImportPayload, String> {
    let root_obj = storage_root.as_object();
    let provider_id = root_obj
        .map(resolve_storage_provider_id)
        .unwrap_or_else(|| TRAE_DEFAULT_AUTH_PROVIDER_ID.to_string());
    let auth_storage_key = build_auth_storage_key(provider_id.as_str());
    let server_storage_key = build_server_storage_key(provider_id.as_str());
    let entitlement_storage_key = build_entitlement_storage_key(provider_id.as_str());

    let auth_raw = storage_object_value_auth(storage_root, auth_storage_key.as_str())
        .or_else(|| storage_object_value_auth(storage_root, TRAE_STORAGE_AUTH_KEY));
    let entitlement_raw = storage_object_value(storage_root, entitlement_storage_key.as_str())
        .or_else(|| storage_object_value(storage_root, TRAE_STORAGE_ENTITLEMENT_KEY));
    let server_raw = storage_object_value(storage_root, server_storage_key.as_str())
        .or_else(|| storage_object_value(storage_root, TRAE_STORAGE_SERVER_KEY));

    let access_token = pick_string(
        auth_raw.as_ref(),
        &[
            &["accessToken"],
            &["access_token"],
            &["token"],
            &["data", "accessToken"],
            &["data", "access_token"],
            &["auth", "accessToken"],
            &["auth", "token"],
        ],
    )
    .or_else(|| {
        pick_string(
            server_raw.as_ref(),
            &[
                &["accessToken"],
                &["access_token"],
                &["token"],
                &["data", "accessToken"],
                &["data", "token"],
            ],
        )
    })
    .ok_or_else(|| "Trae 本地存储缺少 access token".to_string())?;

    let refresh_token = pick_string(
        auth_raw.as_ref(),
        &[
            &["refreshToken"],
            &["refresh_token"],
            &["RefreshToken"],
            &["exchangeResponse", "Result", "RefreshToken"],
            &["data", "refreshToken"],
            &["data", "refresh_token"],
        ],
    );

    let email = normalize_email(
        pick_string(
            auth_raw.as_ref(),
            &[
                &["email"],
                &["account", "email"],
                &["account", "nonPlainTextEmail"],
                &["NonPlainTextEmail"],
                &["data", "email"],
                &["user", "email"],
                &["userInfo", "email"],
            ],
        )
        .as_deref(),
    )
    .or_else(|| {
        normalize_email(
            pick_string(
                server_raw.as_ref(),
                &[&["email"], &["data", "email"], &["user", "email"]],
            )
            .as_deref(),
        )
    })
    .unwrap_or_else(|| "unknown".to_string());

    let user_id = pick_string(
        auth_raw.as_ref(),
        &[
            &["userId"],
            &["user_id"],
            &["uid"],
            &["id"],
            &["data", "userId"],
            &["data", "uid"],
            &["user", "id"],
        ],
    )
    .or_else(|| {
        pick_string(
            server_raw.as_ref(),
            &[
                &["userId"],
                &["user_id"],
                &["uid"],
                &["id"],
                &["account", "uid"],
                &["data", "userId"],
                &["data", "uid"],
                &["user", "id"],
            ],
        )
    });

    let nickname = pick_string(
        auth_raw.as_ref(),
        &[
            &["nickname"],
            &["name"],
            &["displayName"],
            &["account", "username"],
            &["data", "nickname"],
            &["user", "nickname"],
            &["user", "name"],
        ],
    )
    .or_else(|| {
        pick_string(
            server_raw.as_ref(),
            &[
                &["nickname"],
                &["name"],
                &["displayName"],
                &["data", "nickname"],
                &["user", "name"],
            ],
        )
    });

    let token_type = pick_string(
        auth_raw.as_ref(),
        &[
            &["tokenType"],
            &["token_type"],
            &["TokenType"],
            &["data", "tokenType"],
        ],
    );
    let expires_at = normalize_timestamp(
        pick_i64(
            auth_raw.as_ref(),
            &[
                &["expiresAt"],
                &["expiredAt"],
                &["expires_at"],
                &["TokenExpireAt"],
                &["exchangeResponse", "Result", "TokenExpireAt"],
                &["data", "expiresAt"],
            ],
        )
        .or_else(|| {
            pick_i64(
                server_raw.as_ref(),
                &[&["expiresAt"], &["expires_at"], &["data", "expiresAt"]],
            )
        }),
    );

    let plan_type = pick_string(
        entitlement_raw.as_ref(),
        &[
            &["identityStr"],
            &["identity_str"],
            &["user_pay_identity_str"],
            &["entitlementInfo", "identityStr"],
            &["data", "user_pay_identity_str"],
        ],
    )
    .or_else(|| {
        pick_string(
            server_raw.as_ref(),
            &[
                &["entitlementInfo", "identityStr"],
                &["identityStr"],
                &["data", "entitlementInfo", "identityStr"],
            ],
        )
    });
    let plan_reset_at = normalize_timestamp(
        pick_i64(
            entitlement_raw.as_ref(),
            &[
                &["detail", "subscription_renew_time"],
                &["detail", "subscriptionRenewTime"],
                &["data", "detail", "subscription_renew_time"],
                &["entitlementInfo", "detail", "subscription_renew_time"],
                &["entitlementInfo", "detail", "subscriptionRenewTime"],
            ],
        )
        .or_else(|| {
            pick_i64(
                server_raw.as_ref(),
                &[
                    &["entitlementInfo", "detail", "subscription_renew_time"],
                    &["entitlementInfo", "detail", "subscriptionRenewTime"],
                    &[
                        "data",
                        "entitlementInfo",
                        "detail",
                        "subscription_renew_time",
                    ],
                ],
            )
        }),
    );

    let status = pick_string(
        auth_raw.as_ref(),
        &[&["status"], &["data", "status"], &["loginStatus"]],
    )
    .or_else(|| pick_string(server_raw.as_ref(), &[&["status"], &["data", "status"]]));
    let status_reason = pick_string(
        auth_raw.as_ref(),
        &[
            &["statusReason"],
            &["status_reason"],
            &["message"],
            &["data", "message"],
        ],
    )
    .or_else(|| {
        pick_string(
            server_raw.as_ref(),
            &[&["statusReason"], &["status_reason"], &["message"]],
        )
    });

    let usertag_raw = resolve_usertag_from_storage(
        root_obj,
        user_id.as_deref(),
        auth_raw.as_ref(),
        server_raw.as_ref(),
    );

    Ok(TraeImportPayload {
        email,
        user_id,
        nickname,
        access_token,
        refresh_token,
        token_type,
        expires_at,
        plan_type,
        plan_reset_at,
        trae_auth_raw: auth_raw,
        trae_profile_raw: None,
        trae_entitlement_raw: entitlement_raw,
        trae_usage_raw: None,
        trae_server_raw: server_raw,
        trae_usertag_raw: usertag_raw,
        status,
        status_reason,
    })
}

fn payload_from_import_value(raw: Value) -> Result<TraeImportPayload, String> {
    let obj = raw
        .as_object()
        .ok_or_else(|| "Trae 导入项必须是对象".to_string())?;

    if obj.contains_key(TRAE_STORAGE_AUTH_KEY) || has_trae_auth_storage_key(obj) {
        return payload_from_storage_root(&raw);
    }

    let auth_raw = obj
        .get("trae_auth_raw")
        .cloned()
        .or_else(|| obj.get("auth_raw").cloned())
        .or_else(|| obj.get("auth").cloned());
    let entitlement_raw = obj
        .get("trae_entitlement_raw")
        .cloned()
        .or_else(|| obj.get("entitlement_raw").cloned())
        .or_else(|| obj.get("usage_raw").cloned())
        .or_else(|| obj.get("quota_raw").cloned());
    let usage_raw = obj
        .get("trae_usage_raw")
        .cloned()
        .or_else(|| obj.get("usage_status_raw").cloned())
        .or_else(|| obj.get("ent_usage_raw").cloned());
    let profile_raw = obj
        .get("trae_profile_raw")
        .cloned()
        .or_else(|| obj.get("profile_raw").cloned())
        .or_else(|| obj.get("profile").cloned());
    let server_raw = obj
        .get("trae_server_raw")
        .cloned()
        .or_else(|| obj.get("server_raw").cloned())
        .or_else(|| obj.get("server").cloned());
    let usertag_raw = obj
        .get("trae_usertag_raw")
        .and_then(|value| value.as_str())
        .and_then(|value| normalize_non_empty(Some(value)));

    let access_token = pick_string(
        Some(&raw),
        &[
            &["access_token"],
            &["accessToken"],
            &["token"],
            &["trae_access_token"],
        ],
    )
    .or_else(|| {
        pick_string(
            auth_raw.as_ref(),
            &[&["accessToken"], &["access_token"], &["token"]],
        )
    })
    .ok_or_else(|| "缺少 access_token 字段".to_string())?;

    let refresh_token = pick_string(
        Some(&raw),
        &[
            &["refresh_token"],
            &["refreshToken"],
            &["trae_refresh_token"],
        ],
    )
    .or_else(|| pick_string(auth_raw.as_ref(), &[&["refreshToken"], &["refresh_token"]]));

    let email = normalize_email(
        pick_string(
            Some(&raw),
            &[&["email"], &["trae_email"], &["user", "email"]],
        )
        .as_deref(),
    )
    .or_else(|| {
        normalize_email(
            pick_string(
                auth_raw.as_ref(),
                &[
                    &["email"],
                    &["account", "email"],
                    &["account", "nonPlainTextEmail"],
                    &["NonPlainTextEmail"],
                    &["user", "email"],
                ],
            )
            .as_deref(),
        )
    })
    .unwrap_or_else(|| "unknown".to_string());

    let user_id =
        pick_string(Some(&raw), &[&["user_id"], &["userId"], &["uid"], &["id"]]).or_else(|| {
            pick_string(
                auth_raw.as_ref(),
                &[&["userId"], &["uid"], &["id"], &["account", "uid"]],
            )
        });
    let nickname = pick_string(
        Some(&raw),
        &[
            &["nickname"],
            &["name"],
            &["displayName"],
            &["user", "name"],
        ],
    )
    .or_else(|| {
        pick_string(
            profile_raw.as_ref(),
            &[
                &["nickname"],
                &["name"],
                &["displayName"],
                &["Result", "ScreenName"],
                &["Result", "Nickname"],
            ],
        )
    })
    .or_else(|| pick_string(auth_raw.as_ref(), &[&["account", "username"]]));
    let token_type = pick_string(Some(&raw), &[&["token_type"], &["tokenType"]]).or_else(|| {
        pick_string(
            auth_raw.as_ref(),
            &[&["tokenType"], &["token_type"], &["TokenType"]],
        )
    });
    let expires_at = normalize_timestamp(
        pick_i64(
            Some(&raw),
            &[&["expires_at"], &["expiresAt"], &["expiredAt"]],
        )
        .or_else(|| {
            pick_i64(
                auth_raw.as_ref(),
                &[
                    &["expiresAt"],
                    &["expiredAt"],
                    &["TokenExpireAt"],
                    &["exchangeResponse", "Result", "TokenExpireAt"],
                    &["expires_at"],
                ],
            )
        }),
    );
    let plan_type = pick_string(
        Some(&raw),
        &[
            &["identityStr"],
            &["identity_str"],
            &["user_pay_identity_str"],
        ],
    )
    .or_else(|| {
        pick_string(
            entitlement_raw.as_ref(),
            &[
                &["identityStr"],
                &["identity_str"],
                &["user_pay_identity_str"],
                &["entitlementInfo", "identityStr"],
                &["data", "user_pay_identity_str"],
            ],
        )
    })
    .or_else(|| {
        pick_string(
            usage_raw.as_ref(),
            &[
                &["identityStr"],
                &["identity_str"],
                &["data", "identityStr"],
                &["entitlementInfo", "identityStr"],
            ],
        )
    });
    let plan_reset_at = normalize_timestamp(
        pick_i64(
            Some(&raw),
            &[&["plan_reset_at"], &["detail", "subscription_renew_time"]],
        )
        .or_else(|| {
            pick_i64(
                entitlement_raw.as_ref(),
                &[
                    &["detail", "subscription_renew_time"],
                    &["entitlementInfo", "detail", "subscription_renew_time"],
                ],
            )
        })
        .or_else(|| {
            pick_i64(
                usage_raw.as_ref(),
                &[
                    &["currentPlan", "timeInfo", "nextResetTime"],
                    &["data", "currentPlan", "timeInfo", "nextResetTime"],
                    &["nextResetTime"],
                ],
            )
        }),
    );

    let status = pick_string(Some(&raw), &[&["status"]])
        .or_else(|| pick_string(auth_raw.as_ref(), &[&["status"], &["loginStatus"]]));
    let status_reason = pick_string(Some(&raw), &[&["status_reason"], &["statusReason"]])
        .or_else(|| pick_string(auth_raw.as_ref(), &[&["statusReason"], &["message"]]));

    Ok(TraeImportPayload {
        email,
        user_id,
        nickname,
        access_token,
        refresh_token,
        token_type,
        expires_at,
        plan_type,
        plan_reset_at,
        trae_auth_raw: auth_raw,
        trae_profile_raw: profile_raw,
        trae_entitlement_raw: entitlement_raw,
        trae_usage_raw: usage_raw,
        trae_server_raw: server_raw,
        trae_usertag_raw: usertag_raw,
        status,
        status_reason,
    })
}

fn payloads_from_import_json_value(raw: Value) -> Result<Vec<TraeImportPayload>, String> {
    match raw {
        Value::Array(items) => {
            if items.is_empty() {
                return Err("导入数组为空".to_string());
            }
            let mut payloads = Vec::with_capacity(items.len());
            for (idx, item) in items.into_iter().enumerate() {
                let payload = payload_from_import_value(item)
                    .map_err(|e| format!("第 {} 条 Trae 账号解析失败: {}", idx + 1, e))?;
                payloads.push(payload);
            }
            Ok(payloads)
        }
        Value::Object(obj) => {
            if let Some(accounts_raw) = obj.get("accounts") {
                if let Some(accounts) = accounts_raw.as_array() {
                    if accounts.is_empty() {
                        return Err("导入数组为空".to_string());
                    }
                    let mut payloads = Vec::with_capacity(accounts.len());
                    for (idx, item) in accounts.iter().enumerate() {
                        let payload = payload_from_import_value(item.clone())
                            .map_err(|e| format!("第 {} 条 Trae 账号解析失败: {}", idx + 1, e))?;
                        payloads.push(payload);
                    }
                    return Ok(payloads);
                }
            }
            Ok(vec![payload_from_import_value(Value::Object(obj))?])
        }
        _ => Err("Trae 导入 JSON 必须是对象或数组".to_string()),
    }
}

pub fn import_from_json(json_content: &str) -> Result<Vec<TraeAccount>, String> {
    if let Ok(account) = serde_json::from_str::<TraeAccount>(json_content) {
        let saved = upsert_account_record(account)?;
        return Ok(vec![saved]);
    }

    if let Ok(accounts) = serde_json::from_str::<Vec<TraeAccount>>(json_content) {
        let mut result = Vec::new();
        for account in accounts {
            let saved = upsert_account_record(account)?;
            result.push(saved);
        }
        return Ok(result);
    }

    let value = serde_json::from_str::<Value>(json_content)
        .map_err(|e| format!("解析 JSON 失败: {}", e))?;
    let payloads = payloads_from_import_json_value(value)?;
    let mut result = Vec::with_capacity(payloads.len());
    for payload in payloads {
        let saved = upsert_account(payload)?;
        result.push(saved);
    }
    Ok(result)
}

pub fn export_accounts(account_ids: &[String]) -> Result<String, String> {
    let accounts: Vec<TraeAccount> = account_ids
        .iter()
        .filter_map(|id| load_account(id))
        .collect();
    serde_json::to_string_pretty(&accounts).map_err(|e| format!("序列化失败: {}", e))
}

pub fn get_default_trae_data_dir_for_platform(
    platform: TraePlatformKind,
) -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
        return Ok(home
            .join("Library/Application Support")
            .join(platform.app_support_dir_name()));
    }

    #[cfg(target_os = "windows")]
    {
        let appdata =
            std::env::var("APPDATA").map_err(|_| "无法获取 APPDATA 环境变量".to_string())?;
        return Ok(PathBuf::from(appdata).join(platform.app_support_dir_name()));
    }

    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
        return Ok(home.join(".config").join(platform.app_support_dir_name()));
    }

    #[allow(unreachable_code)]
    Err("Trae 仅支持 macOS、Windows 和 Linux".to_string())
}

pub fn get_default_trae_data_dir() -> Result<PathBuf, String> {
    get_default_trae_data_dir_for_platform(TraePlatformKind::Trae)
}

pub fn get_default_trae_storage_path_for_platform(
    platform: TraePlatformKind,
) -> Result<PathBuf, String> {
    Ok(get_default_trae_data_dir_for_platform(platform)?
        .join("User")
        .join("globalStorage")
        .join("storage.json"))
}

pub fn get_default_trae_storage_path() -> Result<PathBuf, String> {
    get_default_trae_storage_path_for_platform(TraePlatformKind::Trae)
}

fn read_storage_json(path: &Path) -> Result<Value, String> {
    if !path.exists() {
        return Err(format!("Trae storage.json 不存在: {}", path.display()));
    }

    let content = fs::read_to_string(path)
        .map_err(|e| format!("读取 Trae storage.json 失败({}): {}", path.display(), e))?;
    if content.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }

    serde_json::from_str::<Value>(&content)
        .map_err(|e| format!("解析 Trae storage.json 失败({}): {}", path.display(), e))
}

fn write_storage_json(path: &Path, value: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建 Trae 目录失败: {}", e))?;
    }
    let content = serde_json::to_string_pretty(value)
        .map_err(|e| format!("序列化 Trae storage.json 失败: {}", e))?;
    crate::modules::atomic_write::write_string_atomic(path, &content)
        .map_err(|e| format!("写入 Trae storage.json 失败: {}", e))
}

fn to_json_string_value(value: &Value) -> Result<Value, String> {
    let text =
        serde_json::to_string(value).map_err(|e| format!("序列化 Trae 存储键值失败: {}", e))?;
    Ok(Value::String(text))
}

fn to_icube_cipher_string_value(value: &Value) -> Result<Value, String> {
    let plaintext =
        serde_json::to_string(value).map_err(|e| format!("序列化 Trae 存储键值失败: {}", e))?;
    let encrypted = byte_crypto_encrypt_v1(plaintext.as_bytes())?;
    Ok(Value::String(BASE64_STANDARD.encode(encrypted)))
}

fn pick_string_multi(roots: &[Option<&Value>], paths: &[&[&str]]) -> Option<String> {
    for root in roots {
        if let Some(value) = pick_string(*root, paths) {
            return Some(value);
        }
    }
    None
}

fn pick_i64_multi(roots: &[Option<&Value>], paths: &[&[&str]]) -> Option<i64> {
    for root in roots {
        if let Some(value) = pick_i64(*root, paths) {
            return Some(value);
        }
    }
    None
}

fn json_value_to_non_empty_string(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return normalize_non_empty(Some(text));
    }
    if let Some(num) = value.as_i64() {
        return Some(num.to_string());
    }
    if let Some(num) = value.as_u64() {
        return Some(num.to_string());
    }
    None
}

fn parse_json_file(path: &Path) -> Option<Value> {
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str::<Value>(&content).ok()
}

fn is_probable_executable_path(path: &Path) -> bool {
    if path.is_file() {
        return true;
    }
    path.extension()
        .and_then(|value| value.to_str())
        .map(|value| value.eq_ignore_ascii_case("exe"))
        .unwrap_or(false)
}

fn build_trae_product_file_candidates(base_path: &Path) -> Vec<PathBuf> {
    let mut app_roots: Vec<PathBuf> = Vec::new();
    let base_path_string = base_path.to_string_lossy().to_string();

    if let Some(app_idx) = base_path_string.find(".app") {
        app_roots.push(PathBuf::from(&base_path_string[..app_idx + 4]));
    }
    if base_path.is_dir() {
        app_roots.push(base_path.to_path_buf());
    }
    if is_probable_executable_path(base_path) {
        if let Some(parent) = base_path.parent() {
            app_roots.push(parent.to_path_buf());
        }
    }
    if app_roots.is_empty() {
        app_roots.push(base_path.to_path_buf());
    }

    let mut candidates = Vec::new();
    for root in app_roots {
        candidates.extend([
            root.join("Contents")
                .join("Resources")
                .join("app")
                .join("product.json"),
            root.join("Contents")
                .join("Resources")
                .join("app")
                .join("package.json"),
            root.join("resources").join("app").join("product.json"),
            root.join("resources").join("app").join("package.json"),
            root.join("product.json"),
            root.join("package.json"),
        ]);
    }
    candidates
}

#[cfg(target_os = "windows")]
fn trae_product_exe_names(platform: TraePlatformKind) -> &'static [&'static str] {
    match platform {
        TraePlatformKind::Trae => &["Trae.exe"],
        TraePlatformKind::TraeSolo => &["TRAE SOLO.exe", "Trae.exe", "Electron.exe"],
        TraePlatformKind::TraeCn => &["Trae CN.exe", "Trae.exe", "Electron.exe"],
        TraePlatformKind::TraeSoloCn => &["TRAE SOLO CN.exe", "Trae.exe", "Electron.exe"],
    }
}

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[cfg(target_os = "windows")]
fn windows_cmd_output_utf16(args: &[&str]) -> Option<std::process::Output> {
    use std::os::windows::process::CommandExt;

    let mut command = std::process::Command::new("cmd");
    command.args(args);
    command.creation_flags(CREATE_NO_WINDOW);
    command.output().ok()
}

#[cfg(target_os = "windows")]
fn decode_utf16le(bytes: &[u8]) -> String {
    let words: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect();
    String::from_utf16_lossy(&words)
}

#[cfg(target_os = "windows")]
fn registry_line_value(line: &str) -> Option<String> {
    let pos = line.find("REG_")?;
    let after = &line[pos..];
    let value_start = after.find(char::is_whitespace)?;
    let value = after[value_start..].trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

#[cfg(target_os = "windows")]
fn reg_query_value(key: &str, value_name: &str) -> Option<String> {
    let cmd = format!("reg query \"{}\" /v \"{}\"", key, value_name);
    let output = windows_cmd_output_utf16(&["/u", "/c", cmd.as_str()])?;
    if !output.status.success() {
        return None;
    }

    let stdout = decode_utf16le(output.stdout.as_slice());
    let value_name_lower = value_name.to_ascii_lowercase();
    stdout.lines().find_map(|line| {
        let trimmed = line.trim();
        if trimmed
            .to_ascii_lowercase()
            .starts_with(value_name_lower.as_str())
        {
            registry_line_value(trimmed)
        } else {
            None
        }
    })
}

#[cfg(target_os = "windows")]
fn normalize_windows_uninstall_display_name(value: &str) -> String {
    let mut normalized = value
        .trim()
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if let Some(stripped) = normalized.strip_suffix(" (user)") {
        normalized = stripped.to_string();
    }
    normalized
}

#[cfg(target_os = "windows")]
fn windows_uninstall_display_names(platform: TraePlatformKind) -> &'static [&'static str] {
    match platform {
        TraePlatformKind::Trae => &["Trae"],
        TraePlatformKind::TraeSolo => &["TRAE SOLO"],
        TraePlatformKind::TraeCn => &["Trae CN"],
        TraePlatformKind::TraeSoloCn => &["TRAE SOLO CN", "TRAE Work CN"],
    }
}

#[cfg(target_os = "windows")]
fn windows_uninstall_display_name_matches(platform: TraePlatformKind, display_name: &str) -> bool {
    let display_name = normalize_windows_uninstall_display_name(display_name);
    windows_uninstall_display_names(platform)
        .iter()
        .any(|expected| normalize_windows_uninstall_display_name(expected) == display_name)
}

#[cfg(target_os = "windows")]
fn normalize_windows_registry_path(value: &str) -> Option<PathBuf> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    let path = if let Some(rest) = trimmed.strip_prefix('"') {
        rest.split('"').next().unwrap_or(rest).trim()
    } else if let Some(pos) = trimmed.to_ascii_lowercase().find(".exe") {
        &trimmed[..pos + 4]
    } else {
        trimmed.split(',').next().unwrap_or(trimmed).trim()
    };

    let path = path.trim_matches('"').trim();
    if path.is_empty() {
        None
    } else {
        Some(PathBuf::from(path))
    }
}

#[cfg(target_os = "windows")]
fn push_windows_install_dir_candidates(
    candidates: &mut Vec<PathBuf>,
    install_dir: &str,
    platform: TraePlatformKind,
) {
    let Some(root) = normalize_windows_registry_path(install_dir) else {
        return;
    };
    for exe_name in trae_product_exe_names(platform) {
        candidates.push(root.join(exe_name));
    }
    candidates.push(root);
}

#[cfg(target_os = "windows")]
pub(crate) fn windows_trae_install_base_paths(platform: TraePlatformKind) -> Vec<PathBuf> {
    let uninstall_roots = [
        "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
        "HKLM\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
        "HKLM\\Software\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
    ];
    let mut matched_keys = Vec::new();

    for root in uninstall_roots {
        let cmd = format!("reg query \"{}\" /s /v DisplayName", root);
        let Some(output) = windows_cmd_output_utf16(&["/u", "/c", cmd.as_str()]) else {
            continue;
        };
        if !output.status.success() {
            continue;
        }

        let stdout = decode_utf16le(output.stdout.as_slice());
        let mut current_key: Option<String> = None;
        for line in stdout.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("HKEY_") {
                current_key = Some(trimmed.to_string());
                continue;
            }
            if !trimmed.to_ascii_lowercase().starts_with("displayname") {
                continue;
            }
            let Some(display_name) = registry_line_value(trimmed) else {
                continue;
            };
            if windows_uninstall_display_name_matches(platform, display_name.as_str()) {
                if let Some(key) = current_key.as_ref() {
                    matched_keys.push(key.clone());
                }
            }
        }
    }

    let mut candidates = Vec::new();
    for key in matched_keys {
        if let Some(display_icon) = reg_query_value(key.as_str(), "DisplayIcon") {
            if let Some(exe_path) = normalize_windows_registry_path(display_icon.as_str()) {
                if let Some(parent) = exe_path.parent() {
                    candidates.push(parent.to_path_buf());
                }
                candidates.push(exe_path);
            }
        }
        if let Some(install_location) = reg_query_value(key.as_str(), "InstallLocation") {
            push_windows_install_dir_candidates(
                &mut candidates,
                install_location.as_str(),
                platform,
            );
        }
    }

    let mut dedup = BTreeMap::new();
    for candidate in candidates {
        dedup
            .entry(candidate.to_string_lossy().to_string())
            .or_insert(candidate);
    }
    dedup.into_values().collect()
}

#[cfg(target_os = "windows")]
fn normalize_windows_scan_root(raw: &str) -> Option<PathBuf> {
    let mut value = raw
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .to_string();
    if value.is_empty() {
        return None;
    }
    if value.len() == 2 && value.as_bytes().get(1) == Some(&b':') {
        value.push('\\');
    }
    let path = PathBuf::from(value);
    if path.is_dir() {
        Some(path)
    } else {
        None
    }
}

#[cfg(target_os = "windows")]
fn is_windows_drive_root(path: &Path) -> bool {
    let value = path.to_string_lossy().replace('/', "\\");
    let trimmed = value.trim_end_matches('\\');
    trimmed.len() == 2
        && trimmed.as_bytes().get(1) == Some(&b':')
        && trimmed.as_bytes()[0].is_ascii_alphabetic()
}

#[cfg(target_os = "windows")]
fn expand_windows_scan_roots_for_trae(roots: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut expanded = Vec::new();
    for root in roots {
        if is_windows_drive_root(&root) {
            expanded.push(root.join("Program Files"));
            expanded.push(root.join("Program Files (x86)"));
            let users_dir = root.join("Users");
            if let Ok(entries) = fs::read_dir(users_dir) {
                for entry in entries.flatten() {
                    expanded.push(entry.path().join("AppData").join("Local").join("Programs"));
                }
            }
        } else {
            expanded.push(root);
        }
    }

    let mut dedup = BTreeMap::new();
    for root in expanded {
        if root.is_dir() {
            dedup
                .entry(root.to_string_lossy().to_ascii_lowercase())
                .or_insert(root);
        }
    }
    dedup.into_values().collect()
}

#[cfg(target_os = "windows")]
pub(crate) fn windows_trae_scan_root_base_paths(platform: TraePlatformKind) -> Vec<PathBuf> {
    let scan_roots = trae_configured_app_scan_roots(platform);
    if scan_roots.trim().is_empty() {
        return Vec::new();
    }

    let roots = scan_roots
        .split(|ch| matches!(ch, '\n' | '\r' | ';' | ','))
        .filter_map(normalize_windows_scan_root)
        .collect::<Vec<_>>();
    let app_dir = platform.app_support_dir_name();
    let mut candidates = Vec::new();
    for root in expand_windows_scan_roots_for_trae(roots) {
        let root_is_app_dir = root
            .file_name()
            .and_then(|value| value.to_str())
            .map(|value| value.eq_ignore_ascii_case(app_dir))
            .unwrap_or(false);
        if root_is_app_dir {
            for exe_name in trae_product_exe_names(platform) {
                candidates.push(root.join(exe_name));
            }
            candidates.push(root.clone());
        }

        let install_dir = root.join(app_dir);
        for exe_name in trae_product_exe_names(platform) {
            candidates.push(install_dir.join(exe_name));
        }
        candidates.push(install_dir);
    }

    let mut dedup = BTreeMap::new();
    for candidate in candidates {
        dedup
            .entry(candidate.to_string_lossy().to_ascii_lowercase())
            .or_insert(candidate);
    }
    dedup.into_values().collect()
}

#[cfg(target_os = "linux")]
fn trae_product_linux_base_paths(platform: TraePlatformKind) -> &'static [&'static str] {
    match platform {
        TraePlatformKind::Trae => &[
            "/usr/bin/trae",
            "/usr/local/bin/trae",
            "/opt/trae/trae",
            "/opt/Trae",
        ],
        TraePlatformKind::TraeSolo => &[
            "/usr/bin/trae-solo",
            "/usr/local/bin/trae-solo",
            "/opt/trae-solo/trae-solo",
            "/opt/TRAE SOLO",
        ],
        TraePlatformKind::TraeCn => &[
            "/usr/bin/trae-cn",
            "/usr/local/bin/trae-cn",
            "/opt/trae-cn/trae-cn",
            "/opt/Trae CN",
        ],
        TraePlatformKind::TraeSoloCn => &[
            "/usr/bin/trae-solo-cn",
            "/usr/local/bin/trae-solo-cn",
            "/opt/trae-solo-cn/trae-solo-cn",
            "/opt/TRAE SOLO CN",
        ],
    }
}

fn trae_product_base_paths(platform: TraePlatformKind) -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    let configured_path = trae_configured_app_path(platform).trim().to_string();
    if !configured_path.is_empty() {
        candidates.push(PathBuf::from(configured_path));
    }

    #[cfg(target_os = "macos")]
    {
        let app_root = PathBuf::from("/Applications").join(platform.macos_app_name());
        candidates.push(app_root.clone());
        candidates.push(app_root.join("Contents"));
    }

    #[cfg(target_os = "windows")]
    {
        candidates.extend(windows_trae_install_base_paths(platform));
        candidates.extend(windows_trae_scan_root_base_paths(platform));

        let app_dir = platform.app_support_dir_name();
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            let programs_dir = PathBuf::from(&local_app_data)
                .join("Programs")
                .join(app_dir);
            for exe_name in trae_product_exe_names(platform) {
                candidates.push(programs_dir.join(exe_name));
            }
            candidates.push(programs_dir);
        }
        if let Ok(program_files) = std::env::var("ProgramFiles") {
            let install_dir = PathBuf::from(&program_files).join(app_dir);
            for exe_name in trae_product_exe_names(platform) {
                candidates.push(install_dir.join(exe_name));
            }
            candidates.push(install_dir);
        }
    }

    #[cfg(target_os = "linux")]
    {
        for candidate in trae_product_linux_base_paths(platform) {
            candidates.push(PathBuf::from(candidate));
        }
    }

    candidates
}

fn product_auth_config_group(platform: TraePlatformKind) -> &'static str {
    if platform.is_solo() {
        "SOLO"
    } else {
        "TRAE"
    }
}

fn read_product_auth_client_id(root: &Value, platform: TraePlatformKind) -> Option<String> {
    let group = product_auth_config_group(platform);
    let entries = root
        .get("iCubeApp")?
        .get("authConfig")?
        .get(group)?
        .as_object()?;
    let app_type = pick_string(Some(root), &[&["quality"]]).map(|value| value.to_lowercase());

    if let Some(quality) = app_type.and_then(|value| normalize_non_empty(Some(value.as_str()))) {
        if let Some(client_id) = entries
            .get(quality.as_str())
            .and_then(json_value_to_non_empty_string)
        {
            return Some(client_id);
        }
    }

    if let Some(client_id) = entries
        .get("stable")
        .and_then(json_value_to_non_empty_string)
    {
        return Some(client_id);
    }

    entries.values().find_map(json_value_to_non_empty_string)
}

fn detect_product_auth_client_id(platform: TraePlatformKind) -> Option<String> {
    for base_path in trae_product_base_paths(platform) {
        for candidate in build_trae_product_file_candidates(base_path.as_path()) {
            let Some(root) = parse_json_file(candidate.as_path()) else {
                continue;
            };
            if let Some(client_id) = read_product_auth_client_id(&root, platform) {
                return Some(client_id);
            }
        }
    }
    None
}

fn resolve_auth_client_id_from_roots(
    roots: &[Option<&Value>],
    platform: TraePlatformKind,
) -> String {
    let paths: &[&[&str]] = &[
        &["authClientId"],
        &["clientId"],
        &["ClientID"],
        &["platform", "authClientId"],
        &["platform", "clientId"],
        &["exchangeResponse", "ClientID"],
        &["exchangeResponse", "Result", "ClientID"],
        &["Result", "ClientID"],
        &["data", "ClientID"],
    ];
    let fallback = platform.auth_client_id();
    let mut first: Option<String> = None;

    for root in roots {
        for path in paths {
            let Some(candidate) = pick_string(*root, &[*path])
                .and_then(|value| normalize_non_empty(Some(value.as_str())))
            else {
                continue;
            };

            if first.is_none() {
                first = Some(candidate.clone());
            }
            if candidate != fallback {
                return candidate;
            }
        }
    }

    if let Some(product_client_id) = detect_product_auth_client_id(platform) {
        if product_client_id != fallback {
            return product_client_id;
        }
        if first.is_none() {
            first = Some(product_client_id);
        }
    }

    first.unwrap_or_else(|| fallback.to_string())
}

fn platform_metadata_value(platform: TraePlatformKind) -> Value {
    serde_json::json!({
        "platformId": platform.provider_key(),
        "platformName": platform.display_name(),
        "authClientId": platform.auth_client_id(),
        "authDomain": platform.auth_domain(),
    })
}

fn insert_platform_metadata(obj: &mut Map<String, Value>, platform: TraePlatformKind) {
    obj.insert(
        "platformId".to_string(),
        Value::String(platform.provider_key().to_string()),
    );
    obj.insert(
        "platformName".to_string(),
        Value::String(platform.display_name().to_string()),
    );
    obj.insert(
        "authClientId".to_string(),
        Value::String(platform.auth_client_id().to_string()),
    );
    obj.insert(
        "authDomain".to_string(),
        Value::String(platform.auth_domain().to_string()),
    );
}

fn with_platform_metadata(raw: Option<Value>, platform: TraePlatformKind) -> Value {
    let mut obj = match raw {
        Some(Value::Object(value)) => value,
        Some(value) => {
            let mut wrapped = Map::new();
            wrapped.insert("raw".to_string(), value);
            wrapped
        }
        None => Map::new(),
    };
    insert_platform_metadata(&mut obj, platform);
    Value::Object(obj)
}

fn attach_platform_metadata_to_payload(
    payload: &mut TraeImportPayload,
    platform: TraePlatformKind,
) {
    payload.trae_auth_raw = Some(with_platform_metadata(
        payload.trae_auth_raw.take(),
        platform,
    ));

    let mut server_obj = match payload.trae_server_raw.take() {
        Some(Value::Object(value)) => value,
        Some(value) => {
            let mut wrapped = Map::new();
            wrapped.insert("raw".to_string(), value);
            wrapped
        }
        None => Map::new(),
    };
    server_obj.insert("platform".to_string(), platform_metadata_value(platform));
    payload.trae_server_raw = Some(Value::Object(server_obj));
}

fn resolve_platform_from_roots(roots: &[Option<&Value>]) -> TraePlatformKind {
    if let Some(platform_id) = pick_string_multi(
        roots,
        &[
            &["platformId"],
            &["platform_id"],
            &["platform"],
            &["platform", "platformId"],
            &["platform", "platform_id"],
        ],
    ) {
        if let Ok(platform) = TraePlatformKind::parse(Some(platform_id.as_str())) {
            return platform;
        }
    }

    let client_id = pick_string_multi(
        roots,
        &[
            &["authClientId"],
            &["clientId"],
            &["ClientID"],
            &["exchangeResponse", "ClientID"],
            &["exchangeResponse", "Result", "ClientID"],
        ],
    )
    .map(|value| value.trim().to_string());
    let is_solo = client_id
        .as_deref()
        .map(|value| value == TRAE_SOLO_AUTH_CLIENT_ID)
        .unwrap_or(false);

    let domain_hint = pick_string_multi(
        roots,
        &[
            &["authDomain"],
            &["loginHost"],
            &["apiHost"],
            &["host"],
            &["callbackQuery", "host"],
            &["platform", "authDomain"],
        ],
    )
    .map(|value| value.to_ascii_lowercase())
    .unwrap_or_default();
    let provider_hint = pick_string_multi(
        roots,
        &[
            &["providerCode"],
            &["packageType"],
            &["platform", "providerCode"],
            &["platform", "packageType"],
        ],
    )
    .map(|value| value.to_ascii_lowercase())
    .unwrap_or_default();
    let is_cn = domain_hint.contains("trae.cn")
        || domain_hint.contains("trae.com.cn")
        || provider_hint == "cn"
        || provider_hint.ends_with("_cn");

    match (is_solo, is_cn) {
        (true, true) => TraePlatformKind::TraeSoloCn,
        (true, false) => TraePlatformKind::TraeSolo,
        (false, true) => TraePlatformKind::TraeCn,
        (false, false) => TraePlatformKind::Trae,
    }
}

fn resolve_account_platform_kind(account: &TraeAccount) -> TraePlatformKind {
    let profile_root = profile_payload_root(account.trae_profile_raw.as_ref());
    let roots = [
        account.trae_auth_raw.as_ref(),
        profile_root,
        account.trae_server_raw.as_ref(),
        account.trae_entitlement_raw.as_ref(),
        account.trae_usage_raw.as_ref(),
    ];
    resolve_platform_from_roots(&roots)
}

fn profile_payload_root(profile_raw: Option<&Value>) -> Option<&Value> {
    let root = profile_raw?;
    root.get("Result")
        .or_else(|| root.get("result"))
        .or_else(|| root.get("data"))
        .or(Some(root))
}

fn to_unix_millis(raw: i64) -> Option<i64> {
    if raw <= 0 {
        return None;
    }
    if raw > 10_000_000_000 {
        return Some(raw);
    }
    raw.checked_mul(1000)
}

fn normalize_iso_from_i64(raw: i64) -> Option<String> {
    let millis = to_unix_millis(raw)?;
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(millis)
        .map(|value| value.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

fn normalize_iso_from_text(raw: Option<&str>) -> Option<String> {
    let normalized = normalize_non_empty(raw)?;
    let trimmed = normalized.trim();
    if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(trimmed) {
        return Some(
            parsed
                .with_timezone(&chrono::Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        );
    }
    if let Ok(parsed) = trimmed.parse::<i64>() {
        return normalize_iso_from_i64(parsed);
    }
    None
}

fn normalize_iso_from_value(raw: Option<&Value>) -> Option<String> {
    let value = raw?;
    if let Some(text) = value.as_str() {
        return normalize_iso_from_text(Some(text));
    }
    if let Some(number) = value.as_i64() {
        return normalize_iso_from_i64(number);
    }
    if let Some(number) = value.as_u64() {
        if number <= i64::MAX as u64 {
            return normalize_iso_from_i64(number as i64);
        }
    }
    None
}

fn resolve_iso_timestamp(
    field_value: Option<i64>,
    roots: &[Option<&Value>],
    value_paths: &[&[&str]],
) -> Option<String> {
    if let Some(value) = field_value.and_then(normalize_iso_from_i64) {
        return Some(value);
    }

    for root in roots {
        for path in value_paths {
            if let Some(value) = extract_json_value(*root, path) {
                if let Some(normalized) = normalize_iso_from_value(Some(&value)) {
                    return Some(normalized);
                }
            }
        }
    }

    if let Some(value) = pick_i64_multi(roots, value_paths) {
        return normalize_iso_from_i64(value);
    }

    None
}

fn to_store_region(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "cn" | "china-north" => "CN".to_string(),
        "sg" | "singapore-central" => "SG".to_string(),
        "us" | "us-east" | "us-east-1" => "US".to_string(),
        "usttp" => "USTTP".to_string(),
        "unknown" => "UNKNOWN".to_string(),
        other if other.is_empty() => "UNKNOWN".to_string(),
        _ => raw.trim().to_string(),
    }
}

fn resolve_user_tag_for_inject(account: &TraeAccount) -> String {
    let user_id = resolve_account_user_id_for_inject(account);

    if let Some(raw_text) = normalize_non_empty(account.trae_usertag_raw.as_deref()) {
        if let Some(map) = decode_usertag_map(raw_text.as_str()) {
            if let Some(uid) = user_id.as_ref() {
                if let Some(value) = map.get(uid) {
                    return value.clone();
                }
            }
            if map.len() == 1 {
                if let Some(value) = map.values().next() {
                    return value.clone();
                }
            }
        }
        if let Some(value) = normalize_usertag_value(Some(raw_text.as_str())) {
            return value;
        }
    }

    normalize_usertag_value(
        pick_string(
            account.trae_auth_raw.as_ref(),
            &[
                &["account", "userTag"],
                &["userTag"],
                &["callbackQuery", "userTag"],
                &["rawQuery", "userTag"],
                &["data", "userTag"],
            ],
        )
        .as_deref(),
    )
    .or_else(|| {
        normalize_usertag_value(
            pick_string(
                account.trae_server_raw.as_ref(),
                &[&["account", "userTag"], &["userTag"], &["data", "userTag"]],
            )
            .as_deref(),
        )
    })
    .unwrap_or_else(|| "row".to_string())
}

fn merge_usertag_map_for_inject(
    root_obj: &Map<String, Value>,
    user_id: Option<&str>,
    user_tag: &str,
) -> Result<Option<String>, String> {
    let mut map = root_obj
        .get(TRAE_STORAGE_USERTAG_KEY)
        .and_then(|value| value.as_str())
        .and_then(decode_usertag_map)
        .unwrap_or_default();

    if let Some(uid) = user_id.and_then(|value| normalize_non_empty(Some(value))) {
        map.insert(uid, user_tag.to_ascii_lowercase());
    }

    if map.is_empty() {
        return Ok(None);
    }

    let encoded = encode_usertag_map(&map)?;
    Ok(Some(encoded))
}

fn resolve_existing_device_key_storage_id(root_obj: &Map<String, Value>) -> Option<String> {
    for key in root_obj.keys() {
        let Some(device_id) = key.strip_prefix(TRAE_STORAGE_DEVICE_KEY_PREFIX) else {
            continue;
        };
        if let Some(normalized) = normalize_non_empty(Some(device_id)) {
            return Some(normalized);
        }
    }
    None
}

fn resolve_device_id_for_inject(
    root_obj: &Map<String, Value>,
    account: &TraeAccount,
) -> Option<String> {
    pick_string_multi(
        &[
            account.trae_auth_raw.as_ref(),
            account.trae_server_raw.as_ref(),
        ],
        &[
            &["deviceInfo", "DeviceID"],
            &["deviceInfo", "deviceId"],
            &["DeviceID"],
            &["deviceId"],
            &["callbackQuery", "device_id"],
            &["callbackQuery", "x_device_id"],
        ],
    )
    .or_else(|| resolve_existing_device_key_storage_id(root_obj))
}

fn normalize_device_key_pair_value(value: &Value) -> Option<Value> {
    let private_key = pick_string(Some(value), &[&["privateKeyPEM"], &["private_key_pem"]])?;
    let public_key = pick_string(Some(value), &[&["publicKeyPEM"], &["public_key_pem"]])?;
    Some(serde_json::json!({
        "privateKeyPEM": private_key,
        "publicKeyPEM": public_key,
    }))
}

fn resolve_device_key_pair_for_inject(account: &TraeAccount) -> Option<Value> {
    let auth_raw = account.trae_auth_raw.as_ref()?;
    auth_raw
        .get("deviceKeyPair")
        .and_then(normalize_device_key_pair_value)
        .or_else(|| normalize_device_key_pair_value(auth_raw))
}

fn write_device_key_pair_for_inject(
    root_obj: &mut Map<String, Value>,
    account: &TraeAccount,
) -> Result<(), String> {
    let Some(device_key_pair) = resolve_device_key_pair_for_inject(account) else {
        return Ok(());
    };
    let Some(device_id) = resolve_device_id_for_inject(root_obj, account) else {
        return Ok(());
    };
    let storage_key = build_device_key_storage_key(device_id.as_str());
    root_obj.insert(storage_key, to_icube_cipher_string_value(&device_key_pair)?);
    Ok(())
}

fn resolve_storage_keys_for_inject(root_obj: &Map<String, Value>) -> (String, String, String) {
    let provider_id = resolve_storage_provider_id(root_obj);
    (
        build_auth_storage_key(provider_id.as_str()),
        build_server_storage_key(provider_id.as_str()),
        build_entitlement_storage_key(provider_id.as_str()),
    )
}

fn resolve_account_user_id_for_auth_object(
    account: &TraeAccount,
    roots: &[Option<&Value>],
) -> String {
    normalize_non_empty(account.user_id.as_deref())
        .or_else(|| {
            pick_string_multi(
                roots,
                &[&["userId"], &["user_id"], &["uid"], &["UserID"], &["id"]],
            )
        })
        .unwrap_or_default()
}

fn ensure_auth_raw_for_inject(account: &TraeAccount, existing_auth_raw: Option<&Value>) -> Value {
    let auth_raw = account.trae_auth_raw.as_ref();
    let profile_root = profile_payload_root(account.trae_profile_raw.as_ref());
    let server_raw = account.trae_server_raw.as_ref();
    let platform = resolve_account_platform_kind(account);

    let roots = [auth_raw, profile_root, server_raw];
    let user_tag = resolve_user_tag_for_inject(account);

    let user_id = resolve_account_user_id_for_auth_object(account, &roots);

    let username = normalize_non_empty(account.nickname.as_deref())
        .or_else(|| {
            pick_string_multi(
                &roots,
                &[
                    &["ScreenName"],
                    &["nickname"],
                    &["name"],
                    &["displayName"],
                    &["account", "username"],
                ],
            )
        })
        .unwrap_or_else(|| account.email.clone());

    let email = normalize_email(
        pick_string_multi(
            &roots,
            &[
                &["NonPlainTextEmail"],
                &["account", "nonPlainTextEmail"],
                &["account", "email"],
                &["email"],
                &["user", "email"],
            ],
        )
        .as_deref(),
    )
    .unwrap_or_else(|| account.email.clone());

    let avatar_url = pick_string_multi(
        &roots,
        &[&["AvatarUrl"], &["avatar_url"], &["account", "avatar_url"]],
    )
    .unwrap_or_default();
    let description = pick_string_multi(
        &roots,
        &[
            &["Description"],
            &["description"],
            &["account", "description"],
        ],
    )
    .unwrap_or_default();

    let scope = pick_string_multi(&roots, &[&["account", "scope"], &["scope"]])
        .and_then(|value| normalize_non_empty(Some(value.as_str())))
        .map(|value| {
            if value.trim().eq_ignore_ascii_case("trae") {
                "marscode".to_string()
            } else {
                value
            }
        })
        .unwrap_or_else(|| "marscode".to_string());
    let login_scope = pick_string_multi(
        &roots,
        &[
            &["account", "loginScope"],
            &["loginScope"],
            &["callbackQuery", "scope"],
        ],
    )
    .and_then(|value| normalize_non_empty(Some(value.as_str())))
    .unwrap_or_else(|| "trae".to_string());

    let store_country_code = pick_string_multi(
        &roots,
        &[
            &["StoreCountry"],
            &["storeCountry"],
            &["account", "storeCountryCode"],
        ],
    )
    .unwrap_or_default();
    let store_country_src = pick_string_multi(
        &roots,
        &[
            &["StoreCountrySrc"],
            &["storeCountrySrc"],
            &["account", "storeCountrySrc"],
        ],
    )
    .unwrap_or_default();
    let store_region = pick_string_multi(
        &roots,
        &[
            &["account", "storeRegion"],
            &["storeRegion"],
            &["loginRegion"],
            &["callbackQuery", "userRegion"],
            &["userRegion"],
            &["AIRegion"],
        ],
    )
    .map(|value| to_store_region(value.as_str()))
    .unwrap_or_else(|| "UNKNOWN".to_string());

    let ai_region_roots = [profile_root, server_raw, auth_raw];
    let ai_region = pick_string_multi(
        &ai_region_roots,
        &[
            &["AIRegion"],
            &["userRegion", "_aiRegion"],
            &["userRegion", "region"],
            &["callbackQuery", "userRegion"],
            &["loginRegion"],
        ],
    )
    .map(|value| to_store_region(value.as_str()))
    .unwrap_or_else(|| "UNKNOWN".to_string());

    let login_region = normalize_login_region(
        pick_string_multi(
            &roots,
            &[
                &["loginRegion"],
                &["callbackQuery", "userRegion"],
                &["userRegion", "region"],
                &["userRegion", "_aiRegion"],
                &["storeRegion"],
                &["AIRegion"],
            ],
        )
        .as_deref(),
    );

    let api_host = resolve_trae_auth_storage_origin(
        platform,
        pick_string_multi(
            &roots,
            &[
                &["callbackQuery", "host"],
                &["data", "host"],
                &["loginHost"],
                &["host"],
                &["Result", "Host"],
                &["Result", "AIPayHost"],
                &["Result", "AIHost"],
            ],
        )
        .as_deref(),
        Some(store_region.as_str()),
        Some(ai_region.as_str()),
        login_region.as_deref(),
    );
    let client_id = resolve_auth_client_id_from_roots(&roots, platform);

    let expires_at = resolve_iso_timestamp(
        account.expires_at,
        &roots,
        &[
            &["expiredAt"],
            &["expiresAt"],
            &["TokenExpireAt"],
            &["exchangeResponse", "Result", "TokenExpireAt"],
        ],
    )
    .unwrap_or_else(|| {
        (chrono::Utc::now() + chrono::Duration::days(1))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    });

    let refresh_expired_at = resolve_iso_timestamp(
        None,
        &roots,
        &[
            &["refreshExpiredAt"],
            &["RefreshExpireAt"],
            &["Result", "RefreshExpireAt"],
            &["exchangeResponse", "Result", "RefreshExpireAt"],
            &["callbackQuery", "refreshExpireAt"],
        ],
    )
    .unwrap_or_else(|| expires_at.clone());

    let token_release_at = resolve_iso_timestamp(None, &roots, &[&["tokenReleaseAt"]])
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true));

    let refresh_token = normalize_non_empty(account.refresh_token.as_deref())
        .or_else(|| {
            pick_string_multi(
                &roots,
                &[
                    &["refreshToken"],
                    &["refresh_token"],
                    &["RefreshToken"],
                    &["exchangeResponse", "Result", "RefreshToken"],
                ],
            )
        })
        .unwrap_or_default();

    // Merge official auth payloads from both the imported account and current storage so we
    // keep unknown fields across upgrades, switches, and previously "thinned" injections.
    let mut obj = auth_raw
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if let Some(existing_obj) = existing_auth_raw.and_then(Value::as_object) {
        for (key, value) in existing_obj {
            obj.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }
    let had_token_type_key = obj.contains_key("tokenType") || obj.contains_key("token_type");
    let had_region_key = obj.contains_key("region");
    let had_ai_region_key = obj.contains_key("aiRegion");

    let mut account_obj = auth_raw
        .and_then(Value::as_object)
        .and_then(|value| value.get("account"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if let Some(existing_account_obj) = existing_auth_raw
        .and_then(Value::as_object)
        .and_then(|value| value.get("account"))
        .and_then(Value::as_object)
    {
        for (key, value) in existing_account_obj {
            account_obj
                .entry(key.clone())
                .or_insert_with(|| value.clone());
        }
    }

    account_obj.insert("username".to_string(), Value::String(username));
    account_obj.insert("iss".to_string(), Value::String(String::new()));
    account_obj.insert(
        "iat".to_string(),
        Value::Number(serde_json::Number::from(0)),
    );
    account_obj.insert("organization".to_string(), Value::String(String::new()));
    account_obj.insert("work_country".to_string(), Value::String(String::new()));
    account_obj.insert("email".to_string(), Value::String(email));
    account_obj.insert("avatar_url".to_string(), Value::String(avatar_url));
    account_obj.insert("description".to_string(), Value::String(description));
    account_obj.insert("scope".to_string(), Value::String(scope));
    account_obj.insert("loginScope".to_string(), Value::String(login_scope));
    account_obj.insert(
        "storeCountryCode".to_string(),
        Value::String(store_country_code),
    );
    account_obj.insert(
        "storeCountrySrc".to_string(),
        Value::String(store_country_src),
    );
    account_obj.insert("storeRegion".to_string(), Value::String(store_region));
    account_obj.insert("userTag".to_string(), Value::String(user_tag));

    let mut user_region = auth_raw
        .and_then(Value::as_object)
        .and_then(|value| value.get("userRegion"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if let Some(existing_user_region) = existing_auth_raw
        .and_then(Value::as_object)
        .and_then(|value| value.get("userRegion"))
        .and_then(Value::as_object)
    {
        for (key, value) in existing_user_region {
            user_region
                .entry(key.clone())
                .or_insert_with(|| value.clone());
        }
    }
    user_region.insert("region".to_string(), Value::String(ai_region.clone()));
    user_region.insert("_aiRegion".to_string(), Value::String(ai_region.clone()));

    obj.insert(
        "token".to_string(),
        Value::String(account.access_token.clone()),
    );
    obj.insert(
        "accessToken".to_string(),
        Value::String(account.access_token.clone()),
    );
    if !refresh_token.is_empty() {
        obj.insert("refreshToken".to_string(), Value::String(refresh_token));
    }
    if !user_id.is_empty() {
        obj.insert("userId".to_string(), Value::String(user_id));
    }
    if let Some(token_type) = account
        .token_type
        .clone()
        .or_else(|| pick_string_multi(&roots, &[&["tokenType"], &["token_type"]]))
    {
        if had_token_type_key {
            obj.insert("tokenType".to_string(), Value::String(token_type));
        }
    }

    obj.insert("expiredAt".to_string(), Value::String(expires_at));
    obj.insert(
        "refreshExpiredAt".to_string(),
        Value::String(refresh_expired_at),
    );
    obj.insert(
        "tokenReleaseAt".to_string(),
        Value::String(token_release_at),
    );
    obj.insert("host".to_string(), Value::String(api_host.clone()));
    obj.insert("loginHost".to_string(), Value::String(api_host));
    insert_platform_metadata(&mut obj, platform);
    obj.insert("authClientId".to_string(), Value::String(client_id));
    if had_region_key {
        obj.insert("region".to_string(), Value::String(ai_region.clone()));
    }
    if had_ai_region_key {
        obj.insert("aiRegion".to_string(), Value::String(ai_region.clone()));
    }
    obj.insert("userRegion".to_string(), Value::Object(user_region));
    obj.insert("account".to_string(), Value::Object(account_obj));

    Value::Object(obj)
}

fn should_write_server_data_for_inject(value: &Value) -> bool {
    let Some(obj) = value.as_object() else {
        return false;
    };
    obj.contains_key("entitlementInfo")
        || obj.contains_key("serverTimeInfo")
        || obj.contains_key("commercialActivityInfo")
        || obj.contains_key("soloCnInfo")
        || obj.contains_key("saasEntitlementInfo")
}

fn ensure_server_raw_for_inject(account: &TraeAccount) -> Option<Value> {
    let raw = account.trae_server_raw.clone()?;
    if should_write_server_data_for_inject(&raw) {
        return Some(raw);
    }
    None
}

fn ensure_entitlement_raw_for_inject(account: &TraeAccount) -> Option<Value> {
    account.trae_entitlement_raw.clone()
}

fn read_local_trae_auth_from_storage_path(
    storage_path: &Path,
) -> Result<Option<TraeImportPayload>, String> {
    if !storage_path.exists() {
        return Ok(None);
    }
    let storage_root = read_storage_json(storage_path)?;
    let payload = payload_from_storage_root(&storage_root)?;
    Ok(Some(payload))
}

pub fn read_local_trae_auth() -> Result<Option<TraeImportPayload>, String> {
    read_local_trae_auth_for_platform(TraePlatformKind::Trae)
}

pub fn read_local_trae_auth_for_platform(
    platform: TraePlatformKind,
) -> Result<Option<TraeImportPayload>, String> {
    let storage_path = get_default_trae_storage_path_for_platform(platform)?;
    read_local_trae_auth_from_storage_path(&storage_path)
}

pub fn import_from_local() -> Result<Option<TraeAccount>, String> {
    import_from_local_for_platform(TraePlatformKind::Trae)
}

pub fn import_from_local_for_platform(
    platform: TraePlatformKind,
) -> Result<Option<TraeAccount>, String> {
    let mut payload = match read_local_trae_auth_for_platform(platform)? {
        Some(payload) => payload,
        None => return Ok(None),
    };
    attach_platform_metadata_to_payload(&mut payload, platform);
    let account = upsert_account(payload)?;
    logger::log_info(&format!(
        "[Trae Account] 本地导入成功: platform={}, id={}, email={}",
        platform.provider_key(),
        account.id,
        account.email
    ));
    Ok(Some(account))
}

pub(crate) fn resolve_current_account_id(accounts: &[TraeAccount]) -> Option<String> {
    resolve_current_account_id_for_platform(accounts, TraePlatformKind::Trae)
}

pub(crate) fn resolve_current_account_id_for_platform(
    accounts: &[TraeAccount],
    platform: TraePlatformKind,
) -> Option<String> {
    let payload = read_local_trae_auth_for_platform(platform).ok()??;
    let normalized_user_id = normalize_non_empty(payload.user_id.as_deref());
    let normalized_email = normalize_email(Some(payload.email.as_str()));

    accounts
        .iter()
        .find(|account| {
            if resolve_account_platform_kind(account) != platform {
                return false;
            }

            if let (Some(existing), Some(incoming)) = (
                normalize_non_empty(account.user_id.as_deref()),
                normalized_user_id.clone(),
            ) {
                if existing == incoming {
                    return true;
                }
            }

            if let (Some(existing), Some(incoming)) = (
                normalize_email(Some(account.email.as_str())),
                normalized_email.clone(),
            ) {
                return existing == incoming;
            }

            false
        })
        .map(|account| account.id.clone())
}

pub(crate) fn resolve_running_account_refresh_protection_map(
    accounts: &[TraeAccount],
) -> BTreeMap<String, Option<PathBuf>> {
    let mut protected = BTreeMap::new();

    for platform in all_trae_platform_kinds() {
        if crate::modules::process::is_trae_running_for_platform(platform) {
            if let Some(current_id) = resolve_current_account_id_for_platform(accounts, platform) {
                let default_storage_path =
                    get_default_trae_storage_path_for_platform(platform).ok();
                protected.insert(current_id, default_storage_path);
            }
        }
    }

    match crate::modules::trae_instance::resolve_running_bound_account_contexts() {
        Ok(contexts) => {
            for context in contexts {
                let account_id = context.account_id;
                let storage_path = context.storage_path;
                protected
                    .entry(account_id)
                    .and_modify(|current| {
                        if current.is_none() {
                            *current = Some(storage_path.clone());
                        }
                    })
                    .or_insert(Some(storage_path));
            }
        }
        Err(err) => {
            logger::log_warn(&format!(
                "[Trae Refresh] 读取运行中实例绑定账号失败，跳过实例保护名单: {}",
                err
            ));
        }
    }

    protected
}

pub fn inject_to_trae(account_id: &str) -> Result<(), String> {
    inject_to_trae_for_platform(TraePlatformKind::Trae, account_id)
}

pub fn inject_to_trae_for_platform(
    platform: TraePlatformKind,
    account_id: &str,
) -> Result<(), String> {
    let storage_path = get_default_trae_storage_path_for_platform(platform)?;
    inject_to_trae_at_path(storage_path.as_path(), account_id)
}

pub fn inject_to_trae_at_path(storage_path: &Path, account_id: &str) -> Result<(), String> {
    let account =
        load_account(account_id).ok_or_else(|| format!("Trae 账号不存在: {}", account_id))?;
    let mut root = if storage_path.exists() {
        read_storage_json(storage_path)?
    } else {
        Value::Object(Map::new())
    };

    if !root.is_object() {
        root = Value::Object(Map::new());
    }

    let root_obj = root
        .as_object_mut()
        .ok_or_else(|| "Trae storage.json 格式非法".to_string())?;

    let (auth_storage_key, server_storage_key, entitlement_storage_key) =
        resolve_storage_keys_for_inject(root_obj);

    let existing_auth_raw = root_obj
        .get(auth_storage_key.as_str())
        .and_then(|value| parse_value_or_json_string_or_icube_cipher(Some(value)));
    let auth_raw = ensure_auth_raw_for_inject(&account, existing_auth_raw.as_ref());
    root_obj.insert(auth_storage_key, to_icube_cipher_string_value(&auth_raw)?);
    write_device_key_pair_for_inject(root_obj, &account)?;

    if let Some(entitlement_raw) = ensure_entitlement_raw_for_inject(&account) {
        root_obj.insert(
            entitlement_storage_key,
            to_json_string_value(&entitlement_raw)?,
        );
    }

    if let Some(server_raw) = ensure_server_raw_for_inject(&account) {
        root_obj.insert(server_storage_key, to_json_string_value(&server_raw)?);
    }

    let user_tag = resolve_user_tag_for_inject(&account);
    let encoded_usertag_map = merge_usertag_map_for_inject(
        root_obj,
        resolve_account_user_id_for_inject(&account).as_deref(),
        user_tag.as_str(),
    )?;
    if let Some(encoded_map) = encoded_usertag_map {
        root_obj.insert(
            TRAE_STORAGE_USERTAG_KEY.to_string(),
            Value::String(encoded_map),
        );
    } else if let Some(usertag_raw) = normalize_non_empty(account.trae_usertag_raw.as_deref()) {
        root_obj.insert(
            TRAE_STORAGE_USERTAG_KEY.to_string(),
            Value::String(usertag_raw),
        );
    }

    write_storage_json(storage_path, &root)?;

    logger::log_info(&format!(
        "[Trae Account] 注入成功: id={}, email={}, path={}",
        account.id,
        account.email,
        storage_path.display()
    ));
    Ok(())
}

fn extract_response_data(raw: &Value) -> Option<&Value> {
    raw.get("data")
        .or_else(|| raw.get("Result"))
        .or_else(|| raw.get("result"))
        .or_else(|| raw.get("payload"))
}

fn pick_cookie_from_account(account: &TraeAccount) -> Option<String> {
    pick_string(
        account.trae_auth_raw.as_ref(),
        &[
            &["cookie"],
            &["Cookie"],
            &["headers", "cookie"],
            &["headers", "Cookie"],
        ],
    )
}

#[derive(Debug, Clone)]
struct TraeDeviceKeyPair {
    private_key_pem: String,
    public_key_pem: String,
}

fn bytes_to_lower_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{:02x}", byte);
    }
    out
}

fn pem_to_der(pem: &str) -> Result<Vec<u8>, String> {
    let body = pem
        .lines()
        .filter(|line| !line.trim_start().starts_with("-----"))
        .map(str::trim)
        .collect::<String>();
    BASE64_STANDARD
        .decode(body.as_bytes())
        .map_err(|e| format!("解析 Trae 设备私钥 PEM 失败: {}", e))
}

fn resolve_device_key_pair_for_refresh(account: &TraeAccount) -> Option<TraeDeviceKeyPair> {
    let value = resolve_device_key_pair_for_inject(account)?;
    Some(TraeDeviceKeyPair {
        private_key_pem: pick_string(Some(&value), &[&["privateKeyPEM"]])?,
        public_key_pem: pick_string(Some(&value), &[&["publicKeyPEM"]])?,
    })
}

fn resolve_ide_version_for_account(account: &TraeAccount) -> String {
    pick_string_multi(
        &[
            account.trae_auth_raw.as_ref(),
            account.trae_server_raw.as_ref(),
        ],
        &[
            &["deviceInfo", "ClientVersion"],
            &["ClientVersion"],
            &["x_app_version"],
            &["exchangeResponse", "IDEVersion"],
        ],
    )
    .unwrap_or_else(|| TRAE_IDE_VERSION.to_string())
}

fn build_refresh_device_info(account: &TraeAccount, public_key_pem: &str) -> Value {
    let mut device_info = account
        .trae_auth_raw
        .as_ref()
        .and_then(|value| value.get("deviceInfo"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    device_info.insert(
        "DevicePublicKey".to_string(),
        Value::String(public_key_pem.to_string()),
    );
    device_info
        .entry("PlatformCode".to_string())
        .or_insert_with(|| Value::String("IDE_PC".to_string()));
    device_info
        .entry("DeviceType".to_string())
        .or_insert_with(|| Value::String("PC".to_string()));
    device_info
        .entry("ClientVersion".to_string())
        .or_insert_with(|| Value::String(resolve_ide_version_for_account(account)));
    Value::Object(device_info)
}

fn build_device_proof_message(
    client_id: &str,
    refresh_token: &str,
    timestamp: i64,
    nonce: &str,
) -> String {
    let timestamp_text = timestamp.to_string();
    [
        "POST",
        TRAE_AUTH_CODE_EXCHANGE_TOKEN_PATH,
        client_id,
        refresh_token,
        timestamp_text.as_str(),
        nonce,
    ]
    .join("\n")
}

fn sign_trae_device_proof(
    refresh_token: &str,
    private_key_pem: &str,
    client_id: &str,
) -> Result<Value, String> {
    let mut nonce_bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = bytes_to_lower_hex(&nonce_bytes);
    let timestamp = chrono::Utc::now().timestamp();
    let message = build_device_proof_message(client_id, refresh_token, timestamp, nonce.as_str());
    let private_key_der = pem_to_der(private_key_pem)?;
    let rng = SystemRandom::new();
    let key_pair = EcdsaKeyPair::from_pkcs8(
        &ECDSA_P256_SHA256_ASN1_SIGNING,
        private_key_der.as_slice(),
        &rng,
    )
    .map_err(|_| "解析 Trae 设备私钥失败".to_string())?;
    let signature = key_pair
        .sign(&rng, message.as_bytes())
        .map_err(|_| "生成 Trae 设备签名失败".to_string())?;
    Ok(serde_json::json!({
        "Signature": BASE64_STANDARD.encode(signature.as_ref()),
        "Timestamp": timestamp,
        "Nonce": nonce,
    }))
}

async fn request_exchange_token_by_official_refresh(
    client: &reqwest::Client,
    account: &TraeAccount,
    routing_context: &TraeRefreshRoutingContext,
    cookie: Option<&str>,
) -> Result<Value, String> {
    let refresh_token = normalize_non_empty(account.refresh_token.as_deref())
        .ok_or_else(|| "Trae refresh token 缺失，无法按官方流程刷新登录态".to_string())?;
    let device_key_pair = resolve_device_key_pair_for_refresh(account)
        .ok_or_else(|| "Trae 设备密钥缺失，无法按官方新版流程刷新登录态".to_string())?;
    let device_info = build_refresh_device_info(account, device_key_pair.public_key_pem.as_str());
    let client_id = routing_context.client_id.as_str();
    let device_proof = sign_trae_device_proof(
        refresh_token.as_str(),
        device_key_pair.private_key_pem.as_str(),
        client_id,
    )?;
    let body = serde_json::json!({
        "ClientID": client_id,
        "ClientSecret": "",
        "RefreshToken": refresh_token,
        "DeviceInfo": device_info,
        "DeviceProof": device_proof,
        "IDEVersion": resolve_ide_version_for_account(account),
    });
    let urls = build_api_urls(
        routing_context.login_host.as_str(),
        TRAE_AUTH_CODE_EXCHANGE_TOKEN_PATH,
    );
    let response = request_trae_json_with_candidates(
        client,
        Method::POST,
        urls.as_slice(),
        account.access_token.as_str(),
        cookie,
        Some(body),
    )
    .await?;
    let root = extract_response_data(&response).unwrap_or(&response);
    if pick_string(
        Some(root),
        &[&["Token"], &["accessToken"], &["access_token"], &["token"]],
    )
    .is_none()
    {
        return Err("Trae 官方 ExchangeToken 响应缺少 access token".to_string());
    }
    Ok(response)
}

fn normalize_login_region(raw: Option<&str>) -> Option<String> {
    let value = normalize_non_empty(raw)?;
    let normalized = match value.trim().to_ascii_lowercase().as_str() {
        "china-north" => "cn".to_string(),
        "singapore-central" => "sg".to_string(),
        "us-east" | "us-east-1" => "us".to_string(),
        other => other.to_string(),
    };
    Some(normalized)
}

fn build_refresh_routing_context(account: &TraeAccount) -> TraeRefreshRoutingContext {
    let platform = resolve_account_platform_kind(account);
    let profile_root = profile_payload_root(account.trae_profile_raw.as_ref());
    let roots = [
        account.trae_auth_raw.as_ref(),
        profile_root,
        account.trae_server_raw.as_ref(),
        account.trae_entitlement_raw.as_ref(),
        account.trae_usage_raw.as_ref(),
    ];

    let login_region = normalize_login_region(
        pick_string_multi(
            &roots,
            &[
                &["loginRegion"],
                &["callbackQuery", "userRegion"],
                &["userRegion", "region"],
                &["userRegion", "_aiRegion"],
                &["storeRegion"],
                &["AIRegion"],
            ],
        )
        .as_deref(),
    );

    let store_region = pick_string_multi(
        &roots,
        &[
            &["storeRegion"],
            &["account", "storeRegion"],
            &["userRegion", "region"],
            &["callbackQuery", "userRegion"],
            &["AIRegion"],
        ],
    )
    .map(|value| to_store_region(value.as_str()));

    let ai_region = pick_string_multi(
        &roots,
        &[
            &["AIRegion"],
            &["userRegion", "_aiRegion"],
            &["userRegion", "region"],
            &["callbackQuery", "userRegion"],
            &["loginRegion"],
        ],
    )
    .map(|value| to_store_region(value.as_str()));

    let client_id = resolve_auth_client_id_from_roots(&roots, platform);
    let login_host = resolve_trae_account_api_origin(
        platform,
        pick_string_multi(
            &roots,
            &[
                &["loginHost"],
                &["host"],
                &["account", "host"],
                &["callbackQuery", "host"],
                &["data", "host"],
                &["Result", "Host"],
                &["Result", "AIPayHost"],
                &["Result", "AIHost"],
                &["result", "loginHost"],
                &["data", "loginHost"],
                &["exchangeResponse", "Result", "loginHost"],
            ],
        )
        .as_deref(),
        store_region.as_deref(),
        ai_region.as_deref(),
        login_region.as_deref(),
    );

    TraeRefreshRoutingContext {
        platform,
        client_id,
        login_host,
        login_region,
        store_region,
        ai_region,
    }
}

fn build_refresh_api_urls(account: &TraeAccount, path: &str) -> Vec<String> {
    let context = build_refresh_routing_context(account);
    build_api_urls(context.login_host.as_str(), path)
}

fn merge_refresh_routing_context(response: &Value, context: &TraeRefreshRoutingContext) -> Value {
    let Some(response_obj) = response.as_object() else {
        return response.clone();
    };

    let mut merged = response_obj.clone();

    if !context.login_host.is_empty() {
        merged
            .entry("loginHost".to_string())
            .or_insert_with(|| Value::String(context.login_host.clone()));
        merged
            .entry("host".to_string())
            .or_insert_with(|| Value::String(context.login_host.clone()));
    }

    if let Some(login_region) = context.login_region.as_ref() {
        merged
            .entry("loginRegion".to_string())
            .or_insert_with(|| Value::String(login_region.clone()));
    }

    if let Some(store_region) = context.store_region.as_ref() {
        merged
            .entry("storeRegion".to_string())
            .or_insert_with(|| Value::String(store_region.clone()));
    }

    if let Some(ai_region) = context.ai_region.as_ref() {
        merged
            .entry("AIRegion".to_string())
            .or_insert_with(|| Value::String(ai_region.clone()));
    }
    merged
        .entry("authClientId".to_string())
        .or_insert_with(|| Value::String(context.client_id.clone()));

    Value::Object(merged)
}

fn merge_exchange_auth_raw(
    existing_auth_raw: Option<&Value>,
    exchange_response: &Value,
    context: &TraeRefreshRoutingContext,
    access_token: &str,
    refresh_token: Option<&str>,
    token_type: Option<&str>,
    expires_at: Option<i64>,
) -> Value {
    let mut merged = match existing_auth_raw {
        Some(Value::Object(obj)) => obj.clone(),
        _ => Map::new(),
    };

    merged.insert("exchangeResponse".to_string(), exchange_response.clone());
    merged.insert("token".to_string(), Value::String(access_token.to_string()));
    merged.insert(
        "accessToken".to_string(),
        Value::String(access_token.to_string()),
    );

    if let Some(refresh) = normalize_non_empty(refresh_token) {
        merged.insert("refreshToken".to_string(), Value::String(refresh));
    }

    if let Some(kind) = normalize_non_empty(token_type) {
        merged.insert("tokenType".to_string(), Value::String(kind));
    }

    let response_roots = [Some(exchange_response)];

    if let Some(expired_at) = resolve_iso_timestamp(
        expires_at,
        &response_roots,
        &[
            &["expiredAt"],
            &["expiresAt"],
            &["TokenExpireAt"],
            &["Result", "TokenExpireAt"],
        ],
    ) {
        merged.insert("expiredAt".to_string(), Value::String(expired_at));
    }

    if let Some(refresh_expired_at) = resolve_iso_timestamp(
        None,
        &response_roots,
        &[
            &["refreshExpiredAt"],
            &["RefreshExpireAt"],
            &["Result", "RefreshExpireAt"],
        ],
    ) {
        merged.insert(
            "refreshExpiredAt".to_string(),
            Value::String(refresh_expired_at),
        );
    }

    if let Some(token_release_at) =
        resolve_iso_timestamp(None, &response_roots, &[&["tokenReleaseAt"]])
    {
        merged.insert(
            "tokenReleaseAt".to_string(),
            Value::String(token_release_at),
        );
    }

    if !context.login_host.is_empty() {
        merged
            .entry("host".to_string())
            .or_insert_with(|| Value::String(context.login_host.clone()));
        merged
            .entry("loginHost".to_string())
            .or_insert_with(|| Value::String(context.login_host.clone()));
    }

    if let Some(login_region) = context.login_region.as_ref() {
        merged
            .entry("loginRegion".to_string())
            .or_insert_with(|| Value::String(login_region.clone()));
    }

    if let Some(store_region) = context.store_region.as_ref() {
        merged
            .entry("storeRegion".to_string())
            .or_insert_with(|| Value::String(store_region.clone()));
    }

    if let Some(ai_region) = context.ai_region.as_ref() {
        merged
            .entry("AIRegion".to_string())
            .or_insert_with(|| Value::String(ai_region.clone()));
    }

    Value::Object(merged)
}

fn header_value_or_dash(headers: &reqwest::header::HeaderMap, key: &str) -> String {
    headers
        .get(key)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string())
}

fn build_body_preview(body_text: &str, max_chars: usize) -> String {
    let mut preview = String::new();
    let mut count = 0usize;
    for ch in body_text.chars() {
        if count >= max_chars {
            preview.push_str("...[truncated]");
            break;
        }
        match ch {
            '\n' => preview.push_str("\\n"),
            '\r' => preview.push_str("\\r"),
            '\t' => preview.push_str("\\t"),
            _ => preview.push(ch),
        }
        count += 1;
    }
    if preview.is_empty() {
        "<empty>".to_string()
    } else {
        preview
    }
}

async fn parse_trae_response_body(response: reqwest::Response, url: &str) -> Result<Value, String> {
    let status = response.status();
    let status_code = status.as_u16();
    if status_code == 401 || status_code == 403 {
        return Err("Trae 会话已过期或未认证，请重新登录".to_string());
    }

    let headers = response.headers();
    let content_type = headers
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("-")
        .to_string();
    let x_request_id = header_value_or_dash(headers, "x-request-id");
    let request_id = header_value_or_dash(headers, "request-id");
    let cf_ray = header_value_or_dash(headers, "cf-ray");

    let body_text = response
        .text()
        .await
        .map_err(|e| format!("读取 Trae 响应失败({}): {}", url, e))?;
    let body_trimmed = body_text.trim();
    if body_trimmed.is_empty() {
        return Ok(Value::Object(Map::new()));
    }

    serde_json::from_str::<Value>(&body_text).map_err(|e| {
        let body_preview = build_body_preview(body_trimmed, 200);
        format!(
            "解析 Trae 响应 JSON 失败({}): {} | status={} | content-type={} | x-request-id={} | request-id={} | cf-ray={} | body_preview={}",
            url,
            e,
            status_code,
            content_type,
            x_request_id,
            request_id,
            cf_ray,
            body_preview
        )
    })
}

async fn request_trae_json(
    client: &reqwest::Client,
    method: Method,
    url: &str,
    access_token: &str,
    cookie: Option<&str>,
    body: Option<Value>,
) -> Result<Value, String> {
    let mut request = client
        .request(method, url)
        .header("Accept", "application/json")
        .header("User-Agent", "Trae/1.0.0 antigravity-cockpit-tools")
        .header("Authorization", format!("Bearer {}", access_token))
        .header("x-cloudide-token", access_token);

    if let Some(cookie_header) = cookie.and_then(|value| normalize_non_empty(Some(value))) {
        request = request.header("Cookie", cookie_header);
    }
    if let Some(payload) = body {
        request = request
            .header("Content-Type", "application/json")
            .json(&payload);
    }

    let response = request
        .send()
        .await
        .map_err(|e| format!("请求 Trae 接口失败({}): {}", url, e))?;

    parse_trae_response_body(response, url).await
}

async fn request_trae_json_with_candidates(
    client: &reqwest::Client,
    method: Method,
    urls: &[String],
    access_token: &str,
    cookie: Option<&str>,
    body: Option<Value>,
) -> Result<Value, String> {
    let mut errors = Vec::new();
    for url in urls {
        match request_trae_json(
            client,
            method.clone(),
            url.as_str(),
            access_token,
            cookie,
            body.clone(),
        )
        .await
        {
            Ok(response) => return Ok(response),
            Err(err) => errors.push(format!("{} => {}", url, err)),
        }
    }

    if errors.is_empty() {
        return Err("Trae 请求地址为空".to_string());
    }

    Err(errors.join(" | "))
}

async fn request_trae_pay_json(
    client: &reqwest::Client,
    method: Method,
    url: &str,
    access_token: &str,
    cookie: Option<&str>,
    body: Option<Value>,
) -> Result<Value, String> {
    let mut request = client
        .request(method, url)
        .header("Accept", "application/json")
        .header("User-Agent", "Trae/1.0.0 antigravity-cockpit-tools")
        .header("Authorization", format!("Cloud-IDE-JWT {}", access_token));

    if let Some(cookie_header) = cookie.and_then(|value| normalize_non_empty(Some(value))) {
        request = request.header("Cookie", cookie_header);
    }
    if let Some(payload) = body {
        request = request
            .header("Content-Type", "application/json")
            .json(&payload);
    }

    let response = request
        .send()
        .await
        .map_err(|e| format!("请求 Trae 接口失败({}): {}", url, e))?;

    parse_trae_response_body(response, url).await
}

async fn request_trae_pay_json_with_candidates(
    client: &reqwest::Client,
    method: Method,
    urls: &[String],
    access_token: &str,
    cookie: Option<&str>,
    body: Option<Value>,
) -> Result<Value, String> {
    let mut errors = Vec::new();
    for url in urls {
        match request_trae_pay_json(
            client,
            method.clone(),
            url.as_str(),
            access_token,
            cookie,
            body.clone(),
        )
        .await
        {
            Ok(response) => return Ok(response),
            Err(err) => errors.push(format!("{} => {}", url, err)),
        }
    }

    if errors.is_empty() {
        return Err("Trae 请求地址为空".to_string());
    }

    Err(errors.join(" | "))
}

fn apply_profile_response(account: &mut TraeAccount, response: &Value) {
    let profile_root = extract_response_data(response).unwrap_or(response);
    account.trae_profile_raw = Some(response.clone());

    if let Some(email) = normalize_email(
        pick_string(
            Some(profile_root),
            &[
                &["NonPlainTextEmail"],
                &["Email"],
                &["email"],
                &["user", "email"],
                &["userInfo", "email"],
                &["profile", "email"],
            ],
        )
        .as_deref(),
    ) {
        account.email = email;
    }

    if let Some(user_id) = normalize_non_empty(
        pick_string(
            Some(profile_root),
            &[
                &["UserID"],
                &["userId"],
                &["user_id"],
                &["uid"],
                &["id"],
                &["user", "id"],
            ],
        )
        .as_deref(),
    ) {
        account.user_id = Some(user_id);
    }

    if let Some(nickname) = normalize_non_empty(
        pick_string(
            Some(profile_root),
            &[
                &["ScreenName"],
                &["Nickname"],
                &["nickname"],
                &["name"],
                &["displayName"],
                &["user", "name"],
            ],
        )
        .as_deref(),
    ) {
        account.nickname = Some(nickname);
    }
}

fn usage_identity_from_product_type(product_type: i64) -> Option<&'static str> {
    match product_type {
        6 => Some("Ultra"),
        4 => Some("Pro+"),
        1 => Some("Pro"),
        9 => Some("Pro"),
        8 => Some("Lite"),
        0 => Some("Free"),
        _ => None,
    }
}

fn usage_pack_product_type(pack: &Value) -> Option<i64> {
    pick_i64(
        Some(pack),
        &[
            &["entitlement_base_info", "product_type"],
            &["product_type"],
        ],
    )
}

fn apply_entitlement_response(account: &mut TraeAccount, response: &Value) {
    if let Some(code) = pick_i64(Some(response), &[&["code"]]) {
        if code != 0 {
            return;
        }
    }

    account.trae_entitlement_raw = Some(response.clone());

    if let Some(plan_type) =
        normalize_non_empty(pick_string(Some(response), &[&["user_pay_identity_str"]]).as_deref())
    {
        account.plan_type = Some(plan_type);
    }

    account.plan_reset_at = normalize_timestamp(pick_i64(
        Some(response),
        &[&["detail", "subscription_renew_time"]],
    ));
}

fn apply_usage_response(account: &mut TraeAccount, response: &Value) {
    if let Some(code) = pick_i64(Some(response), &[&["code"]]) {
        if code != 0 {
            return;
        }
    }

    account.trae_usage_raw = Some(response.clone());

    if let Some(pack_list) = response
        .get("user_entitlement_pack_list")
        .and_then(|value| value.as_array())
    {
        let filtered_packs: Vec<&Value> = pack_list
            .iter()
            .filter(|pack| usage_pack_product_type(pack) != Some(3))
            .collect();

        let find_pack = |product_type: i64| {
            filtered_packs
                .iter()
                .copied()
                .find(|pack| usage_pack_product_type(pack) == Some(product_type))
        };

        let pack = find_pack(6)
            .or_else(|| find_pack(4))
            .or_else(|| find_pack(1))
            .or_else(|| find_pack(9))
            .or_else(|| find_pack(8))
            .or_else(|| find_pack(0));
        if let Some(pack) = pack {
            if let Some(product_type) = usage_pack_product_type(pack) {
                if let Some(identity) = usage_identity_from_product_type(product_type) {
                    account.plan_type = Some(identity.to_string());
                }
            }

            let reset_at = pick_i64(Some(pack), &[&["entitlement_base_info", "end_time"]])
                .and_then(|value| if value > 0 { Some(value + 1) } else { None });

            account.plan_reset_at = normalize_timestamp(reset_at);
        }
    }
}

fn apply_exchange_response(
    account: &mut TraeAccount,
    response: &Value,
    context: &TraeRefreshRoutingContext,
) {
    let merged_response = merge_refresh_routing_context(response, context);
    let exchange_root = extract_response_data(&merged_response).unwrap_or(&merged_response);

    let access_token = pick_string(
        Some(exchange_root),
        &[&["Token"], &["accessToken"], &["access_token"], &["token"]],
    );
    let refresh_token = pick_string(
        Some(exchange_root),
        &[&["RefreshToken"], &["refreshToken"], &["refresh_token"]],
    );
    let token_type = pick_string(
        Some(exchange_root),
        &[&["TokenType"], &["tokenType"], &["token_type"]],
    );
    let expires_at = normalize_timestamp(pick_i64(
        Some(exchange_root),
        &[
            &["TokenExpireAt"],
            &["expiresAt"],
            &["expires_at"],
            &["expired_at"],
        ],
    ));

    if let Some(token) = access_token {
        account.access_token = token;
    }
    if let Some(refresh) = refresh_token {
        account.refresh_token = Some(refresh);
    }
    if let Some(kind) = token_type {
        account.token_type = Some(kind);
    }
    if expires_at.is_some() {
        account.expires_at = expires_at;
    }

    account.trae_auth_raw = Some(merge_exchange_auth_raw(
        account.trae_auth_raw.as_ref(),
        &merged_response,
        context,
        account.access_token.as_str(),
        account.refresh_token.as_deref(),
        account.token_type.as_deref(),
        account.expires_at,
    ));
}

fn apply_check_login_response(
    account: &mut TraeAccount,
    response: &Value,
    context: &TraeRefreshRoutingContext,
) {
    let merged_response = merge_refresh_routing_context(response, context);
    let root = extract_response_data(&merged_response).unwrap_or(&merged_response);

    if let Some(status) = normalize_non_empty(
        pick_string(
            Some(root),
            &[&["status"], &["loginStatus"], &["authStatus"], &["result"]],
        )
        .as_deref(),
    ) {
        account.status = Some(status);
    }
    if let Some(reason) = normalize_non_empty(
        pick_string(
            Some(root),
            &[
                &["statusReason"],
                &["status_reason"],
                &["message"],
                &["error"],
            ],
        )
        .as_deref(),
    ) {
        account.status_reason = Some(reason);
    }

    account.trae_server_raw = Some(merged_response);
}

async fn refresh_account_async_once(account_id: &str) -> Result<TraeAccount, String> {
    let existing = load_account(account_id).ok_or_else(|| "账号不存在".to_string())?;
    logger::log_info(&format!(
        "[Trae Refresh] 开始刷新账号: id={}, email={}",
        existing.id, existing.email
    ));

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

    let mut account = existing.clone();

    let cookie = pick_cookie_from_account(&account);
    let routing_context = build_refresh_routing_context(&account);
    logger::log_info(&format!(
        "[Trae Refresh] 使用路由: id={}, platform={}, host={}, login_region={}, store_region={}, ai_region={}",
        account.id,
        routing_context.platform.provider_key(),
        routing_context.login_host,
        routing_context.login_region.as_deref().unwrap_or("-"),
        routing_context.store_region.as_deref().unwrap_or("-"),
        routing_context.ai_region.as_deref().unwrap_or("-")
    ));

    if normalize_non_empty(account.refresh_token.as_deref()).is_none() {
        return Err("Trae refresh token 缺失，无法按官方流程刷新登录态".to_string());
    }

    let exchange_response = match request_exchange_token_by_official_refresh(
        &client,
        &account,
        &routing_context,
        cookie.as_deref(),
    )
    .await
    {
        Ok(response) => response,
        Err(official_err) => {
            logger::log_warn(&format!(
                "[Trae Refresh] 官方新版 ExchangeToken 失败，尝试旧接口 fallback: {}",
                official_err
            ));
            let exchange_body = serde_json::json!({
                "ClientID": routing_context.client_id.as_str(),
                "RefreshToken": account.refresh_token.clone().unwrap_or_default(),
                "ClientSecret": TRAE_EXCHANGE_CLIENT_SECRET,
                "UserID": "",
                "refreshToken": account.refresh_token.clone().unwrap_or_default(),
                "refresh_token": account.refresh_token.clone().unwrap_or_default(),
                "token": account.access_token.clone(),
            });
            let exchange_urls = build_refresh_api_urls(&account, TRAE_EXCHANGE_TOKEN_PATH);
            request_trae_json_with_candidates(
                &client,
                Method::POST,
                exchange_urls.as_slice(),
                &account.access_token,
                cookie.as_deref(),
                Some(exchange_body),
            )
            .await
            .map_err(|err| {
                format!(
                    "Trae ExchangeToken 失败: official={} | legacy={}",
                    official_err, err
                )
            })?
        }
    };
    let exchange_context = build_refresh_routing_context(&account);
    apply_exchange_response(&mut account, &exchange_response, &exchange_context);

    let profile_urls = build_refresh_api_urls(&account, TRAE_GET_USER_INFO_PATH);
    match request_trae_json_with_candidates(
        &client,
        Method::POST,
        profile_urls.as_slice(),
        &account.access_token,
        cookie.as_deref(),
        Some(serde_json::json!({})),
    )
    .await
    {
        Ok(response) => apply_profile_response(&mut account, &response),
        Err(err) => logger::log_warn(&format!("[Trae Refresh] GetUserInfo 失败: {}", err)),
    }

    let check_login_urls = build_refresh_api_urls(&account, TRAE_CHECK_LOGIN_PATH);
    match request_trae_json_with_candidates(
        &client,
        Method::POST,
        check_login_urls.as_slice(),
        &account.access_token,
        cookie.as_deref(),
        Some(serde_json::json!({
            "IDEVersion": TRAE_IDE_VERSION,
        })),
    )
    .await
    {
        Ok(response) => {
            let check_login_context = build_refresh_routing_context(&account);
            apply_check_login_response(&mut account, &response, &check_login_context);
        }
        Err(err) => logger::log_warn(&format!("[Trae Refresh] CheckLogin 失败: {}", err)),
    }

    let entitlement_urls = build_refresh_api_urls(&account, TRAE_PAY_STATUS_PATH);
    let entitlement_response = request_trae_pay_json_with_candidates(
        &client,
        Method::POST,
        entitlement_urls.as_slice(),
        &account.access_token,
        cookie.as_deref(),
        Some(serde_json::json!({})),
    )
    .await;

    let mut quota_query_errors: Vec<String> = Vec::new();
    match entitlement_response {
        Ok(response) => apply_entitlement_response(&mut account, &response),
        Err(err) => {
            logger::log_warn(&format!("[Trae Refresh] ide_user_pay_status 失败: {}", err));
            quota_query_errors.push(err);
        }
    }

    let usage_urls = build_refresh_api_urls(&account, TRAE_ENT_USAGE_PATH);
    let usage_response = request_trae_pay_json_with_candidates(
        &client,
        Method::POST,
        usage_urls.as_slice(),
        &account.access_token,
        cookie.as_deref(),
        Some(serde_json::json!({
            "require_usage": true,
        })),
    )
    .await;

    let mut usage_refreshed = false;
    match usage_response {
        Ok(response) => {
            apply_usage_response(&mut account, &response);
            usage_refreshed = true;
        }
        Err(err) => {
            logger::log_warn(&format!("[Trae Refresh] ide_user_ent_usage 失败: {}", err));
            quota_query_errors.push(err);
        }
    }

    let refreshed_at = now_ts();
    if usage_refreshed {
        account.quota_query_last_error = None;
        account.quota_query_last_error_at = None;
        account.usage_updated_at = Some(refreshed_at);
    } else if !quota_query_errors.is_empty() {
        account.quota_query_last_error = Some(quota_query_errors.join(" | "));
        account.quota_query_last_error_at = Some(chrono::Utc::now().timestamp_millis());
    }
    account.last_used = refreshed_at;
    let updated = account.clone();
    upsert_account_record(account)?;
    logger::log_info(&format!(
        "[Trae Refresh] 刷新完成: id={}, email={}",
        updated.id, updated.email
    ));
    Ok(updated)
}

pub async fn refresh_account_async(account_id: &str) -> Result<TraeAccount, String> {
    let result = refresh_account_async_once(account_id).await;
    if let Err(err) = &result {
        persist_quota_query_error(account_id, err);
    }
    result
}

fn apply_runtime_storage_payload_for_usage_refresh(
    account: &mut TraeAccount,
    runtime_storage_path: Option<&Path>,
) {
    let Some(storage_path) = runtime_storage_path else {
        return;
    };

    let payload = match read_local_trae_auth_from_storage_path(storage_path) {
        Ok(Some(payload)) => payload,
        Ok(None) => return,
        Err(err) => {
            logger::log_warn(&format!(
                "[Trae Refresh] 读取运行中实例 storage 失败，跳过本地会话同步: path={}, error={}",
                storage_path.display(),
                err
            ));
            return;
        }
    };

    let payload_user_id = normalize_non_empty(payload.user_id.as_deref());
    let payload_email = normalize_identity_email(Some(payload.email.as_str()));
    if !account_matches_import_identity(
        account,
        payload_user_id.as_deref(),
        payload_email.as_deref(),
    ) {
        logger::log_warn(&format!(
            "[Trae Refresh] 运行中实例 storage 与目标账号不匹配，跳过本地会话同步: account_id={}, path={}",
            account.id,
            storage_path.display()
        ));
        return;
    }

    let previous_access_token = account.access_token.clone();
    apply_payload(account, payload);
    logger::log_info(&format!(
        "[Trae Refresh] 已同步运行中实例会话快照: account_id={}, path={}, token_changed={}",
        account.id,
        storage_path.display(),
        if previous_access_token == account.access_token {
            "false"
        } else {
            "true"
        }
    ));
}

async fn refresh_account_usage_only_async_once(
    account_id: &str,
    runtime_storage_path: Option<&Path>,
) -> Result<TraeAccount, String> {
    let existing = load_account(account_id).ok_or_else(|| "账号不存在".to_string())?;
    logger::log_info(&format!(
        "[Trae Refresh] 开始仅额度刷新: id={}, email={}",
        existing.id, existing.email
    ));

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

    let mut account = existing.clone();
    apply_runtime_storage_payload_for_usage_refresh(&mut account, runtime_storage_path);

    let cookie = pick_cookie_from_account(&account);
    let routing_context = build_refresh_routing_context(&account);
    logger::log_info(&format!(
        "[Trae Refresh] 仅额度刷新使用路由: id={}, platform={}, host={}, login_region={}, store_region={}, ai_region={}",
        account.id,
        routing_context.platform.provider_key(),
        routing_context.login_host,
        routing_context.login_region.as_deref().unwrap_or("-"),
        routing_context.store_region.as_deref().unwrap_or("-"),
        routing_context.ai_region.as_deref().unwrap_or("-")
    ));

    let entitlement_urls = build_refresh_api_urls(&account, TRAE_PAY_STATUS_PATH);
    let entitlement_response = request_trae_pay_json_with_candidates(
        &client,
        Method::POST,
        entitlement_urls.as_slice(),
        &account.access_token,
        cookie.as_deref(),
        Some(serde_json::json!({})),
    )
    .await;

    let mut quota_query_errors: Vec<String> = Vec::new();
    match entitlement_response {
        Ok(response) => apply_entitlement_response(&mut account, &response),
        Err(err) => {
            logger::log_warn(&format!("[Trae Refresh] ide_user_pay_status 失败: {}", err));
            quota_query_errors.push(err);
        }
    }

    let usage_urls = build_refresh_api_urls(&account, TRAE_ENT_USAGE_PATH);
    let usage_response = request_trae_pay_json_with_candidates(
        &client,
        Method::POST,
        usage_urls.as_slice(),
        &account.access_token,
        cookie.as_deref(),
        Some(serde_json::json!({
            "require_usage": true,
        })),
    )
    .await;

    let mut usage_refreshed = false;
    match usage_response {
        Ok(response) => {
            apply_usage_response(&mut account, &response);
            usage_refreshed = true;
        }
        Err(err) => {
            logger::log_warn(&format!("[Trae Refresh] ide_user_ent_usage 失败: {}", err));
            quota_query_errors.push(err);
        }
    }

    let refreshed_at = now_ts();
    if usage_refreshed {
        account.quota_query_last_error = None;
        account.quota_query_last_error_at = None;
        account.usage_updated_at = Some(refreshed_at);
    } else if !quota_query_errors.is_empty() {
        account.quota_query_last_error = Some(quota_query_errors.join(" | "));
        account.quota_query_last_error_at = Some(chrono::Utc::now().timestamp_millis());
    }
    account.last_used = refreshed_at;

    let updated = account.clone();
    upsert_account_record(account)?;
    logger::log_info(&format!(
        "[Trae Refresh] 仅额度刷新完成: id={}, email={}",
        updated.id, updated.email
    ));
    Ok(updated)
}

pub async fn refresh_account_usage_only_async(
    account_id: &str,
    runtime_storage_path: Option<&Path>,
) -> Result<TraeAccount, String> {
    let result = refresh_account_usage_only_async_once(account_id, runtime_storage_path).await;
    if let Err(err) = &result {
        persist_quota_query_error(account_id, err);
    }
    result
}

async fn refresh_accounts(
    accounts: Vec<TraeAccount>,
) -> Result<Vec<(String, Result<TraeAccount, String>)>, String> {
    let protection_map = resolve_running_account_refresh_protection_map(&accounts);
    let mut results = Vec::with_capacity(accounts.len());
    for account in accounts {
        let account_id = account.id.clone();
        if let Some(storage_path) = protection_map.get(account_id.as_str()) {
            logger::log_info(&format!(
                "[Trae Refresh] 运行中实例账号走仅额度刷新: account_id={}, storage_path={}",
                account_id,
                storage_path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "-".to_string())
            ));
            let result =
                refresh_account_usage_only_async(account_id.as_str(), storage_path.as_deref())
                    .await;
            results.push((account_id, result));
            continue;
        }
        let result = refresh_account_async(account_id.as_str()).await;
        results.push((account_id, result));
    }
    Ok(results)
}

pub async fn refresh_all_tokens() -> Result<Vec<(String, Result<TraeAccount, String>)>, String> {
    refresh_accounts(list_accounts_checked()?).await
}

pub async fn refresh_tokens_for_platform(
    platform: TraePlatformKind,
) -> Result<Vec<(String, Result<TraeAccount, String>)>, String> {
    let accounts = list_accounts_checked()?
        .into_iter()
        .filter(|account| resolve_account_platform_kind(account) == platform)
        .collect();
    refresh_accounts(accounts).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::trae::TraeAccount;

    fn sample_account() -> TraeAccount {
        TraeAccount {
            id: "trae_test".to_string(),
            email: "lijie769328281@gmail.com".to_string(),
            user_id: Some("7463021402682639361".to_string()),
            nickname: Some("李杰".to_string()),
            tags: None,
            access_token: "old-access".to_string(),
            refresh_token: Some("old-refresh".to_string()),
            token_type: Some("Bearer".to_string()),
            expires_at: Some(1_777_220_302),
            plan_type: None,
            plan_reset_at: None,
            trae_auth_raw: None,
            trae_profile_raw: Some(serde_json::json!({
                "Result": {
                    "ScreenName": "李杰",
                    "NonPlainTextEmail": "lijie769328281@gmail.com",
                    "UserID": "7463021402682639361",
                    "AvatarUrl": "https://example.com/avatar.png",
                    "Description": "",
                    "StoreCountry": "jp",
                    "StoreCountrySrc": "uid",
                    "AIRegion": "SG",
                }
            })),
            trae_entitlement_raw: None,
            trae_usage_raw: None,
            trae_server_raw: None,
            trae_usertag_raw: Some("row".to_string()),
            status: None,
            status_reason: None,
            quota_query_last_error: None,
            quota_query_last_error_at: None,
            usage_updated_at: None,
            created_at: 0,
            last_used: 0,
        }
    }

    #[test]
    fn apply_exchange_response_preserves_existing_auth_context() {
        let mut account = sample_account();
        account.trae_auth_raw = Some(serde_json::json!({
            "host": "https://api-sg-central.trae.ai",
            "loginHost": "https://api-sg-central.trae.ai",
            "refreshExpiredAt": "2026-10-09T16:18:22.466Z",
            "tokenReleaseAt": "2026-04-12T16:18:25.030Z",
            "account": {
                "username": "李杰"
            }
        }));

        let response = serde_json::json!({
            "Result": {
                "Token": "new-access",
                "RefreshToken": "new-refresh",
                "TokenType": "Bearer",
                "TokenExpireAt": 1777220302466_u64,
                "RefreshExpireAt": 1791562702466_u64
            }
        });
        let context = TraeRefreshRoutingContext {
            platform: TraePlatformKind::Trae,
            client_id: TRAE_AUTH_CLIENT_ID.to_string(),
            login_host: "https://growsg-normal.trae.ai".to_string(),
            login_region: Some("sg".to_string()),
            store_region: Some("SG".to_string()),
            ai_region: Some("SG".to_string()),
        };

        apply_exchange_response(&mut account, &response, &context);

        let auth_raw = account
            .trae_auth_raw
            .as_ref()
            .and_then(Value::as_object)
            .expect("auth raw should be object");

        assert_eq!(account.access_token, "new-access");
        assert_eq!(account.refresh_token.as_deref(), Some("new-refresh"));
        assert_eq!(
            auth_raw.get("host").and_then(Value::as_str),
            Some("https://api-sg-central.trae.ai")
        );
        assert_eq!(
            auth_raw.get("refreshExpiredAt").and_then(Value::as_str),
            Some("2026-10-09T16:18:22.466Z")
        );
        assert_eq!(
            auth_raw
                .get("exchangeResponse")
                .and_then(|value| value.get("Result"))
                .and_then(|value| value.get("RefreshExpireAt"))
                .and_then(Value::as_u64),
            Some(1791562702466_u64)
        );
        assert_eq!(
            auth_raw
                .get("account")
                .and_then(|value| value.get("username"))
                .and_then(Value::as_str),
            Some("李杰")
        );
    }

    #[test]
    fn product_auth_client_id_uses_platform_and_quality() {
        let root = serde_json::json!({
            "quality": "stable",
            "iCubeApp": {
                "authConfig": {
                    "TRAE": {
                        "stable": "trae-stable-client"
                    },
                    "SOLO": {
                        "stable": "solo-stable-client"
                    }
                }
            }
        });

        assert_eq!(
            read_product_auth_client_id(&root, TraePlatformKind::Trae).as_deref(),
            Some("trae-stable-client")
        );
        assert_eq!(
            read_product_auth_client_id(&root, TraePlatformKind::TraeSolo).as_deref(),
            Some("solo-stable-client")
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_uninstall_display_name_matching_is_platform_scoped() {
        assert!(windows_uninstall_display_name_matches(
            TraePlatformKind::Trae,
            "Trae (User)"
        ));
        assert!(!windows_uninstall_display_name_matches(
            TraePlatformKind::Trae,
            "Trae CN (User)"
        ));
        assert!(windows_uninstall_display_name_matches(
            TraePlatformKind::TraeCn,
            "Trae CN (User)"
        ));
        assert!(windows_uninstall_display_name_matches(
            TraePlatformKind::TraeSoloCn,
            "TRAE Work CN (User)"
        ));
        assert!(windows_uninstall_display_name_matches(
            TraePlatformKind::TraeSoloCn,
            "TRAE SOLO CN"
        ));
        assert!(!windows_uninstall_display_name_matches(
            TraePlatformKind::TraeSolo,
            "TRAE Work CN (User)"
        ));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_registry_path_normalization_handles_icons_and_install_dirs() {
        assert_eq!(
            normalize_windows_registry_path("\"D:\\Apps\\Trae CN\\Trae CN.exe\",0"),
            Some(PathBuf::from("D:\\Apps\\Trae CN\\Trae CN.exe"))
        );
        assert_eq!(
            normalize_windows_registry_path("D:\\Apps\\TRAE SOLO CN\\"),
            Some(PathBuf::from("D:\\Apps\\TRAE SOLO CN\\"))
        );
    }

    #[test]
    fn refresh_routing_context_prefers_stored_dynamic_auth_client_id() {
        let mut account = sample_account();
        account.trae_auth_raw = Some(serde_json::json!({
            "platformId": "trae_solo",
            "authClientId": TRAE_SOLO_AUTH_CLIENT_ID
        }));
        account.trae_server_raw = Some(serde_json::json!({
            "platform": {
                "authClientId": "solo-dynamic-client"
            }
        }));

        let context = build_refresh_routing_context(&account);

        assert_eq!(context.platform, TraePlatformKind::TraeSolo);
        assert_eq!(context.client_id, "solo-dynamic-client");
    }

    #[test]
    fn platform_scoped_payload_identity_keeps_cn_and_global_accounts_separate() {
        let global_payload = TraeImportPayload {
            email: "same@example.com".to_string(),
            user_id: Some("user-1".to_string()),
            nickname: None,
            access_token: "access-global".to_string(),
            refresh_token: Some("refresh-global".to_string()),
            token_type: None,
            expires_at: None,
            plan_type: None,
            plan_reset_at: None,
            trae_auth_raw: Some(serde_json::json!({
                "platformId": "trae",
                "authDomain": "www.trae.ai",
                "authClientId": TRAE_AUTH_CLIENT_ID
            })),
            trae_profile_raw: None,
            trae_entitlement_raw: None,
            trae_usage_raw: None,
            trae_server_raw: None,
            trae_usertag_raw: None,
            status: None,
            status_reason: None,
        };
        let cn_payload = TraeImportPayload {
            trae_auth_raw: Some(serde_json::json!({
                "platformId": "trae_cn",
                "authDomain": "www.trae.cn",
                "authClientId": TRAE_AUTH_CLIENT_ID
            })),
            access_token: "access-cn".to_string(),
            refresh_token: Some("refresh-cn".to_string()),
            ..global_payload.clone()
        };

        let global_platform = resolve_payload_platform_kind(&global_payload);
        let cn_platform = resolve_payload_platform_kind(&cn_payload);

        assert_eq!(global_platform, TraePlatformKind::Trae);
        assert_eq!(cn_platform, TraePlatformKind::TraeCn);
        assert_ne!(
            resolve_platform_scoped_payload_identity(global_platform, &global_payload),
            resolve_platform_scoped_payload_identity(cn_platform, &cn_payload)
        );
    }

    #[test]
    fn ensure_auth_raw_for_inject_preserves_unknown_fields_from_existing_payload() {
        let mut account = sample_account();
        account.nickname = Some("tester".to_string());
        account.trae_auth_raw = Some(serde_json::json!({
            "strictToken": "from-source",
            "account": {
                "sourceBadge": "raw",
                "username": "raw-name"
            },
            "userRegion": {
                "sourceRegionMeta": "raw",
                "_aiRegion": "US",
                "region": "US"
            }
        }));
        let existing = serde_json::json!({
            "existingOnly": "keep-me",
            "tokenType": "Bearer",
            "account": {
                "tenantId": "tenant-123",
                "username": "old-name"
            },
            "userRegion": {
                "resolvedFrom": "storage",
                "_aiRegion": "US",
                "region": "US"
            }
        });

        let auth_raw = ensure_auth_raw_for_inject(&account, Some(&existing));
        let auth_obj = auth_raw.as_object().expect("auth raw should be object");

        assert_eq!(
            auth_obj.get("existingOnly").and_then(Value::as_str),
            Some("keep-me")
        );
        assert_eq!(
            auth_obj.get("strictToken").and_then(Value::as_str),
            Some("from-source")
        );
        assert_eq!(
            auth_obj.get("accessToken").and_then(Value::as_str),
            Some("old-access")
        );
        assert_eq!(
            auth_obj.get("token").and_then(Value::as_str),
            Some("old-access")
        );
        assert_eq!(
            auth_obj
                .get("account")
                .and_then(|value| value.get("tenantId"))
                .and_then(Value::as_str),
            Some("tenant-123")
        );
        assert_eq!(
            auth_obj
                .get("account")
                .and_then(|value| value.get("sourceBadge"))
                .and_then(Value::as_str),
            Some("raw")
        );
        assert_eq!(
            auth_obj
                .get("userRegion")
                .and_then(|value| value.get("resolvedFrom"))
                .and_then(Value::as_str),
            Some("storage")
        );
        assert_eq!(
            auth_obj
                .get("userRegion")
                .and_then(|value| value.get("sourceRegionMeta"))
                .and_then(Value::as_str),
            Some("raw")
        );
        assert_eq!(
            auth_obj
                .get("account")
                .and_then(|value| value.get("username"))
                .and_then(Value::as_str),
            Some("tester")
        );
        assert_eq!(
            auth_obj
                .get("userRegion")
                .and_then(|value| value.get("_aiRegion"))
                .and_then(Value::as_str),
            Some("SG")
        );
    }

    #[test]
    fn ensure_auth_raw_for_inject_recovers_refresh_expiry_and_host() {
        let mut account = sample_account();
        account.trae_auth_raw = Some(serde_json::json!({
            "host": "https://www.trae.ai",
            "storeRegion": "SG",
            "AIRegion": "SG",
            "loginRegion": "sg",
            "Result": {
                "RefreshExpireAt": 1791562702466_u64
            }
        }));

        let auth_raw = ensure_auth_raw_for_inject(&account, None);
        let auth_obj = auth_raw.as_object().expect("auth raw should be object");

        assert_eq!(
            auth_obj.get("host").and_then(Value::as_str),
            Some("https://www.trae.ai")
        );
        assert_eq!(
            auth_obj.get("loginHost").and_then(Value::as_str),
            Some("https://www.trae.ai")
        );
        assert_eq!(
            auth_obj.get("accessToken").and_then(Value::as_str),
            Some("old-access")
        );
        assert_eq!(
            auth_obj.get("refreshExpiredAt").and_then(Value::as_str),
            Some("2026-10-09T16:18:22.466Z")
        );
        assert_eq!(
            auth_obj
                .get("account")
                .and_then(|value| value.get("username"))
                .and_then(Value::as_str),
            Some("李杰")
        );
    }

    #[test]
    fn ensure_auth_raw_for_inject_uses_official_scope_pair() {
        let mut account = sample_account();
        account.trae_auth_raw = Some(serde_json::json!({
            "callbackQuery": {
                "scope": "trae"
            }
        }));

        let auth_raw = ensure_auth_raw_for_inject(&account, None);
        let auth_obj = auth_raw.as_object().expect("auth raw should be object");

        assert_eq!(
            auth_obj
                .get("account")
                .and_then(|value| value.get("scope"))
                .and_then(Value::as_str),
            Some("marscode")
        );
        assert_eq!(
            auth_obj
                .get("account")
                .and_then(|value| value.get("loginScope"))
                .and_then(Value::as_str),
            Some("trae")
        );
    }

    #[test]
    fn ensure_auth_raw_for_inject_prefers_callback_host_for_storage() {
        let mut account = sample_account();
        account.trae_auth_raw = Some(serde_json::json!({
            "host": "https://growsg-normal.trae.ai",
            "loginHost": "https://growsg-normal.trae.ai",
            "callbackQuery": {
                "host": "https://api-sg-central.trae.ai",
                "scope": "trae"
            },
            "storeRegion": "SG",
            "AIRegion": "SG",
            "loginRegion": "sg"
        }));

        let auth_raw = ensure_auth_raw_for_inject(&account, None);
        let auth_obj = auth_raw.as_object().expect("auth raw should be object");

        assert_eq!(
            auth_obj.get("host").and_then(Value::as_str),
            Some("https://api-sg-central.trae.ai")
        );
        assert_eq!(
            auth_obj.get("loginHost").and_then(Value::as_str),
            Some("https://api-sg-central.trae.ai")
        );
    }

    #[test]
    fn device_proof_message_matches_official_newline_format() {
        let message = build_device_proof_message("client-1", "refresh-1", 1_783_355_376, "nonce-1");

        assert_eq!(
            message,
            "POST\n/trae/api/v3/oauth/ExchangeToken\nclient-1\nrefresh-1\n1783355376\nnonce-1"
        );
    }

    #[test]
    fn storage_provider_resolution_ignores_device_key_pair_entries() {
        let mut root = Map::new();
        root.insert(
            "iCubeAuthInfo://icube-dc:7633793279305631249".to_string(),
            Value::String("device-key-pair".to_string()),
        );
        root.insert(
            "iCubeEntitlementInfo://icube-dc:7633793279305631249".to_string(),
            Value::String("{}".to_string()),
        );

        assert_eq!(
            resolve_storage_provider_id(&root),
            TRAE_DEFAULT_AUTH_PROVIDER_ID
        );
        assert!(!has_trae_auth_storage_key(&root));

        root.insert(
            TRAE_STORAGE_AUTH_KEY.to_string(),
            Value::String("auth-payload".to_string()),
        );
        assert_eq!(
            resolve_storage_provider_id(&root),
            TRAE_DEFAULT_AUTH_PROVIDER_ID
        );
        assert!(has_trae_auth_storage_key(&root));
    }

    #[test]
    fn device_key_pair_for_inject_uses_official_device_storage_key() {
        let mut account = sample_account();
        account.trae_auth_raw = Some(serde_json::json!({
            "deviceInfo": {
                "DeviceID": "7633793279305631249"
            },
            "deviceKeyPair": {
                "privateKeyPEM": "private-key",
                "publicKeyPEM": "public-key"
            }
        }));
        let mut root = Map::new();

        write_device_key_pair_for_inject(&mut root, &account).expect("write device key");

        assert!(root.contains_key("iCubeAuthInfo://icube-dc:7633793279305631249"));
        assert!(!has_trae_auth_storage_key(&root));
        let decoded = root
            .get("iCubeAuthInfo://icube-dc:7633793279305631249")
            .and_then(|value| parse_value_or_json_string_or_icube_cipher(Some(value)))
            .expect("decoded device key");
        assert_eq!(
            decoded.get("privateKeyPEM").and_then(Value::as_str),
            Some("private-key")
        );
        assert_eq!(
            decoded.get("publicKeyPEM").and_then(Value::as_str),
            Some("public-key")
        );
    }
}
