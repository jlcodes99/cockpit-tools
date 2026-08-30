// Trae 账号模块：Platform identity, encrypted storage and account index lifecycle。
// 通过 include! 保持原 modules::trae_account 作用域和平台行为。
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
/// Trae CN / TRAE SOLO CN 官方 pay 接口当前以 v2 为准（参考社区 #1281）。
const TRAE_CN_PAY_STATUS_PATH: &str = "/trae/api/v2/pay/ide_user_pay_status";
const TRAE_CN_ENT_USAGE_PATH: &str = "/trae/api/v2/pay/ide_user_ent_usage";
const TRAE_CN_CURRENT_ENTITLEMENT_LIST_PATH: &str =
    "/trae/api/v2/pay/user_current_entitlement_list";
const TRAE_AUTH_DOMAIN: &str = "www.trae.ai";
const TRAE_CN_AUTH_DOMAIN: &str = "www.trae.cn";
const TRAE_AUTH_CLIENT_ID: &str = "ono9krqynydwx5";
const TRAE_SOLO_AUTH_CLIENT_ID: &str = "en1oxy7wnw8j9n";
const TRAE_EXCHANGE_CLIENT_SECRET: &str = "-";
const TRAE_IDE_VERSION: &str = "3.5.66";
const TRAE_NEED_REFRESH_WINDOW_MILLISECONDS: i64 = 24 * 60 * 60 * 1000;
const TRAE_CHECK_LOGIN_INVALID_ERROR_CODES: [&str; 5] =
    ["20324", "20101", "20315", "20125", "20126"];

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

#[derive(Clone, Debug)]
pub struct TraeCheckLoginVerdict {
    pub is_valid: bool,
    pub error_code: Option<String>,
    pub is_login: Option<bool>,
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

    // Only fallback to email when one side is missing user_id.
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
    match crate::modules::secure_account_storage::deserialize_account_file::<TraeAccount>(
        &account_path,
        &content,
    ) {
        Ok((account, needs_rotation)) => {
            if needs_rotation {
                let account_for_rewrite = account.clone();
                crate::modules::deferred_account_rewrite::schedule_account_rewrite_if_unchanged(
                    "trae",
                    account_for_rewrite.id.clone(),
                    account_path.clone(),
                    content.as_bytes(),
                    move || {
                        crate::modules::secure_account_storage::serialize_account_file(
                            "trae",
                            &account_for_rewrite,
                        )
                    },
                );
            }
            Some(account)
        }
        Err(_) => None,
    }
}

fn save_account_file(account: &TraeAccount) -> Result<(), String> {
    let path = resolve_account_file_path(account.id.as_str())?;
    let content = crate::modules::secure_account_storage::serialize_account_file("trae", account)?;
    crate::modules::atomic_write::write_string_atomic(&path, &content)
        .map_err(|e| format!("保存 Trae 账号失败: {}", e))
}

