//! SQLite authority for Cockpit's instance registries. Official profile files are separate.
use super::legacy_json_input::{Input, ReadError};
use crate::models::{DefaultInstanceSettings, InstanceProfile, InstanceStore};
use fs2::FileExt;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, ErrorKind, Read},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

const DATABASE: &str = "instance_registry.sqlite";
const ROWS: usize = 32;
const BYTES: u64 = 2 * 1024 * 1024;
const TIME: Duration = Duration::from_millis(25);
pub(crate) const REGISTRIES: &[&str] = &[
    "instances.json",
    "antigravity_legacy_instances.json",
    "claude_instances.json",
    "codebuddy_cn_instances.json",
    "codebuddy_instances.json",
    "codex_instances.json",
    "cursor_instances.json",
    "github_copilot_instances.json",
    "grok_instances.json",
    "kiro_instances.json",
    "minimax_code_instances.json",
    "qoder_instances.json",
    "trae_cn_instances.json",
    "trae_instances.json",
    "trae_solo_cn_instances.json",
    "trae_solo_instances.json",
    "windsurf_instances.json",
    "workbuddy_instances.json",
    "zcode_instances.json",
];

#[derive(Debug, Default)]
pub(crate) struct Revision(AtomicU64);

impl Clone for Revision {
    fn clone(&self) -> Self {
        Self(AtomicU64::new(self.get()))
    }
}

impl Revision {
    pub(crate) fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }

    pub(crate) fn set(&self, value: u64) {
        self.0.store(value, Ordering::Relaxed);
    }
}

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
struct State {
    active: bool,
    revision: u64,
    stamp: Option<String>,
    backup: bool,
    offset: u64,
    position: usize,
    in_instances: bool,
    next_item: bool,
    saw_instances: bool,
    defaults: Option<Value>,
}

fn error(cause: impl std::fmt::Display) -> String {
    format!("读写实例登记失败: {cause}")
}

pub(crate) fn cleanup_gate(root: &Path, exclusive: bool) -> Result<File, String> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("instance_registry.lock"))
        .map_err(error)?;
    if exclusive {
        FileExt::try_lock_exclusive(&file)
    } else {
        FileExt::try_lock_shared(&file)
    }
    .map_err(error)?;
    Ok(file)
}

fn open(root: &Path, read_only: bool) -> Result<Option<Connection>, String> {
    let path = root.join(DATABASE);
    if read_only && !path.try_exists().map_err(error)? {
        return Ok(None);
    }
    let flags = if read_only {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
    };
    let mut conn = Connection::open_with_flags(path, flags).map_err(error)?;
    conn.busy_timeout(Duration::from_millis(80))
        .map_err(error)?;
    let version: u32 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(error)?;
    match version {
        1 => {}
        0 if read_only => return Ok(None),
        0 => {
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(error)?;
            tx.execute_batch(
                "CREATE TABLE IF NOT EXISTS instance_registry (
                    registry TEXT PRIMARY KEY, active INTEGER NOT NULL, state TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS instance_profiles (
                    registry TEXT NOT NULL, position INTEGER NOT NULL,
                    instance_id TEXT NOT NULL, user_data_dir TEXT NOT NULL, account_id TEXT,
                    profile TEXT NOT NULL, PRIMARY KEY(registry, position)
                 );
                 CREATE INDEX IF NOT EXISTS instance_profiles_id ON instance_profiles(registry, instance_id);
                 CREATE INDEX IF NOT EXISTS instance_profiles_account ON instance_profiles(registry, account_id);
                 PRAGMA user_version=1;",
            ).map_err(error)?;
            tx.commit().map_err(error)?;
        }
        _ => return Err(error("数据库版本不受支持")),
    }
    Ok(Some(conn))
}

fn writer(root: &Path) -> Result<Connection, String> {
    open(root, false)?.ok_or_else(|| error("数据库无法写入"))
}

