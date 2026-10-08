// Codex 会话展示信息：按官方客户端展示规则解析会话标题与项目名。
//
// 官方 Codex 客户端（Desktop / app-server）的会话标题取自会话目录与状态库中的展示名，
// 没有生成标题时会回退到首条用户消息（压缩空白并截断 60 字符）。
// Cockpit 过去只读 session_index.jsonl，缺失时只能显示会话 ID，于是出现
// “会话管理里的对话名称和 Codex 不一致”。本模块只读官方数据文件，
// 任何文件缺失、表结构变化或读取失败都会静默回退到调用方原有逻辑。
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};

const STATE_DB_FILE: &str = "state_5.sqlite";
const SQLITE_DIR_NAME: &str = "sqlite";
const LOCAL_THREAD_CATALOG_TABLE: &str = "local_thread_catalog";
const LOCAL_CATALOG_HOST_ID: &str = "local";
const SQLITE_BUSY_TIMEOUT: Duration = Duration::from_millis(300);
/// 与客户端保持一致：首条用户消息兜底标题的最大字符数。
const CLIENT_TITLE_MAX_CHARS: usize = 60;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProjectRoot {
    normalized_path: String,
    name: String,
}

/// 一次会话列表读取期间复用的展示信息快照。
#[derive(Debug, Default, Clone)]
pub(crate) struct SessionDisplayContext {
    catalog_titles: HashMap<String, String>,
    catalog_local_host_ids: HashSet<String>,
    thread_names: HashMap<String, String>,
    thread_previews: HashMap<String, String>,
    project_roots: Vec<ProjectRoot>,
}

impl SessionDisplayContext {
    /// 读取实例目录下的官方会话展示信息；任何读取失败都按“没有额外信息”处理。
    pub(crate) fn load(data_dir: &Path) -> Self {
        let mut context = Self::default();
        for path in state_db_candidates(data_dir) {
            context.read_state_db(&path);
        }
        for path in local_catalog_candidates(data_dir) {
            context.read_local_thread_catalog(&path);
        }
        context.project_roots.sort_by(|left, right| {
            right
                .normalized_path
                .len()
                .cmp(&left.normalized_path.len())
                .then_with(|| left.normalized_path.cmp(&right.normalized_path))
        });
        context
    }

