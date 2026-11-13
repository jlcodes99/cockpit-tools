use super::{
    background_maintenance as maintenance,
    legacy_json_input::{Input, ReadError},
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader, ErrorKind},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub(crate) const LEGACY_FILE: &str = "provider_current_accounts.json";
const DATABASE: &str = "provider_current_accounts.sqlite";

#[derive(Default, Deserialize)]
struct LegacyRefs {
    #[serde(default, rename = "version")]
    _version: String,
    #[serde(default)]
    current_accounts: HashMap<String, String>,
}

fn normalize_platform(platform: &str) -> Result<&'static str, String> {
    match platform.trim() {
        "windsurf" => Ok("windsurf"),
        "kiro" => Ok("kiro"),
        "cursor" => Ok("cursor"),
        "grok" => Ok("grok"),
        "claude_desktop_account" => Ok("claude_desktop_account"),
        "claude_code_account" => Ok("claude_code_account"),
        "codebuddy" => Ok("codebuddy"),
        "codebuddy_cn" | "codebuddy-cn" => Ok("codebuddy_cn"),
        "qoder" => Ok("qoder"),
        "zcode" => Ok("zcode"),
        "trae" => Ok("trae"),
        "trae_solo" | "trae-solo" => Ok("trae_solo"),
        "trae_cn" | "trae-cn" => Ok("trae_cn"),
        "trae_solo_cn" | "trae-solo-cn" => Ok("trae_solo_cn"),
        "workbuddy" => Ok("workbuddy"),
        "minimaxcode" => Ok("minimaxcode"),
        "github_copilot" | "github-copilot" | "ghcp" => Ok("github_copilot"),
        other => Err(format!("不支持的平台: {}", other)),
    }
}

#[derive(Clone)]
struct CurrentStore {
    root: PathBuf,
}

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
struct State {
    source: u8,
    stamp: Option<String>,
    offset: u64,
    in_map: bool,
    next_item: bool,
    saw_map: bool,
    #[serde(default)]
    saw_version: bool,
    complete: bool,
}

impl CurrentStore {
    fn legacy(&self, source: u8) -> PathBuf {
        let path = self.root.join(LEGACY_FILE);
        if source == 2 {
            path.with_extension("json.bak")
        } else {
            path
        }
    }