fn state(conn: &Connection, name: &str) -> Result<State, String> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT state FROM instance_registry WHERE registry=?1",
            [name],
            |row| row.get(0),
        )
        .optional()
        .map_err(error)?;
    raw.map(|raw| serde_json::from_str(&raw).map_err(error))
        .transpose()
        .map(Option::unwrap_or_default)
}

fn set_state(conn: &Connection, name: &str, state: &State) -> Result<(), String> {
    conn.execute(
        "INSERT INTO instance_registry(registry,active,state) VALUES(?1,?2,?3)
         ON CONFLICT(registry) DO UPDATE SET active=excluded.active,state=excluded.state",
        params![
            name,
            state.active,
            serde_json::to_string(state).map_err(error)?
        ],
    )
    .map_err(error)?;
    Ok(())
}

fn insert(conn: &Connection, name: &str, position: usize, profile: &Value) -> Result<(), String> {
    conn.execute(
        "INSERT INTO instance_profiles(registry,position,instance_id,user_data_dir,account_id,profile)
         VALUES(?1,?2,?3,?4,?5,?6)
         ON CONFLICT(registry,position) DO UPDATE SET
           instance_id=excluded.instance_id,user_data_dir=excluded.user_data_dir,
           account_id=excluded.account_id,profile=excluded.profile
         WHERE instance_profiles.profile<>excluded.profile",
        params![
            name, position, profile["id"].as_str(), profile["userDataDir"].as_str(),
            profile["bindAccountId"].as_str(), serde_json::to_string(profile).map_err(error)?
        ],
    ).map_err(error)?;
    Ok(())
}

fn profiles<T: DeserializeOwned>(conn: &Connection, name: &str) -> Result<Vec<T>, String> {
    let mut query = conn
        .prepare("SELECT profile FROM instance_profiles WHERE registry=?1 ORDER BY position")
        .map_err(error)?;
    let rows = query
        .query_map([name], |row| row.get::<_, String>(0))
        .map_err(error)?;
    rows.map(|row| serde_json::from_str(&row.map_err(error)?).map_err(error))
        .collect()
}

fn legacy_path(path: &Path, backup: bool) -> std::path::PathBuf {
    if backup {
        path.with_extension("json.bak")
    } else {
        path.to_path_buf()
    }
}

fn read_file<T: DeserializeOwned>(
    path: &Path,
    max_bytes: Option<u64>,
) -> Result<Option<T>, String> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(cause) if cause.kind() == ErrorKind::NotFound => return Ok(None),
        Err(cause) => return Err(error(cause)),
    };
    if let Some(max_bytes) = max_bytes {
        let mut bytes = Vec::new();
        file.take(max_bytes + 1)
            .read_to_end(&mut bytes)
            .map_err(error)?;
        if bytes.len() as u64 > max_bytes {
            return Err(error("旧实例登记超过读取上限"));
        }
        if bytes.iter().all(u8::is_ascii_whitespace) {
            return Ok(None);
        }
        return serde_json::from_slice(&bytes).map(Some).map_err(error);
    }
    let mut reader = BufReader::new(file);
    loop {
        let bytes = reader.fill_buf().map_err(error)?;
        if bytes.is_empty() {
            return Ok(None);
        }
        let whitespace = bytes
            .iter()
            .take_while(|byte| byte.is_ascii_whitespace())
            .count();
        let content = whitespace < bytes.len();
        reader.consume(whitespace);
        if content {
            return serde_json::from_reader(reader).map(Some).map_err(error);
        }
    }
}

fn legacy<T: DeserializeOwned>(path: &Path, max_bytes: Option<u64>) -> Result<Option<T>, String> {
    let primary = read_file(path, max_bytes);
    match primary {
        Ok(Some(raw)) => Ok(Some(raw)),
        _ => match read_file(&legacy_path(path, true), max_bytes) {
            Ok(Some(raw)) => Ok(Some(raw)),
            Ok(None) => primary,
            Err(cause) => Err(cause),
        },
    }
}

struct RawStore(Value);