    /// 按官方客户端展示规则解析会话标题：
    /// 会话目录展示名 → 状态库会话名 → 传入的 session_index 标题 → 首条用户消息（截断 60 字符）。
    pub(crate) fn resolve_title(
        &self,
        session_id: &str,
        index_title: Option<&str>,
    ) -> Option<String> {
        let candidates = [
            self.catalog_titles.get(session_id).map(String::as_str),
            self.thread_names.get(session_id).map(String::as_str),
            index_title,
        ];
        for candidate in candidates.into_iter().flatten() {
            let trimmed = candidate.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
        self.thread_previews
            .get(session_id)
            .and_then(|value| truncate_client_title(value))
    }

    /// 解析工作目录所属项目名（官方客户端可重命名的分组名）。
    pub(crate) fn project_name_for_cwd(&self, cwd: &str) -> Option<String> {
        let normalized_cwd = normalize_path_for_compare(cwd)?;
        self.project_roots
            .iter()
            .find(|root| path_matches_project_root(&normalized_cwd, &root.normalized_path))
            .map(|root| root.name.clone())
    }

    fn read_state_db(&mut self, path: &Path) {
        let Some(connection) = open_read_only(path) else {
            return;
        };
        for (id, name, preview) in read_thread_display_rows(&connection) {
            self.thread_names.entry(id.clone()).or_insert(name);
            if !preview.is_empty() {
                self.thread_previews.entry(id).or_insert(preview);
            }
        }
        for root in read_project_roots(&connection) {
            if self
                .project_roots
                .iter()
                .any(|existing| existing.normalized_path == root.normalized_path)
            {
                continue;
            }
            self.project_roots.push(root);
        }
    }

    fn read_local_thread_catalog(&mut self, path: &Path) {
        let Some(connection) = open_read_only(path) else {
            return;
        };
        for (host_id, thread_id, display_title) in read_catalog_rows(&connection) {
            let is_local_host = host_id == LOCAL_CATALOG_HOST_ID;
            let existing_is_local = self.catalog_local_host_ids.contains(&thread_id);
            if !is_local_host && existing_is_local {
                continue;
            }
            self.catalog_titles.insert(thread_id.clone(), display_title);
            if is_local_host {
                self.catalog_local_host_ids.insert(thread_id);
            }
        }
    }
}

fn read_thread_display_rows(connection: &Connection) -> Vec<(String, String, String)> {
    let columns = table_columns(connection, "threads");
    if !columns.contains("id") {
        return Vec::new();
    }
    let name_expr = sqlite_text_expr(&columns, "name");
    let preview_expr = sqlite_text_expr(&columns, "preview");
    let sql = format!("SELECT id, {name_expr}, {preview_expr} FROM threads");
    let Ok(mut statement) = connection.prepare(&sql) else {
        return Vec::new();
    };
    let Ok(rows) = statement.query_map([], |row| {
        Ok((
            row.get::<usize, String>(0)?,
            row.get::<usize, String>(1)?,
            row.get::<usize, String>(2)?,
        ))
    }) else {
        return Vec::new();
    };
    rows.flatten()
        .filter_map(|(id, name, preview)| {
            let id = id.trim().to_string();
            if id.is_empty() {
                return None;
            }
            Some((id, name.trim().to_string(), preview.trim().to_string()))
        })
        .collect()
}

fn read_project_roots(connection: &Connection) -> Vec<ProjectRoot> {
    let project_columns = table_columns(connection, "projects");
    let root_columns = table_columns(connection, "project_roots");
    if !project_columns.contains("id")
        || !project_columns.contains("name")
        || !root_columns.contains("project_id")
        || !root_columns.contains("path")
    {
        return Vec::new();
    }
    let mut names = HashMap::<String, String>::new();
    if let Ok(mut statement) = connection.prepare("SELECT id, COALESCE(name, '') FROM projects") {
        if let Ok(rows) = statement.query_map([], |row| {
            Ok((row.get::<usize, String>(0)?, row.get::<usize, String>(1)?))
        }) {
            for (id, name) in rows.flatten() {
                let name = name.trim();
                if id.trim().is_empty() || name.is_empty() {
                    continue;
                }
                names.insert(id, name.to_string());
            }
        }
    }
    if names.is_empty() {
        return Vec::new();
    }
    let Ok(mut statement) =
        connection.prepare("SELECT project_id, COALESCE(path, '') FROM project_roots")
    else {
        return Vec::new();
    };
    let Ok(rows) = statement.query_map([], |row| {
        Ok((row.get::<usize, String>(0)?, row.get::<usize, String>(1)?))
    }) else {
        return Vec::new();
    };
    rows.flatten()
        .filter_map(|(project_id, path)| {
            let name = names.get(project_id.trim())?;
            let normalized_path = normalize_path_for_compare(&path)?;
            Some(ProjectRoot {
                normalized_path,
                name: name.clone(),
            })
        })
        .collect()
}

fn read_catalog_rows(connection: &Connection) -> Vec<(String, String, String)> {
    let columns = table_columns(connection, LOCAL_THREAD_CATALOG_TABLE);
    if !columns.contains("thread_id") || !columns.contains("display_title") {
        return Vec::new();
    }
    let host_expr = if columns.contains("host_id") {
        "COALESCE(host_id, '')"
    } else {
        "''"
    };
    let sql =
        format!("SELECT thread_id, display_title, {host_expr} FROM {LOCAL_THREAD_CATALOG_TABLE}");
    let Ok(mut statement) = connection.prepare(&sql) else {
        return Vec::new();
    };
    let Ok(rows) = statement.query_map([], |row| {
        Ok((
            row.get::<usize, String>(0)?,
            row.get::<usize, String>(1)?,
            row.get::<usize, String>(2)?,
        ))
    }) else {
        return Vec::new();
    };
    rows.flatten()
        .filter_map(|(thread_id, display_title, host_id)| {
            let thread_id = thread_id.trim().to_string();
            let display_title = display_title.trim().to_string();
            if thread_id.is_empty() || display_title.is_empty() {
                return None;
            }
            Some((host_id.trim().to_string(), thread_id, display_title))
        })
        .collect()
}

fn open_read_only(path: &Path) -> Option<Connection> {
    if !path.is_file() {
        return None;
    }
    let connection =
        Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    let _ = connection.busy_timeout(SQLITE_BUSY_TIMEOUT);
    Some(connection)
}

fn state_db_candidates(data_dir: &Path) -> Vec<PathBuf> {
    let mut candidates = vec![
        data_dir.join(SQLITE_DIR_NAME).join(STATE_DB_FILE),
        data_dir.join(STATE_DB_FILE),
    ];
    candidates.dedup();
    candidates
}

fn local_catalog_candidates(data_dir: &Path) -> Vec<PathBuf> {
    let sqlite_dir = data_dir.join(SQLITE_DIR_NAME);
    let Ok(entries) = fs::read_dir(&sqlite_dir) else {
        return Vec::new();
    };
    let mut candidates = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| {
            matches!(
                path.extension().and_then(OsStr::to_str),
                Some(extension) if extension.eq_ignore_ascii_case("db") || extension.eq_ignore_ascii_case("sqlite")
            )
        })
        .collect::<Vec<_>>();
    candidates.sort();
    candidates
}