    fn open(&self) -> Result<Connection, String> {
        let conn =
            Connection::open(self.root.join(DATABASE)).map_err(error)?;
        conn.busy_timeout(Duration::from_millis(100))
            .map_err(error)?;
        let version: u32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(error)?;
        match version {
            1 => {}
            0 => conn
                .execute_batch(
                    "BEGIN IMMEDIATE;
                 CREATE TABLE IF NOT EXISTS current_refs (
                    platform TEXT PRIMARY KEY, account_id TEXT, source INTEGER NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS migration (
                    singleton INTEGER PRIMARY KEY CHECK(singleton=1), state TEXT NOT NULL
                 );
                 PRAGMA user_version=1;
                 COMMIT;",
                )
                .map_err(error)?,
            _ => return Err("本地当前账号数据库版本不受支持".into()),
        }
        Ok(conn)
    }

    fn load(&self, key: &str) -> Result<Option<String>, String> {
        let mut conn = self.open()?;
        let tx = conn.transaction().map_err(error)?;
        let state = read_state(&tx)?;
        let visible = if state.complete { state.source } else { 0 };
        let row: Option<Option<String>> = tx
            .query_row(
                "SELECT account_id FROM current_refs WHERE platform=?1 AND (source=0 OR source=?2)",
                params![key, visible],
                |row| row.get(0),
            )
            .optional()
            .map_err(error)?;
        tx.commit().map_err(error)?;
        if let Some(row) = row {
            return Ok(row);
        }
        if state.complete {
            return Ok(None);
        }
        self.schedule();
        let legacy = self.read_legacy(&state)?;
        Ok(legacy
            .current_accounts
            .get(key)
            .and_then(|id| normalize_id(Some(id)))
            .map(str::to_owned))
    }

    fn write(&self, key: &str, id: Option<&str>, expected: Option<&str>) -> Result<(), String> {
        let mut conn = self.open()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(error)?;
        let state = read_state(&tx)?;
        // A NULL row records an intentional clear and prevents a legacy
        // reference from returning. Stale cleanup compares the observed ID.
        tx.execute(
            "INSERT INTO current_refs(platform,account_id,source) VALUES(?1,?2,0)
             ON CONFLICT(platform) DO UPDATE SET account_id=excluded.account_id,source=0
             WHERE ?3 IS NULL OR current_refs.account_id=?3
               OR (current_refs.source<>0 AND current_refs.source<>?4)",
            params![key, id, expected, state.source],
        )
        .map_err(error)?;
        tx.commit().map_err(error)?;
        if !state.complete {
            self.schedule();
        }
        Ok(())
    }

    fn read_legacy(&self, state: &State) -> Result<LegacyRefs, String> {
        let read = |source| read_snapshot(&self.legacy(source));
        if state.source == 2 {
            return read(2)?.ok_or_else(|| "旧当前账号备份不可读".into());
        }
        match read(1) {
            Ok(Some(snapshot)) => Ok(snapshot),
            Ok(None) => match read(2)? {
                Some(backup) => Ok(backup),
                None if state.stamp.is_none() => Ok(LegacyRefs::default()),
                None => Err("迁移期间旧当前账号映射不可用".into()),
            },
            Err(primary) => match read(2) {
                Ok(Some(backup)) => Ok(backup),
                _ => Err(primary),
            },
        }
    }

    fn schedule(&self) {
        let store = self.clone();
        maintenance::schedule(
            format!("provider-current:{}", self.root.display()),
            move || store.migrate_batch(),
        );
    }

    fn migrate_batch(&self) -> Result<bool, String> {
        let mut conn = self.open()?;
        let before = read_state(&conn)?;
        if before.complete {
            return Ok(true);
        }
        let mut next = before.clone();
        if next.source == 0 {
            next.source = 1;
        }
        let mut rows = Vec::new();
        match File::open(self.legacy(next.source)) {
            Ok(file) => {
                let metadata = file.metadata().map_err(error)?;
                let stamp = format!(
                    "{}:{:?}",
                    metadata.len(),
                    metadata.modified().map_err(error)?
                );
                if next.stamp.as_ref().is_some_and(|saved| saved != &stamp) {
                    return Err("迁移期间旧当前账号映射发生变化".into());
                }
                next.stamp = Some(stamp);
                let mut input =
                    Input::new(file, next.offset, maintenance::BATCH_BYTES).map_err(error)?;
                match read_batch(&mut input, &mut next) {
                    Ok(_)
                        if next.complete
                            && next.offset == 0
                            && next.source == 1
                            && self.legacy(2).try_exists().map_err(error)? =>
                    {
                        next = State {
                            source: 2,
                            ..State::default()
                        };
                    }
                    Ok(batch) => rows = batch,
                    Err(ReadError::Format)
                        if next.source == 1 && self.legacy(2).try_exists().map_err(error)? =>
                    {
                        next = State {
                            source: 2,
                            ..State::default()
                        };
                    }
                    Err(cause) => return Err(legacy_error(cause)),
                }
            }
            Err(cause)
                if cause.kind() == ErrorKind::NotFound
                    && next.source == 1
                    && self.legacy(2).try_exists().map_err(error)? =>
            {
                next = State {
                    source: 2,
                    ..State::default()
                };
            }
            Err(cause)
                if cause.kind() == ErrorKind::NotFound
                    && next.source == 1
                    && next.stamp.is_none() =>
            {
                next.complete = true;
            }
            Err(cause) => return Err(error(cause)),
        }
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(error)?;
        if read_state(&tx)? != before {
            return Ok(false);
        }
        for (key, id) in rows {
            tx.execute(
                "INSERT INTO current_refs(platform,account_id,source) VALUES(?1,?2,?3)
                 ON CONFLICT(platform) DO UPDATE SET account_id=excluded.account_id,source=excluded.source
                 WHERE current_refs.source<>0",
                params![key, normalize_id(Some(&id)), next.source],
            ).map_err(error)?;
        }
        tx.execute(
            "INSERT INTO migration(singleton,state) VALUES(1,?1)
             ON CONFLICT(singleton) DO UPDATE SET state=excluded.state",
            [serde_json::to_string(&next).map_err(error)?],
        )
        .map_err(error)?;
        tx.commit().map_err(error)?;
        Ok(next.complete)
    }
}

fn read_snapshot(path: &Path) -> Result<Option<LegacyRefs>, String> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(cause) if cause.kind() == ErrorKind::NotFound => return Ok(None),
        Err(cause) => return Err(error(cause)),
    };
    let mut input = BufReader::new(file);
    loop {
        let buf = input.fill_buf().map_err(error)?;
        if buf.is_empty() {
            return Ok(None);
        }
        let spaces = buf
            .iter()
            .take_while(|byte| byte.is_ascii_whitespace())
            .count();
        let has_content = spaces < buf.len();
        input.consume(spaces);
        if has_content {
            break;
        }
    }
    serde_json::from_reader(input).map(Some).map_err(error)
}