fn delete_account_file(account_id: &str) -> Result<(), String> {
    let path = resolve_account_file_path(account_id)?;
    if path.exists() {
        crate::modules::atomic_write::remove_file_locked(&path)
            .map_err(|e| format!("删除 Trae 账号文件失败: {}", e))?;
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

fn pick_bool(root: Option<&Value>, paths: &[&[&str]]) -> Option<bool> {
    for path in paths {
        if let Some(value) = extract_json_value(root, path) {
            if let Some(boolean) = value.as_bool() {
                return Some(boolean);
            }
            if let Some(num) = value.as_i64() {
                return Some(num != 0);
            }
            if let Some(text) = value.as_str() {
                let trimmed = text.trim();
                if trimmed.eq_ignore_ascii_case("true") || trimmed == "1" {
                    return Some(true);
                }
                if trimmed.eq_ignore_ascii_case("false") || trimmed == "0" {
                    return Some(false);
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
    normalize_non_empty(account.user_id.as_deref())
        .or_else(|| {
            pick_string(
                account.trae_auth_raw.as_ref(),
                &[
                    &["userId"],
                    &["user_id"],
                    &["uid"],
                    &["UserID"],
                    &["id"],
                    &["account", "uid"],
                    &["account", "userId"],
                    &["account", "user_id"],
                    &["user", "id"],
                    &["user", "userId"],
                ],
            )
        })
        .or_else(|| {
            pick_string(
                account.trae_profile_raw.as_ref(),
                &[
                    &["userId"],
                    &["user_id"],
                    &["uid"],
                    &["id"],
                    &["user", "id"],
                    &["user", "userId"],
                    &["account", "uid"],
                    &["account", "userId"],
                ],
            )
        })
        .or_else(|| {
            // Profile payloads are sometimes nested under data/Result.
            profile_payload_root(account.trae_profile_raw.as_ref()).and_then(|root| {
                pick_string(
                    Some(root),
                    &[
                        &["userId"],
                        &["user_id"],
                        &["uid"],
                        &["id"],
                        &["user", "id"],
                        &["user", "userId"],
                        &["account", "uid"],
                        &["account", "userId"],
                    ],
                )
            })
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

fn is_runtime_preserved_auth_key(key: &str) -> bool {
    matches!(
        key,
        "platformId"
            | "platform_id"
            | "platformName"
            | "platform"
            | "callbackQuery"
            | "callback_query"
            | "deviceInfo"
            | "device_info"
            | "deviceKeyPair"
            | "device_key_pair"
            | "exchangeResponse"
            | "exchange_response"
            | "host"
            | "loginHost"
            | "login_host"
            | "apiHost"
            | "authDomain"
            | "auth_domain"
            | "authClientId"
            | "auth_client_id"
            | "clientId"
            | "client_id"
            | "ClientID"
            | "loginRegion"
            | "storeRegion"
            | "AIRegion"
            | "userRegion"
            | "scope"
    )
}

fn merge_runtime_json(existing: Option<&Value>, incoming: Option<&Value>) -> Option<Value> {
    let Some(incoming) = incoming else {
        return existing.cloned();
    };
    let Some(incoming_object) = incoming.as_object() else {
        return Some(incoming.clone());
    };

    let mut merged = existing
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for (key, incoming_value) in incoming_object {
        let value = match merged.get(key) {
            Some(existing_value) if existing_value.is_object() && incoming_value.is_object() => {
                merge_runtime_json(Some(existing_value), Some(incoming_value))
                    .unwrap_or_else(|| incoming_value.clone())
            }
            _ => incoming_value.clone(),
        };
        merged.insert(key.clone(), value);
    }
    Some(Value::Object(merged))
}

fn merge_runtime_auth_json(existing: Option<&Value>, incoming: Option<&Value>) -> Value {
    let mut merged = existing
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if let Some(incoming_object) = incoming.and_then(Value::as_object) {
        for (key, incoming_value) in incoming_object {
            if is_runtime_preserved_auth_key(key) && merged.contains_key(key) {
                continue;
            }
            let value = match merged.get(key) {
                Some(existing_value)
                    if existing_value.is_object() && incoming_value.is_object() =>
                {
                    merge_runtime_json(Some(existing_value), Some(incoming_value))
                        .unwrap_or_else(|| incoming_value.clone())
                }
                _ => incoming_value.clone(),
            };
            merged.insert(key.clone(), value);
        }
    }
    Value::Object(merged)
}

fn apply_runtime_session_payload(account: &mut TraeAccount, payload: TraeImportPayload) {
    if !payload.access_token.trim().is_empty() {
        account.access_token = payload.access_token;
    }
    if let Some(refresh_token) = normalize_non_empty(payload.refresh_token.as_deref()) {
        account.refresh_token = Some(refresh_token);
    }
    if let Some(token_type) = normalize_non_empty(payload.token_type.as_deref()) {
        account.token_type = Some(token_type);
    }
    if payload.expires_at.is_some() {
        account.expires_at = normalize_timestamp(payload.expires_at);
    }

    let mut auth_raw = merge_runtime_auth_json(
        account.trae_auth_raw.as_ref(),
        payload.trae_auth_raw.as_ref(),
    );
    if let Some(auth_object) = auth_raw.as_object_mut() {
        auth_object.insert(
            "accessToken".to_string(),
            Value::String(account.access_token.clone()),
        );
        auth_object.insert(
            "token".to_string(),
            Value::String(account.access_token.clone()),
        );
        if let Some(refresh_token) = account.refresh_token.as_ref() {
            auth_object.insert(
                "refreshToken".to_string(),
                Value::String(refresh_token.clone()),
            );
        }
        if let Some(token_type) = account.token_type.as_ref() {
            auth_object.insert("tokenType".to_string(), Value::String(token_type.clone()));
        }
        if let Some(expires_at) = account.expires_at {
            auth_object.insert(
                "expiresAt".to_string(),
                Value::Number(serde_json::Number::from(expires_at)),
            );
        }
    }
    account.trae_auth_raw = Some(auth_raw);
    account.trae_profile_raw = merge_runtime_json(
        account.trae_profile_raw.as_ref(),
        payload.trae_profile_raw.as_ref(),
    );
    account.trae_entitlement_raw = merge_runtime_json(
        account.trae_entitlement_raw.as_ref(),
        payload.trae_entitlement_raw.as_ref(),
    );
    account.trae_usage_raw = merge_runtime_json(
        account.trae_usage_raw.as_ref(),
        payload.trae_usage_raw.as_ref(),
    );
    account.trae_server_raw = merge_runtime_json(
        account.trae_server_raw.as_ref(),
        payload.trae_server_raw.as_ref(),
    );
}

fn runtime_payload_matches_account_platform(
    account: &TraeAccount,
    payload: &TraeImportPayload,
) -> bool {
    resolve_account_platform_kind(account) == resolve_payload_platform_kind(payload)
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
    let normalized_email = normalize_identity_email(Some(payload.email.as_str()));
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
            account_matches_import_identity(
                account,
                normalized_user_id.as_deref(),
                normalized_email.as_deref(),
            )
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

