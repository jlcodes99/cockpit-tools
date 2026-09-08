use crate::models::codex::{
    CodexAccount, CodexApiModelMapping, CodexApiProviderMode, CodexAppSpeed, CodexAppSpeedConfig,
    CodexQuickConfig, CodexQuota, CodexTokens,
};
use crate::models::codex_local_access::{
    CodexLocalAccessAccountModelRule, CodexLocalAccessAccountWindowQuery,
    CodexLocalAccessAccountWindowStats, CodexLocalAccessAppendAccountsResult,
    CodexLocalAccessChatMessage, CodexLocalAccessChatResult, CodexLocalAccessClientBaseUrlHost,
    CodexLocalAccessCustomRoutingRule, CodexLocalAccessGatewayMode,
    CodexLocalAccessImageGenerationPolicy, CodexLocalAccessModelAlias,
    CodexLocalAccessModelPricing, CodexLocalAccessPortCleanupResult, CodexLocalAccessQuotaReserve,
    CodexLocalAccessRequestKind, CodexLocalAccessRoutingStrategy, CodexLocalAccessScope,
    CodexLocalAccessState, CodexLocalAccessTestFailure, CodexLocalAccessTestResult,
    CodexLocalAccessTimeoutPreset, CodexLocalAccessTimeouts, CodexLocalAccessUsageEventPage,
};
use crate::modules::{
    account, codex_account, codex_local_access, codex_oauth, codex_quota, codex_session_visibility,
    codex_speed, codex_wakeup, codex_wakeup_scheduler, config, hermes_auth, logger, openclaw_auth,
    opencode_auth, process,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use tauri::AppHandle;
use tauri::Emitter;
use tauri_plugin_opener::OpenerExt;

// Codex 命令按职责拆分为以下三个源码片段；这些文件通过 include! 保持原有模块作用域和命令调用路径不变。
// - codex_account_commands.rs：账号、授权、切号、导入导出与配额命令。
// - codex_model_provider_commands.rs：模型供应商配置、连接测试和用量查询命令。
// - codex_local_access_commands.rs：本地 API 服务、API Key 与网关管理命令。
// 各片段中的公开命令仍由本模块统一导出，调用方无需改变。
include!("codex_account_commands.rs");
include!("codex_model_provider_commands.rs");
include!("codex_local_access_commands.rs");