impl<'de> Deserialize<'de> for RawStore {
    fn deserialize<D: serde::Deserializer<'de>>(input: D) -> Result<Self, D::Error> {
        let raw = Value::deserialize(input)?;
        InstanceStore::deserialize(&raw).map_err(serde::de::Error::custom)?;
        Ok(Self(raw))
    }
}

fn registry_parts(path: &Path) -> Result<(&Path, &str), String> {
    let root = path.parent().ok_or_else(|| error("实例目录缺失"))?;
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| error("实例登记文件名无效"))?;
    Ok((root, name))
}

fn read_active(conn: &Connection, name: &str, state: State) -> Result<InstanceStore, String> {
    let mut store = InstanceStore::new();
    store.instances = profiles(conn, name)?;
    if let Some(defaults) = state.defaults {
        store.default_settings = serde_json::from_value(defaults).map_err(error)?;
    }
    store.storage_revision.set(state.revision);
    Ok(store)
}

// Also used by desktop proxy recovery, which must not initialize or rewrite storage.
pub(crate) fn read(
    path: &Path,
    max_legacy_bytes: Option<u64>,
) -> Result<(InstanceStore, bool), String> {
    let (root, name) = registry_parts(path)?;
    let mut importing = false;
    if let Some(mut conn) = open(root, true)? {
        let tx = conn.transaction().map_err(error)?;
        let state = state(&tx, name)?;
        if state.active {
            return Ok((read_active(&tx, name, state)?, true));
        }
        importing = state.stamp.is_some() || state.backup;
    }
    match legacy(path, max_legacy_bytes)? {
        Some(store) => Ok((store, false)),
        None if importing => Err(error("迁移期间旧实例登记不可用")),
        None => Ok((InstanceStore::new(), true)),
    }
}

pub(crate) fn active_revision(path: &Path) -> Result<u64, String> {
    let (root, name) = registry_parts(path)?;
    open(root, true)?
        .map(|conn| state(&conn, name).map(|state| state.revision))
        .transpose()
        .map(Option::unwrap_or_default)
}

fn preserve_unrepresented(next: &mut Value, previous: Value) {
    if let (Some(next), Value::Object(previous)) = (next.as_object_mut(), previous) {
        for (key, value) in previous {
            next.entry(key).or_insert(value);
        }
    }
}

pub(crate) fn save(path: &Path, store: &InstanceStore, preserve_extra: bool) -> Result<(), String> {
    let (root, name) = registry_parts(path)?;
    fs::create_dir_all(root).map_err(error)?;
    let _gate = cleanup_gate(root, false)?;
    let mut next_defaults = serde_json::to_value(&store.default_settings).map_err(error)?;
    let mut next_profiles = store
        .instances
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(error)?;
    let mut conn = writer(root)?;
    // Legacy parsing happens outside the write transaction.
    let legacy = if preserve_extra && !state(&conn, name)?.active {
        legacy(path, None)?
    } else {
        None
    };
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(error)?;
    let before = state(&tx, name)?;
    if store.storage_revision.get() != before.revision {
        return Err(error("实例登记已更新，请重试"));
    }
    if preserve_extra {
        let (old_defaults, old_profiles) = if before.active {
            (before.defaults.clone(), profiles(&tx, name)?)
        } else if let Some(RawStore(mut legacy)) = legacy {
            let mut profiles = legacy["instances"].take();
            (
                Some(legacy["defaultSettings"].take()),
                profiles
                    .as_array_mut()
                    .map(std::mem::take)
                    .unwrap_or_default(),
            )
        } else {
            (None, Vec::new())
        };
        if let Some(previous) = old_defaults {
            preserve_unrepresented(&mut next_defaults, previous);
        }
        let mut by_id: HashMap<String, Value> = old_profiles
            .into_iter()
            .filter_map(|profile| Some((profile["id"].as_str()?.to_owned(), profile)))
            .collect();
        for profile in &mut next_profiles {
            if let Some(previous) = profile["id"].as_str().and_then(|id| by_id.remove(id)) {
                preserve_unrepresented(profile, previous);
            }
        }
    }
    tx.execute(
        "DELETE FROM instance_profiles WHERE registry=?1 AND position>=?2",
        params![name, next_profiles.len()],
    )
    .map_err(error)?;
    for (position, profile) in next_profiles.iter().enumerate() {
        insert(&tx, name, position, profile)?;
    }
    let revision = before
        .revision
        .checked_add(1)
        .ok_or_else(|| error("实例登记版本超出范围"))?;
    set_state(
        &tx,
        name,
        &State {
            active: true,
            revision,
            defaults: Some(next_defaults),
            ..State::default()
        },
    )?;
    tx.commit().map_err(error)?;
    store.storage_revision.set(revision);
    Ok(())
}