fn read_state(conn: &Connection) -> Result<State, String> {
    let raw: Option<String> = conn
        .query_row("SELECT state FROM migration WHERE singleton=1", [], |row| {
            row.get(0)
        })
        .optional()
        .map_err(error)?;
    raw.map(|raw| serde_json::from_str(&raw).map_err(error))
        .transpose()
        .map(Option::unwrap_or_default)
}

fn read_batch(input: &mut Input, state: &mut State) -> Result<Vec<(String, String)>, ReadError> {
    let start = state.offset;
    let started = Instant::now();
    let mut rows = Vec::new();
    if start == 0 {
        if input.peek()?.is_none() {
            state.complete = true;
            return Ok(rows);
        }
        input.expect(b'{')?;
    }
    loop {
        match read_item(input, state) {
            Ok(Some(row)) => rows.push(row),
            Ok(None) => {}
            Err(ReadError::Budget) if state.offset > start => break,
            Err(cause) => return Err(cause),
        }
        state.offset = input.offset;
        if state.complete
            || rows.len() >= maintenance::BATCH_ROWS
            || input.budget == 0
            || started.elapsed() >= maintenance::BATCH_TIME
        {
            break;
        }
    }
    Ok(rows)
}

fn read_item(input: &mut Input, state: &mut State) -> Result<Option<(String, String)>, ReadError> {
    if input.peek()? == Some(b'}') {
        input.take()?;
        if state.in_map {
            state.in_map = false;
            state.next_item = true;
        } else {
            if input.peek()?.is_some() {
                return Err(ReadError::Format);
            }
            state.complete = true;
        }
        return Ok(None);
    }
    if state.next_item {
        input.expect(b',')?;
    }
    let key = input.value::<String>()?;
    input.expect(b':')?;
    if state.in_map {
        let account_id = input.value()?;
        state.next_item = true;
        return Ok(Some((key, account_id)));
    }
    if key == "current_accounts" {
        if state.saw_map {
            return Err(ReadError::Format);
        }
        input.expect(b'{')?;
        state.saw_map = true;
        state.in_map = true;
        state.next_item = false;
    } else {
        if key == "version" {
            if state.saw_version {
                return Err(ReadError::Format);
            }
            input.value::<String>()?;
            state.saw_version = true;
        } else {
            input.value::<serde::de::IgnoredAny>()?;
        }
        state.next_item = true;
    }
    Ok(None)
}

fn normalize_id(id: Option<&str>) -> Option<&str> {
    id.map(str::trim).filter(|id| !id.is_empty())
}

fn error(cause: impl std::fmt::Display) -> String {
    format!("读写当前账号映射失败: {cause}")
}

fn legacy_error(cause: ReadError) -> String {
    match cause {
        ReadError::Budget => "旧当前账号映射单项超出迁移预算，原文件已保留".into(),
        ReadError::Format => "旧当前账号映射格式无效，原文件已保留".into(),
        ReadError::Io(cause) => error(cause),
    }
}

fn store() -> Result<CurrentStore, String> {
    Ok(CurrentStore {
        root: super::account::get_data_dir()?,
    })
}

pub fn schedule_migration() -> Result<(), String> {
    store()?.schedule();
    Ok(())
}

pub fn get_current_account_id(platform: &str) -> Result<Option<String>, String> {
    let key = normalize_platform(platform)?;
    store()?.load(key)
}

pub fn resolve_existing_current_account_id<'a, I>(platform: &str, existing_ids: I) -> Option<String>
where
    I: IntoIterator<Item = &'a str>,
{
    let store = store().ok()?;
    let key = normalize_platform(platform).ok()?;
    let current_id = match store.load(key) {
        Ok(id) => id?,
        Err(error) => {
            super::logger::log_warn(&format!(
                "[CurrentAccounts] 读取平台 {key} 的当前账号失败: {error}"
            ));
            return None;
        }
    };
    if existing_ids.into_iter().any(|id| id == current_id.as_str()) {
        Some(current_id)
    } else {
        if let Err(error) = store.write(key, None, Some(&current_id)) {
            super::logger::log_warn(&format!("[CurrentAccounts] 清理无效账号引用失败: {error}"));
        }
        None
    }
}