fn table_columns(connection: &Connection, table: &str) -> HashSet<String> {
    let escaped = table.replace('"', "\"\"");
    let Ok(mut statement) = connection.prepare(&format!("PRAGMA table_info(\"{escaped}\")")) else {
        return HashSet::new();
    };
    let Ok(columns) = statement.query_map([], |row| row.get::<usize, String>(1)) else {
        return HashSet::new();
    };
    columns.flatten().collect()
}

fn sqlite_text_expr(columns: &HashSet<String>, column: &str) -> String {
    if columns.contains(column) {
        format!("COALESCE({column}, '')")
    } else {
        "''".to_string()
    }
}

fn normalize_path_for_compare(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut normalized = trimmed.replace('\\', "/");
    while normalized.len() > 1 && normalized.ends_with('/') {
        normalized.pop();
    }
    if normalized.trim_matches('/').is_empty() {
        return None;
    }
    if cfg!(windows) {
        Some(normalized.to_ascii_lowercase())
    } else {
        Some(normalized)
    }
}

fn path_matches_project_root(normalized_cwd: &str, normalized_root: &str) -> bool {
    if normalized_cwd == normalized_root {
        return true;
    }
    normalized_cwd
        .strip_prefix(normalized_root)
        .is_some_and(|rest| rest.starts_with('/'))
}