pub(crate) type References = (HashSet<String>, Vec<(String, String)>);

pub(crate) fn active_references(root: &Path) -> Result<References, String> {
    let Some(mut conn) = open(root, true)? else {
        return Ok(References::default());
    };
    let tx = conn.transaction().map_err(error)?;
    let names = {
        let mut query = tx
            .prepare("SELECT registry FROM instance_registry WHERE active=1")
            .map_err(error)?;
        let rows = query.query_map([], |row| row.get(0)).map_err(error)?;
        rows.collect::<Result<HashSet<_>, _>>().map_err(error)?
    };
    {
        let mut query = tx
            .prepare("SELECT registry FROM instance_registry WHERE active=0")
            .map_err(error)?;
        let rows = query
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(error)?;
        for name in rows {
            let path = root.join(name.map_err(error)?);
            if !path.try_exists().map_err(error)?
                && !legacy_path(&path, true).try_exists().map_err(error)?
            {
                return Err(error("迁移期间旧实例登记不可用"));
            }
        }
    }
    let profiles = {
        let mut query = tx
            .prepare(
                "SELECT p.instance_id,p.user_data_dir FROM instance_profiles p
             JOIN instance_registry r USING(registry) WHERE r.active=1",
            )
            .map_err(error)?;
        let rows = query
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(error)?
    };
    Ok((names, profiles))
}

pub(crate) fn migrate_batch(path: &Path) -> Result<bool, String> {
    let (root, name) = registry_parts(path)?;
    if !path.try_exists().map_err(error)? && !legacy_path(path, true).try_exists().map_err(error)? {
        return Ok(true);
    }
    let mut conn = writer(root)?;
    let before = state(&conn, name)?;
    if before.active {
        return Ok(true);
    }
    let mut next = before.clone();
    let rows = match import_batch(path, &mut next) {
        Ok(rows) => rows,
        Err(ReadError::Format)
            if !next.backup && legacy_path(path, true).try_exists().map_err(error)? =>
        {
            next = State {
                backup: true,
                ..State::default()
            };
            Vec::new()
        }
        Err(ReadError::Format) => return Err(error("旧实例登记格式无效，已保留原文件")),
        Err(ReadError::Budget) => return Err(error("单条旧实例登记超出迁移预算，已保留原文件")),
        Err(ReadError::Io(cause)) => return Err(error(cause)),
    };
    let _gate = cleanup_gate(root, false)?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(error)?;
    if state(&tx, name)? != before {
        return Ok(false);
    }
    if next.backup != before.backup {
        tx.execute("DELETE FROM instance_profiles WHERE registry=?1", [name])
            .map_err(error)?;
    }
    for row in rows {
        insert(&tx, name, next.position, &row)?;
        next.position += 1;
    }
    set_state(&tx, name, &next)?;
    tx.commit().map_err(error)?;
    Ok(next.active)
}

