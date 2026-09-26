//! 受管模型目录（`cockpit-model-catalog.json`）的生成版本戳。
//!
//! 目录内容由代码生成，却会长期留在用户 profile 里：升级应用后如果只改了生成
//! 逻辑而不重建文件，客户端会继续读旧内容（例如 Grok 模型缺少多智能体能力声明）。
//! 这里在同目录写一个 meta 文件，记录生成器版本与目录内容哈希；启动时比对即可
//! 判断是否需要重建，既不依赖切号，也不必每次启动都重新生成。

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

/// 受管模型目录生成逻辑的版本号。改动目录结构或模型能力字段时递增。
///
/// 3：上下文与压缩统一为 90% 口径（压缩阈值不再留空），GPT-6 系列上下文修正为 256K。
pub(crate) const MANAGED_MODEL_CATALOG_GENERATOR_VERSION: u32 = 3;

const META_FILE_NAME: &str = "cockpit-model-catalog.meta.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ManagedModelCatalogMeta {
    generator: u32,
    #[serde(rename = "appVersion")]
    app_version: String,
    #[serde(rename = "catalogHash")]
    catalog_hash: String,
    #[serde(rename = "writtenAt")]
    written_at_ms: i64,
}

pub(crate) fn managed_catalog_meta_path(catalog_path: &Path) -> PathBuf {
    catalog_path.with_file_name(META_FILE_NAME)
}

fn managed_catalog_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// catalog 写入成功后刷新版本戳；失败只影响版本判断，不影响目录本身。
pub(crate) fn write_managed_catalog_meta(catalog_path: &Path) -> Result<(), String> {
    let content = fs::read_to_string(catalog_path).map_err(|e| {
        format!(
            "读取模型目录以写入版本戳失败: path={}, error={}",
            catalog_path.display(),
            e
        )
    })?;
    let meta = ManagedModelCatalogMeta {
        generator: MANAGED_MODEL_CATALOG_GENERATOR_VERSION,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        catalog_hash: managed_catalog_hash(&content),
        written_at_ms: chrono::Utc::now().timestamp_millis(),
    };
    let serialized = serde_json::to_string_pretty(&meta)
        .map_err(|e| format!("序列化模型目录版本戳失败: {}", e))?;
    crate::modules::atomic_write::write_string_atomic(
        &managed_catalog_meta_path(catalog_path),
        &serialized,
    )
}

/// catalog 被删除时同步清理版本戳，避免留下孤儿 meta。
pub(crate) fn remove_managed_catalog_meta(catalog_path: &Path) {
    let path = managed_catalog_meta_path(catalog_path);
    if path.exists() {
        let _ = fs::remove_file(path);
    }
}

/// 判断受管模型目录是否落后于当前生成逻辑。
///
/// - 目录文件不存在：返回 false（没有受管目录，无需重建）
/// - 版本戳缺失 / 内容哈希不匹配 / 生成器版本落后：返回 true
pub(crate) fn managed_catalog_needs_rebuild(catalog_path: &Path) -> bool {
    if !catalog_path.exists() {
        return false;
    }
    let Ok(content) = fs::read_to_string(catalog_path) else {
        return false;
    };
    let Ok(serialized) = fs::read_to_string(managed_catalog_meta_path(catalog_path)) else {
        return true;
    };
    let Ok(meta) = serde_json::from_str::<ManagedModelCatalogMeta>(&serialized) else {
        return true;
    };
    meta.generator != MANAGED_MODEL_CATALOG_GENERATOR_VERSION
        || meta.catalog_hash != managed_catalog_hash(&content)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, content: &str) {
        fs::create_dir_all(path.parent().expect("parent")).expect("create dir");
        fs::write(path, content).expect("write");
    }

    #[test]
    fn missing_catalog_never_needs_rebuild() {
        let dir = std::env::temp_dir().join(format!("catalog-meta-a-{}", std::process::id()));
        let catalog = dir.join("cockpit-model-catalog.json");
        let _ = fs::remove_dir_all(&dir);
        assert!(!managed_catalog_needs_rebuild(&catalog));
    }

    #[test]
    fn catalog_without_meta_needs_rebuild_until_stamped() {
        let dir = std::env::temp_dir().join(format!("catalog-meta-b-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let catalog = dir.join("cockpit-model-catalog.json");
        write(&catalog, "{\"models\":[]}");
        assert!(managed_catalog_needs_rebuild(&catalog));
        write_managed_catalog_meta(&catalog).expect("write meta");
        assert!(!managed_catalog_needs_rebuild(&catalog));
        // 内容被外部改写后哈希不再匹配，需要重建。
        write(&catalog, "{\"models\":[{\"slug\":\"grok-4.6\"}]}");
        assert!(managed_catalog_needs_rebuild(&catalog));
        let _ = fs::remove_dir_all(&dir);
    }

}