/// 与客户端一致：压缩空白，超过 60 字符时截断并追加省略号。
fn truncate_client_title(value: &str) -> Option<String> {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    let chars = collapsed.chars().collect::<Vec<_>>();
    if chars.len() <= CLIENT_TITLE_MAX_CHARS {
        return Some(collapsed);
    }
    let head = chars[..CLIENT_TITLE_MAX_CHARS - 1]
        .iter()
        .collect::<String>();
    Some(format!("{}…", head.trim_end()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_temp_dir(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        let base_dir =
            std::env::temp_dir().join(format!("{}-{}-{}", prefix, std::process::id(), unique));
        if base_dir.exists() {
            fs::remove_dir_all(&base_dir).expect("cleanup old temp dir");
        }
        fs::create_dir_all(base_dir.join(SQLITE_DIR_NAME)).expect("create temp dir");
        base_dir
    }

    fn create_state_db(data_dir: &Path, threads: &[(&str, Option<&str>, &str)]) {
        let path = data_dir.join(SQLITE_DIR_NAME).join(STATE_DB_FILE);
        let connection = Connection::open(&path).expect("open state db");
        connection
            .execute_batch(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, name TEXT, preview TEXT NOT NULL DEFAULT '');",
            )
            .expect("create threads table");
        for (id, name, preview) in threads {
            connection
                .execute(
                    "INSERT INTO threads (id, name, preview) VALUES (?1, ?2, ?3)",
                    rusqlite::params![id, name, preview],
                )
                .expect("insert thread row");
        }
    }

    fn create_projects(data_dir: &Path, projects: &[(&str, &str)], roots: &[(&str, &str)]) {
        let path = data_dir.join(SQLITE_DIR_NAME).join(STATE_DB_FILE);
        let connection = Connection::open(&path).expect("open state db");
        connection
            .execute_batch(
                "CREATE TABLE projects (id TEXT PRIMARY KEY, name TEXT);\
                 CREATE TABLE project_roots (project_id TEXT, position INTEGER, path TEXT);",
            )
            .expect("create project tables");
        for (id, name) in projects {
            connection
                .execute(
                    "INSERT INTO projects (id, name) VALUES (?1, ?2)",
                    rusqlite::params![id, name],
                )
                .expect("insert project row");
        }
        for (project_id, path) in roots {
            connection
                .execute(
                    "INSERT INTO project_roots (project_id, position, path) VALUES (?1, 0, ?2)",
                    rusqlite::params![project_id, path],
                )
                .expect("insert project root row");
        }
    }

    fn create_catalog(data_dir: &Path, file_name: &str, rows: &[(&str, &str, &str)]) {
        let path = data_dir.join(SQLITE_DIR_NAME).join(file_name);
        let connection = Connection::open(&path).expect("open catalog db");
        connection
            .execute_batch(
                "CREATE TABLE local_thread_catalog (\
                     host_id TEXT NOT NULL, thread_id TEXT NOT NULL, display_title TEXT NOT NULL,\
                     PRIMARY KEY (host_id, thread_id));",
            )
            .expect("create catalog table");
        for (host_id, thread_id, display_title) in rows {
            connection
                .execute(
                    "INSERT INTO local_thread_catalog (host_id, thread_id, display_title) VALUES (?1, ?2, ?3)",
                    rusqlite::params![host_id, thread_id, display_title],
                )
                .expect("insert catalog row");
        }
    }

    #[test]
    fn resolve_title_prefers_client_display_sources() {
        let data_dir = make_temp_dir("codex-session-display-title-test");
        create_state_db(
            &data_dir,
            &[
                ("catalog-wins", Some("state name"), "preview text"),
                ("state-only", Some("state name"), "preview text"),
                ("preview-only", None, "你好"),
                ("index-only", None, ""),
            ],
        );
        create_catalog(
            &data_dir,
            "codex-dev.db",
            &[
                ("local", "catalog-wins", "客户端标题"),
                ("local", "preview-only", "你好"),
            ],
        );

        let context = SessionDisplayContext::load(&data_dir);
        assert_eq!(
            context
                .resolve_title("catalog-wins", Some("index title"))
                .as_deref(),
            Some("客户端标题")
        );
        assert_eq!(
            context
                .resolve_title("state-only", Some("index title"))
                .as_deref(),
            Some("state name")
        );
        assert_eq!(
            context.resolve_title("preview-only", None).as_deref(),
            Some("你好")
        );
        assert_eq!(
            context
                .resolve_title("index-only", Some("index title"))
                .as_deref(),
            Some("index title")
        );
        assert_eq!(context.resolve_title("unknown", None), None);
    }

    #[test]
    fn resolve_title_falls_back_to_truncated_first_message() {
        let data_dir = make_temp_dir("codex-session-display-preview-test");
        let long_message = "a".repeat(120);
        create_state_db(
            &data_dir,
            &[
                ("long-preview", None, long_message.as_str()),
                ("spaced", None, "  你好\n\n  世界  "),
            ],
        );

        let context = SessionDisplayContext::load(&data_dir);
        let title = context
            .resolve_title("long-preview", None)
            .expect("preview title");
        assert_eq!(title.chars().count(), CLIENT_TITLE_MAX_CHARS);
        assert!(title.ends_with('…'));
        assert_eq!(
            context.resolve_title("spaced", None).as_deref(),
            Some("你好 世界")
        );
    }

    #[test]
    fn resolve_title_prefers_local_catalog_host_rows() {
        let data_dir = make_temp_dir("codex-session-display-host-test");
        create_state_db(&data_dir, &[]);
        create_catalog(
            &data_dir,
            "codex-dev.db",
            &[
                ("chatgpt:abc", "thread-1", "云端标题"),
                ("local", "thread-1", "本地标题"),
            ],
        );

        let context = SessionDisplayContext::load(&data_dir);
        assert_eq!(
            context.resolve_title("thread-1", None).as_deref(),
            Some("本地标题")
        );
    }

    #[test]
    fn project_name_prefers_longest_root_match() {
        let data_dir = make_temp_dir("codex-session-display-project-test");
        create_state_db(&data_dir, &[]);
        create_projects(
            &data_dir,
            &[("project-a", "cockpit tools"), ("project-b", "子目录项目")],
            &[
                ("project-a", "/private/var/www/antigravity-cockpit-tools"),
                ("project-b", "/private/var/www/antigravity-cockpit-tools/src"),
            ],
        );

        let context = SessionDisplayContext::load(&data_dir);
        assert_eq!(
            context
                .project_name_for_cwd("/private/var/www/antigravity-cockpit-tools")
                .as_deref(),
            Some("cockpit tools")
        );
        assert_eq!(
            context
                .project_name_for_cwd("/private/var/www/antigravity-cockpit-tools/src/modules")
                .as_deref(),
            Some("子目录项目")
        );
        assert_eq!(
            context.project_name_for_cwd("/private/var/www/cockpit-tools"),
            None
        );
    }

    #[test]
    fn missing_databases_fall_back_to_index_title() {
        let data_dir = make_temp_dir("codex-session-display-empty-test");
        let context = SessionDisplayContext::load(&data_dir);
        assert_eq!(
            context
                .resolve_title("thread-1", Some("index title"))
                .as_deref(),
            Some("index title")
        );
        assert_eq!(context.resolve_title("thread-1", None), None);
        assert_eq!(context.project_name_for_cwd("/tmp/project"), None);
    }
}