fn import_batch(path: &Path, state: &mut State) -> Result<Vec<Value>, ReadError> {
    let file = match File::open(legacy_path(path, state.backup)) {
        Ok(file) => file,
        Err(cause)
            if cause.kind() == ErrorKind::NotFound && !state.backup && state.stamp.is_none() =>
        {
            return Err(ReadError::Format);
        }
        Err(cause) => return Err(cause.into()),
    };
    let metadata = file.metadata()?;
    let stamp = format!("{}:{:?}", metadata.len(), metadata.modified()?);
    if state.stamp.as_ref().is_some_and(|old| old != &stamp) {
        return Err(std::io::Error::other("旧实例登记在迁移期间发生变化").into());
    }
    state.stamp = Some(stamp);
    let mut input = Input::new(file, state.offset, BYTES)?;
    let start = state.offset;
    let started = Instant::now();
    let mut rows = Vec::new();
    if start == 0 {
        input.expect(b'{')?;
    }
    loop {
        match import_item(&mut input, state) {
            Ok(Some(row)) => rows.push(row),
            Ok(None) => {}
            Err(ReadError::Budget) if state.offset > start => break,
            Err(cause) => return Err(cause),
        }
        state.offset = input.offset;
        if state.active || rows.len() >= ROWS || input.budget == 0 || started.elapsed() >= TIME {
            return Ok(rows);
        }
    }
    Ok(rows)
}