pub fn set_current_account_id(platform: &str, account_id: Option<&str>) -> Result<(), String> {
    let key = normalize_platform(platform)?;
    store()?.write(key, normalize_id(account_id), None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_data_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cockpit-provider-current-{}-{}",
            name,
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp data dir");
        dir
    }

    struct DataDirGuard {
        dir: PathBuf,
        previous_data_dir: Option<String>,
    }

    impl DataDirGuard {
        fn new(name: &str) -> Self {
            let dir = temp_data_dir(name);
            let previous_data_dir = std::env::var("COCKPIT_TOOLS_DATA_DIR").ok();
            std::env::set_var("COCKPIT_TOOLS_DATA_DIR", &dir);
            Self {
                dir,
                previous_data_dir,
            }
        }
    }

    impl Drop for DataDirGuard {
        fn drop(&mut self) {
            match self.previous_data_dir.as_ref() {
                Some(value) => std::env::set_var("COCKPIT_TOOLS_DATA_DIR", value),
                None => std::env::remove_var("COCKPIT_TOOLS_DATA_DIR"),
            }
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn provider_current_state_normalizes_platform_aliases() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .expect("lock env");
        let _guard = DataDirGuard::new("aliases");

        set_current_account_id("github-copilot", Some("gh-account")).expect("set github alias");
        assert_eq!(
            get_current_account_id("github_copilot").expect("get github canonical"),
            Some("gh-account".to_string())
        );
        assert_eq!(
            get_current_account_id("ghcp").expect("get github short alias"),
            Some("gh-account".to_string())
        );

        set_current_account_id("codebuddy-cn", Some("cn-account")).expect("set cn alias");
        assert_eq!(
            get_current_account_id("codebuddy_cn").expect("get cn canonical"),
            Some("cn-account".to_string())
        );

        set_current_account_id("trae-solo", Some("solo-account")).expect("set trae solo alias");
        assert_eq!(
            get_current_account_id("trae_solo").expect("get trae solo canonical"),
            Some("solo-account".to_string())
        );

        set_current_account_id("trae-solo-cn", Some("solo-cn-account"))
            .expect("set trae solo cn alias");
        assert_eq!(
            get_current_account_id("trae_solo_cn").expect("get trae solo cn canonical"),
            Some("solo-cn-account".to_string())
        );

        set_current_account_id("grok", Some("grok-account")).expect("set grok current");
        assert_eq!(
            get_current_account_id("grok").expect("get grok canonical"),
            Some("grok-account".to_string())
        );
    }

    #[test]
    fn provider_current_state_clears_stale_current_account_id() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .expect("lock env");
        let _guard = DataDirGuard::new("stale");

        set_current_account_id("cursor", Some("stale-account")).expect("set cursor current");
        assert_eq!(
            resolve_existing_current_account_id("cursor", ["other-account"].iter().copied()),
            None
        );
        assert_eq!(
            get_current_account_id("cursor").expect("get cursor after stale cleanup"),
            None
        );

        set_current_account_id("cursor", Some("active-account")).expect("set active cursor");
        assert_eq!(
            resolve_existing_current_account_id(
                "cursor",
                ["other-account", "active-account"].iter().copied()
            ),
            Some("active-account".to_string())
        );
    }
    #[test]
    fn provider_current_state_migration_keeps_new_platform_writes_and_intentional_clears() {
        let directory = tempfile::tempdir().unwrap();
        let first = CurrentStore {
            root: directory.path().into(),
        };
        let mut legacy: HashMap<_, _> = (0..60)
            .map(|index| (format!("legacy-{index}"), format!("id-{index}")))
            .collect();
        legacy.insert("cursor".into(), "old-cursor".into());
        legacy.insert("windsurf".into(), "old-windsurf".into());
        legacy.insert("workbuddy".into(), "old-workbuddy".into());
        let bytes = serde_json::to_vec(&serde_json::json!({
            "version": "1.0", "current_accounts": legacy,
        }))
        .unwrap();
        fs::write(first.legacy(1), &bytes).unwrap();
        assert!(!first.migrate_batch().unwrap());
        let checkpoint = read_state(&first.open().unwrap()).unwrap();
        assert!(checkpoint.offset > 0);

        first.write("cursor", None, None).unwrap();
        first.write("windsurf", Some("new-windsurf"), None).unwrap();
        let second = CurrentStore {
            root: directory.path().into(),
        };
        second
            .write("workbuddy", Some("new-workbuddy"), None)
            .unwrap();
        second
            .write("windsurf", None, Some("old-windsurf"))
            .unwrap();
        for _ in 0..20 {
            if second.migrate_batch().unwrap() {
                break;
            }
        }
        assert!(read_state(&second.open().unwrap()).unwrap().complete);
        assert_eq!(second.load("cursor").unwrap(), None);
        assert_eq!(
            second.load("windsurf").unwrap().as_deref(),
            Some("new-windsurf")
        );
        assert_eq!(
            second.load("workbuddy").unwrap().as_deref(),
            Some("new-workbuddy")
        );
        assert_eq!(second.load("legacy-59").unwrap().as_deref(), Some("id-59"));
        assert_eq!(fs::read(first.legacy(1)).unwrap(), bytes);
    }
}