fn import_item(input: &mut Input, state: &mut State) -> Result<Option<Value>, ReadError> {
    if input.peek()? == Some(if state.in_instances { b']' } else { b'}' }) {
        input.take()?;
        if state.in_instances {
            state.in_instances = false;
            state.next_item = true;
        } else {
            if !state.saw_instances || input.peek()?.is_some() {
                return Err(ReadError::Format);
            }
            state.active = true;
        }
        return Ok(None);
    }
    if state.next_item {
        input.expect(b',')?;
    }
    if state.in_instances {
        let row: Value = input.value()?;
        serde_json::from_value::<InstanceProfile>(row.clone()).map_err(|_| ReadError::Format)?;
        state.next_item = true;
        return Ok(Some(row));
    }
    let field: String = input.value()?;
    input.expect(b':')?;
    match field.as_str() {
        "instances" if !state.saw_instances => {
            input.expect(b'[')?;
            state.saw_instances = true;
            state.in_instances = true;
            state.next_item = false;
            return Ok(None);
        }
        "defaultSettings" if state.defaults.is_none() => {
            let raw: Value = input.value()?;
            serde_json::from_value::<DefaultInstanceSettings>(raw.clone())
                .map_err(|_| ReadError::Format)?;
            state.defaults = Some(raw);
        }
        "instances" | "defaultSettings" => return Err(ReadError::Format),
        _ => {
            input.value::<serde::de::IgnoredAny>()?;
        }
    }
    state.next_item = true;
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture(root: &Path, count: usize) -> Value {
        let profiles: Vec<_> = (0..count)
            .map(|i| {
                json!({
                    "id": format!("instance-{i}"),
                    "name": format!("Instance {i}"),
                    "userDataDir": root.join(format!("profile-{i}")),
                    "extraArgs": "--example",
                    "bindAccountId": format!("account-{i}"),
                    "createdAt": i,
                    "lastPid": 22
                })
            })
            .collect();
        json!({
            "instances": profiles,
            "defaultSettings": {
                "bindAccountId": "default",
                "followLocalAccount": false,
                "lastPid": 23
            }
        })
    }

    fn finish(path: &Path) {
        for _ in 0..12 {
            if migrate_batch(path).unwrap() {
                return;
            }
        }
        panic!("migration did not finish");
    }

    #[test]
    fn instance_registry_resumes_and_preserves_bindings_and_default_pid() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("codex_instances.json");
        let bytes = serde_json::to_vec(&fixture(root.path(), 78)).unwrap();
        fs::write(&path, &bytes).unwrap();
        let (old, complete) = read(&path, None).unwrap();
        assert!(!complete);
        assert!(!root.path().join(DATABASE).exists());
        assert_eq!(old.instances.len(), 78);

        assert!(!migrate_batch(&path).unwrap());
        assert!(active_references(root.path()).unwrap().1.is_empty());
        finish(&path);
        let (next, complete) = read(&path, None).unwrap();
        assert!(complete);
        assert_eq!(next.instances.len(), 78);
        assert_eq!(next.instances[77].id, "instance-77");
        assert_eq!(
            next.instances[77].bind_account_id.as_deref(),
            Some("account-77")
        );
        assert_eq!(
            next.default_settings.bind_account_id.as_deref(),
            Some("default")
        );
        assert_eq!(next.default_settings.last_pid, Some(23));
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }

    #[test]
    fn instance_registry_edit_and_removal_win_over_migration_and_stale_save() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("codex_instances.json");
        fs::write(
            &path,
            serde_json::to_vec(&fixture(root.path(), 78)).unwrap(),
        )
        .unwrap();
        assert!(!migrate_batch(&path).unwrap());
        let (mut edit, _) = read(&path, None).unwrap();
        let stale = edit.clone();
        edit.instances.remove(0);
        edit.instances[0].bind_account_id = None;
        save(&path, &edit, false).unwrap();
        assert!(migrate_batch(&path).unwrap());
        assert!(save(&path, &stale, false).is_err());
        let (next, complete) = read(&path, None).unwrap();
        assert!(complete);
        assert_eq!(next.instances.len(), 77);
        assert_eq!(next.instances[0].id, "instance-1");
        assert_eq!(next.instances[0].bind_account_id, None);
        assert!(!active_references(root.path())
            .unwrap()
            .1
            .iter()
            .any(|(id, _)| id == "instance-0"));
    }

    #[test]
    fn instance_registry_cli_edits_preserve_desktop_fields_from_backup() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("codex_instances.json");
        let mut raw = fixture(root.path(), 1);
        raw["defaultSettings"]["futureDefaultField"] = json!(true);
        raw["instances"][0]["futureProfileField"] = json!("keep");
        fs::write(&path, "{broken").unwrap();
        fs::write(legacy_path(&path, true), serde_json::to_vec(&raw).unwrap()).unwrap();
        let (mut edit, _) = read(&path, None).unwrap();
        edit.instances[0].bind_account_id = Some("updated".into());
        save(&path, &edit, true).unwrap();
        let conn = writer(root.path()).unwrap();
        let current = state(&conn, "codex_instances.json").unwrap();
        assert_eq!(current.defaults.unwrap()["futureDefaultField"], true);
        let profile = profiles::<Value>(&conn, "codex_instances.json")
            .unwrap()
            .remove(0);
        assert_eq!(profile["futureProfileField"], "keep");
        assert_eq!(profile["bindAccountId"], "updated");
        assert_eq!(fs::read_to_string(&path).unwrap(), "{broken");
    }

    #[test]
    fn instance_registry_missing_source_after_checkpoint_is_an_error() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("codex_instances.json");
        fs::write(
            &path,
            serde_json::to_vec(&fixture(root.path(), 78)).unwrap(),
        )
        .unwrap();
        assert!(!migrate_batch(&path).unwrap());
        fs::remove_file(&path).unwrap();
        assert!(read(&path, None).is_err());
        assert!(active_references(root.path()).is_err());
    }

    #[test]
    fn instance_registry_oversized_profile_keeps_legacy_available() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("codex_instances.json");
        let mut raw = fixture(root.path(), 2);
        raw["instances"][1]["extraArgs"] = json!("x".repeat(BYTES as usize + 1));
        fs::write(&path, serde_json::to_vec(&raw).unwrap()).unwrap();
        assert!(!migrate_batch(&path).unwrap());
        assert!(migrate_batch(&path).unwrap_err().contains("超出迁移预算"));
        assert!(active_references(root.path()).unwrap().1.is_empty());
        let (old, complete) = read(&path, None).unwrap();
        assert!(!complete);
        assert_eq!(old.instances.len(), 2);
        assert!(old.instances[1].extra_args.len() as u64 > BYTES);
    }

    #[test]
    fn instance_registry_cleanup_gate_excludes_writes_until_rename_is_safe() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("codex_instances.json");
        let exclusive = cleanup_gate(root.path(), true).unwrap();
        assert!(save(&path, &InstanceStore::new(), false).is_err());
        assert!(!root.path().join(DATABASE).exists());
        drop(exclusive);
        save(&path, &InstanceStore::new(), false).unwrap();
    }
}
