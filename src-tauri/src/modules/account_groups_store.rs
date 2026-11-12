//! Platform-scoped groups and ordered membership, with read-only legacy import.
use super::{
    background_maintenance as maintenance,
    legacy_json_input::{Input, ReadError},
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashSet, VecDeque},
    fs::{self, File},
    io::{BufReader, ErrorKind},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub(crate) const ANTIGRAVITY: &str = "antigravity";
pub(crate) const CODEX: &str = "codex";
const CONFLICT: &str = "ACCOUNT_GROUPS_CHANGED";
const DATABASE: &str = "account_groups.sqlite";

pub(crate) fn filename(platform: &str) -> String {
    let lower = platform.trim().to_ascii_lowercase();
    match lower.as_str() {
        "antigravity" | "gemini" => "account_groups.json".into(),
        "codex" => "codex_account_groups.json".into(),
        "claude" => "claude_manager_account_groups.json".into(),
        other => {
            let safe: String = other
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            format!("{safe}_account_groups.json")
        }
    }
}

pub(crate) fn store(platform: &str) -> Result<GroupStore, String> {
    Ok(GroupStore::new(
        super::account::get_data_dir()?,
        filename(platform),
    ))
}

pub(crate) fn schedule_legacy_groups() -> Result<(), String> {
    let root = super::account::get_data_dir()?;
    let mut pending = VecDeque::new();
    let mut seen = HashSet::new();
    for entry in fs::read_dir(&root).map_err(error)? {
        let entry = entry.map_err(error)?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let scope = name.strip_suffix(".bak").unwrap_or(name);
        if (scope == "account_groups.json" || scope.ends_with("_account_groups.json"))
            && entry.file_type().map_err(error)?.is_file()
            && seen.insert(scope.to_string())
        {
            pending.push_back((GroupStore::new(root.clone(), scope.to_string()), 0));
        }
    }
    if !pending.is_empty() {
        maintenance::schedule(
            format!("account-groups:startup:{}", root.display()),
            move || {
                let Some((store, failures)) = pending.pop_front() else {
                    return Ok(true);
                };
                match store.migrate_batch() {
                    Ok(true) => {}
                    Ok(false) => pending.push_back((store, 0)),
                    Err(error) => {
                        super::logger::log_warn(&format!(
                            "[AccountGroups] 分组迁移失败: scope={}, attempt={}, error={error}",
                            store.scope,
                            failures + 1,
                        ));
                        if failures < 4 {
                            pending.push_back((store, failures + 1));
                        }
                    }
                }
                Ok(pending.is_empty())
            },
        );
    }
    Ok(())
}

#[derive(Clone)]
pub(crate) struct GroupStore {
    root: PathBuf,
    scope: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GroupSnapshot {
    pub data: String,
    pub revision: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct State {
    revision: u64,
    complete: bool,
    present: bool,
    source: i64,
    stamp: Option<String>,
    offset: u64,
    position: usize,
}

impl GroupStore {
    pub(super) fn new(root: PathBuf, scope: String) -> Self {
        Self { root, scope }
    }

    fn legacy(&self, source: i64) -> PathBuf {
        let primary = self.root.join(&self.scope);
        if source == 2 {
            primary.with_extension("json.bak")
        } else {
            primary
        }
    }

    fn open(&self) -> Result<Connection, String> {
        let conn = Connection::open(self.root.join(DATABASE)).map_err(error)?;
        conn.busy_timeout(Duration::from_millis(120))
            .map_err(error)?;
        conn.pragma_update(None, "foreign_keys", true)
            .map_err(error)?;
        let version: u32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(error)?;
        match version {
            1 => {}
            0 => conn.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE IF NOT EXISTS collections (
                    scope TEXT PRIMARY KEY, state TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS account_groups (
                    scope TEXT NOT NULL, source INTEGER NOT NULL, position INTEGER NOT NULL,
                    group_id TEXT, name_key TEXT, sort_order INTEGER, metadata TEXT NOT NULL, has_members INTEGER NOT NULL,
                    PRIMARY KEY(scope, source, position)
                 );
                 CREATE INDEX IF NOT EXISTS group_by_id ON account_groups(scope, source, group_id);
                 CREATE INDEX IF NOT EXISTS group_by_name ON account_groups(scope, source, name_key);
                 CREATE TABLE IF NOT EXISTS group_members (
                    scope TEXT NOT NULL, source INTEGER NOT NULL, position INTEGER NOT NULL,
                    member_position INTEGER NOT NULL, account_key TEXT, payload TEXT NOT NULL,
                    PRIMARY KEY(scope, source, position, member_position),
                    FOREIGN KEY(scope, source, position)
                        REFERENCES account_groups(scope, source, position) ON DELETE CASCADE
                 );
                 CREATE INDEX IF NOT EXISTS group_by_account ON group_members(scope, source, account_key);
                 PRAGMA user_version=1;
                 COMMIT;",
            ).map_err(error)?,
            _ => return Err("本地账号分组数据库版本不受支持".into()),
        }
        Ok(conn)
    }

    fn state(&self, conn: &Connection) -> Result<State, String> {
        let row: Option<String> = conn
            .query_row(
                "SELECT state FROM collections WHERE scope=?1",
                [&self.scope],
                |row| row.get(0),
            )
            .optional()
            .map_err(error)?;
        row.map(|row| serde_json::from_str(&row).map_err(error))
            .transpose()
            .map(Option::unwrap_or_default)
    }

    fn set_state(&self, conn: &Connection, state: &State) -> Result<(), String> {
        conn.execute(
            "INSERT INTO collections(scope,state) VALUES (?1,?2)
             ON CONFLICT(scope) DO UPDATE SET state=excluded.state",
            params![self.scope, encode(state)?],
        )
        .map_err(error)?;
        Ok(())
    }

    fn read_legacy(&self, state: &State) -> Result<Option<Vec<Value>>, String> {
        if state.source == 2 {
            return read_snapshot(&self.legacy(2))?
                .ok_or_else(|| "旧账号分组备份不可读".into())
                .map(Some);
        }
        match read_snapshot(&self.legacy(1)) {
            Ok(Some(rows)) => Ok(Some(rows)),
            Ok(None) => match read_snapshot(&self.legacy(2))? {
                None if state.stamp.is_some() => Err("迁移期间旧账号分组文件不可用".into()),
                backup => Ok(backup),
            },
            Err(primary) => match read_snapshot(&self.legacy(2)) {
                Ok(Some(rows)) => Ok(Some(rows)),
                _ => Err(primary),
            },
        }
    }

    pub(crate) fn snapshot(&self) -> Result<GroupSnapshot, String> {
        let (state, rows) = self.load()?;
        Ok(GroupSnapshot {
            data: encode(&rows.unwrap_or_default())?,
            revision: state.revision,
        })
    }

    fn load(&self) -> Result<(State, Option<Vec<Value>>), String> {
        let mut conn = self.open()?;
        let tx = conn.transaction().map_err(error)?;
        let state = self.state(&tx)?;
        if state.complete {
            let rows = if state.present {
                Some(self.read_db(&tx, state.source)?)
            } else {
                None
            };
            tx.commit().map_err(error)?;
            return Ok((state, rows));
        }
        tx.commit().map_err(error)?;
        self.schedule();
        let rows = self.read_legacy(&state)?;
        Ok((state, rows))
    }

    pub(crate) fn groups_for_member(&self, account_id: &str) -> Result<Vec<Value>, String> {
        let mut conn = self.open()?;
        let tx = conn.transaction().map_err(error)?;
        let state = self.state(&tx)?;
        if !state.complete {
            tx.commit().map_err(error)?;
            self.schedule();
            return Ok(self
                .read_legacy(&state)?
                .unwrap_or_default()
                .into_iter()
                .filter(|group| {
                    group
                        .get("accountIds")
                        .and_then(Value::as_array)
                        .is_some_and(|members| {
                            members
                                .iter()
                                .any(|id| id.as_str().map(str::trim) == Some(account_id))
                        })
                })
                .collect());
        }
        let mut query = tx
            .prepare(
                "SELECT DISTINCT g.position,g.metadata FROM group_members m
             JOIN account_groups g USING(scope,source,position)
             WHERE m.scope=?1 AND m.source=?2 AND m.account_key=?3 ORDER BY g.position",
            )
            .map_err(error)?;
        let rows = query
            .query_map(params![self.scope, state.source, account_id], |row| {
                row.get::<_, String>(1)
            })
            .map_err(error)?;
        rows.map(|row| serde_json::from_str(&row.map_err(error)?).map_err(error))
            .collect()
    }

    fn read_db(&self, conn: &Connection, source: i64) -> Result<Vec<Value>, String> {
        let mut query = conn.prepare(
            "SELECT metadata, has_members FROM account_groups WHERE scope=?1 AND source=?2 ORDER BY position"
        ).map_err(error)?;
        let groups = query
            .query_map(params![self.scope, source], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?))
            })
            .map_err(error)?;
        let mut result = Vec::new();
        for group in groups {
            let (metadata, has_members) = group.map_err(error)?;
            let mut value: Value = serde_json::from_str(&metadata).map_err(error)?;
            if has_members {
                value["accountIds"] = Value::Array(Vec::new());
            }
            result.push(value);
        }

        let mut query = conn
            .prepare(
                "SELECT position,payload FROM group_members WHERE scope=?1 AND source=?2
             ORDER BY position,member_position",
            )
            .map_err(error)?;
        let members = query
            .query_map(params![self.scope, source], |row| {
                Ok((row.get::<_, usize>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(error)?;
        for member in members {
            let (position, payload) = member.map_err(error)?;
            let accounts = result
                .get_mut(position)
                .and_then(|group| group.get_mut("accountIds"))
                .and_then(Value::as_array_mut)
                .ok_or("账号分组成员引用无效")?;
            accounts.push(serde_json::from_str(&payload).map_err(error)?);
        }
        Ok(result)
    }

    pub(crate) fn replace_json(&self, data: &str, expected: Option<u64>) -> Result<u64, String> {
        self.replace(&parse_groups(data)?, expected, false)
    }

    pub(crate) fn seed_json(&self, data: &str) -> Result<(), String> {
        let (state, groups) = self.load()?;
        if groups.is_none() {
            match self.replace(&parse_groups(data)?, Some(state.revision), true) {
                Err(error) if error == CONFLICT => {}
                result => {
                    result?;
                }
            }
        }
        Ok(())
    }

    /// Assign an imported account by exact name. The completed database uses
    /// the name and membership indexes instead of rewriting the collection.
    pub(crate) fn assign_member(
        &self,
        account_id: &str,
        group_name: &str,
        create: impl Fn(i64) -> Value,
    ) -> Result<(), String> {
        let mut conn = self.open()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(error)?;
        let mut state = self.state(&tx)?;
        if !state.complete || !state.present {
            drop(tx);
            return self.edit(|groups| {
                for group in groups.iter_mut() {
                    if let Some(members) = group.get_mut("accountIds").and_then(Value::as_array_mut)
                    {
                        members.retain(|member| member.as_str().map(str::trim) != Some(account_id));
                    }
                }
                if let Some(group) = groups.iter_mut().find(|group| {
                    group.get("name").and_then(Value::as_str).map(str::trim) == Some(group_name)
                }) {
                    if let Some(members) = group.get_mut("accountIds").and_then(Value::as_array_mut)
                    {
                        members.push(Value::String(account_id.into()));
                    }
                } else {
                    let last_order = groups
                        .iter()
                        .filter_map(|group| group.get("sortOrder").and_then(Value::as_i64))
                        .max()
                        .unwrap_or(-1);
                    groups.push(create(last_order.saturating_add(1)));
                }
            });
        }
        let source = state.source;
        tx.execute(
            "DELETE FROM group_members WHERE scope=?1 AND source=?2 AND account_key=?3",
            params![self.scope, source, account_id],
        )
        .map_err(error)?;
        let existing: Option<(usize, bool)> = tx
            .query_row(
                "SELECT position,has_members FROM account_groups WHERE scope=?1 AND source=?2
             AND name_key=?3 ORDER BY position LIMIT 1",
                params![self.scope, source, group_name],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(error)?;
        match existing {
            Some((position, true)) => {
                tx.execute(
                    "INSERT INTO group_members(scope,source,position,member_position,account_key,payload)
                     SELECT ?1,?2,?3,COALESCE(MAX(member_position),-1)+1,?4,?5
                     FROM group_members WHERE scope=?1 AND source=?2 AND position=?3",
                    params![self.scope, source, position, account_id, encode(&account_id)?],
                ).map_err(error)?;
            }
            Some((_, false)) => {}
            None => {
                let (position, sort_order): (usize, i64) = tx
                    .query_row(
                        "SELECT COALESCE(MAX(position),-1)+1,COALESCE(MAX(sort_order),-1)
                     FROM account_groups WHERE scope=?1 AND source=?2",
                        params![self.scope, source],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .map_err(error)?;
                self.put_group(&tx, source, position, create(sort_order.saturating_add(1)))?;
            }
        }
        state.revision = state.revision.checked_add(1).ok_or("账号分组版本溢出")?;
        self.set_state(&tx, &state)?;
        tx.commit().map_err(error)
    }

    fn edit(&self, update: impl Fn(&mut Vec<Value>)) -> Result<(), String> {
        for _ in 0..3 {
            let (state, groups) = self.load()?;
            let mut groups = groups.unwrap_or_default();
            update(&mut groups);
            match self.replace(&groups, Some(state.revision), false) {
                Err(error) if error == CONFLICT => continue,
                result => return result.map(|_| ()),
            }
        }
        Err(CONFLICT.into())
    }

    fn replace(
        &self,
        rows: &[Value],
        expected: Option<u64>,
        only_absent: bool,
    ) -> Result<u64, String> {
        let mut conn = self.open()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(error)?;
        let state = self.state(&tx)?;
        if expected.is_some_and(|revision| revision != state.revision)
            || (only_absent && state.present)
        {
            return Err(CONFLICT.into());
        }
        tx.execute(
            "DELETE FROM account_groups WHERE scope=?1 AND source=0",
            [&self.scope],
        )
        .map_err(error)?;
        for (position, row) in rows.iter().enumerate() {
            self.put_group(&tx, 0, position, row.clone())?;
        }
        let revision = state.revision.checked_add(1).ok_or("账号分组版本溢出")?;
        self.set_state(
            &tx,
            &State {
                revision,
                complete: true,
                present: true,
                ..State::default()
            },
        )?;
        tx.commit().map_err(error)?;
        Ok(revision)
    }

    fn put_group(
        &self,
        conn: &Connection,
        source: i64,
        position: usize,
        mut row: Value,
    ) -> Result<(), String> {
        let group = row.as_object_mut().ok_or("账号分组项必须是对象")?;
        let members = group.remove("accountIds");
        let group_id = group.get("id").and_then(Value::as_str);
        let name_key = group.get("name").and_then(Value::as_str).map(str::trim);
        let sort_order = group.get("sortOrder").and_then(Value::as_i64);
        conn.execute(
            "INSERT INTO account_groups(scope,source,position,group_id,name_key,sort_order,metadata,has_members)
                VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![self.scope, source, position, group_id, name_key, sort_order, encode(group)?, members.is_some()],
        ).map_err(error)?;
        if let Some(members) = members {
            let keys: Vec<_> = members
                .as_array()
                .ok_or("账号分组成员格式无效")?
                .iter()
                .map(|member| member.as_str().map(str::trim))
                .collect();
            conn.execute(
                "INSERT INTO group_members(scope,source,position,member_position,account_key,payload)
                 SELECT ?1,?2,?3,CAST(key AS INTEGER),
                    ?5 ->> CAST(key AS INTEGER),
                    ?4 -> CAST(key AS INTEGER) FROM json_each(?4)",
                params![self.scope, source, position, encode(&members)?, encode(&keys)?],
            ).map_err(error)?;
        }
        Ok(())
    }

    fn schedule(&self) {
        let store = self.clone();
        maintenance::schedule(
            format!("account-groups:{}", self.root.join(&self.scope).display()),
            move || store.migrate_batch(),
        );
    }

    fn migrate_batch(&self) -> Result<bool, String> {
        let mut conn = self.open()?;
        let before = self.state(&conn)?;
        if before.complete {
            return Ok(true);
        }

        let mut next = before.clone();
        if next.source == 0 {
            next.source = 1;
        }
        let mut groups = Vec::new();
        let path = self.legacy(next.source);
        match File::open(path) {
            Ok(file) => {
                let metadata = file.metadata().map_err(error)?;
                let stamp = format!(
                    "{}:{:?}",
                    metadata.len(),
                    metadata.modified().map_err(error)?
                );
                if next.stamp.as_ref().is_some_and(|stored| stored != &stamp) {
                    return Err("迁移期间旧账号分组文件发生变化".into());
                }
                next.stamp = Some(stamp);
                next.present = true;
                let mut input =
                    Input::new(file, next.offset, maintenance::BATCH_BYTES).map_err(error)?;
                match read_batch(&mut input, &mut next) {
                    Ok(rows) => groups = rows,
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
        if self.state(&tx)? != before {
            return Ok(false);
        }
        for row in groups {
            self.put_group(&tx, next.source, next.position, row)?;
            next.position += 1;
        }
        self.set_state(&tx, &next)?;
        tx.commit().map_err(error)?;
        Ok(next.complete)
    }
}

fn valid_group(row: &Value) -> bool {
    row.as_object()
        .is_some_and(|group| group.get("accountIds").is_none_or(Value::is_array))
}

pub(crate) fn parse_groups(data: &str) -> Result<Vec<Value>, String> {
    let rows: Vec<Value> = serde_json::from_str(data).map_err(error)?;
    if let Some(index) = rows.iter().position(|row| !valid_group(row)) {
        return Err(format!("账号分组项 {index} 或成员格式无效"));
    }
    Ok(rows)
}

fn read_snapshot(path: &Path) -> Result<Option<Vec<Value>>, String> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(cause) if cause.kind() == ErrorKind::NotFound => return Ok(None),
        Err(cause) => return Err(error(cause)),
    };
    let rows: Vec<Value> = serde_json::from_reader(BufReader::new(file)).map_err(error)?;
    if rows.iter().any(|row| !valid_group(row)) {
        return Err("旧账号分组项或成员格式无效".into());
    }
    Ok(Some(rows))
}

fn read_batch(input: &mut Input, state: &mut State) -> Result<Vec<Value>, ReadError> {
    let mut rows = Vec::new();
    let started = Instant::now();
    let start = state.offset;
    if start == 0 {
        input.expect(b'[')?;
    }
    loop {
        let result = (|| {
            if input.peek()? == Some(b']') {
                input.take()?;
                if input.peek()?.is_some() {
                    return Err(ReadError::Format);
                }
                return Ok(None);
            }
            if state.position + rows.len() > 0 {
                input.expect(b',')?;
            }
            let row = input.value::<Value>()?;
            if !valid_group(&row) {
                return Err(ReadError::Format);
            }
            Ok(Some(row))
        })();
        match result {
            Ok(Some(row)) => rows.push(row),
            Ok(None) => state.complete = true,
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

fn encode(value: &impl Serialize) -> Result<String, String> {
    serde_json::to_string(value).map_err(error)
}
fn error(cause: impl std::fmt::Display) -> String {
    format!("读写账号分组失败: {cause}")
}
fn legacy_error(cause: ReadError) -> String {
    match cause {
        ReadError::Budget => "旧账号分组单项超出迁移预算，原文件已保留".into(),
        ReadError::Format => "旧账号分组格式无效，原文件已保留".into(),
        ReadError::Io(cause) => error(cause),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(id: usize) -> Value {
        serde_json::json!({
            "id": format!("group-{id}"), "name": format!("Name {id}"), "createdAt": id,
            "sortOrder": id * 10, "accountIds": ["a", format!("member-{id}"), "a"],
            "quotaAutoRefreshMinutes": if id % 2 == 0 { -1 } else { 37 },
            "extraMetadata": { "preserve": [true, null, {"x": "y"}] },
        })
    }

    fn read(store: &GroupStore) -> Vec<Value> {
        parse_groups(&store.snapshot().unwrap().data).unwrap()
    }

    fn finish(store: &GroupStore) {
        for _ in 0..20 {
            if store.migrate_batch().unwrap() {
                return;
            }
        }
        panic!("migration did not complete");
    }

    #[test]
    fn account_groups_resume_preserves_group_order_members_platform_policy_and_source() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(filename(CODEX));
        let original: Vec<_> = (0..75).map(sample).collect();
        let bytes = serde_json::to_vec_pretty(&original).unwrap();
        fs::write(&path, &bytes).unwrap();
        let store = GroupStore::new(dir.path().into(), filename(CODEX));
        assert!(!store.migrate_batch().unwrap());
        let progress = store.state(&store.open().unwrap()).unwrap();
        assert!(progress.offset > 0);
        assert!(progress.position < original.len());

        let restarted = GroupStore::new(dir.path().into(), filename(CODEX));
        assert_eq!(
            restarted.state(&restarted.open().unwrap()).unwrap(),
            progress
        );
        finish(&restarted);
        assert_eq!(read(&restarted), original);
        let members = restarted.groups_for_member("member-74").unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0]["quotaAutoRefreshMinutes"], -1);
        assert_eq!(fs::read(&path).unwrap(), bytes);
        fs::write(&path, "invalid after cutover").unwrap();
        assert_eq!(read(&restarted), original);
    }

    #[test]
    fn account_groups_edit_and_deletion_win_over_import_and_stale_page_snapshots() {
        let dir = tempfile::tempdir().unwrap();
        let codex = GroupStore::new(dir.path().into(), filename(CODEX));
        let cursor = GroupStore::new(dir.path().into(), filename("cursor"));
        let original: Vec<_> = (0..50).map(sample).collect();
        fs::write(codex.legacy(1), encode(&original).unwrap()).unwrap();
        assert!(!codex.migrate_batch().unwrap());
        let first = codex.snapshot().unwrap();
        let create = |order| {
            serde_json::json!({
                "id": "new-group", "name": "New", "sortOrder": order, "accountIds": ["extra"]
            })
        };
        codex.assign_member("extra", "New", create).unwrap();
        assert_eq!(
            codex.replace_json("[]", Some(first.revision)).unwrap_err(),
            CONFLICT
        );
        assert!(codex.migrate_batch().unwrap());
        assert_eq!(
            read(&codex).last().unwrap()["accountIds"],
            serde_json::json!(["extra"])
        );

        codex
            .assign_member("extra", "Name 2", |_| unreachable!())
            .unwrap();
        let result = read(&codex);
        assert_eq!(
            result[2]["accountIds"],
            serde_json::json!(["a", "member-2", "a", "extra"])
        );
        assert_eq!(result.last().unwrap()["accountIds"], serde_json::json!([]));
        assert_eq!(
            codex.groups_for_member("extra").unwrap()[0]["id"],
            "group-2"
        );

        let edited = codex.snapshot().unwrap();
        codex.replace_json("[]", Some(edited.revision)).unwrap();
        assert!(codex.migrate_batch().unwrap());
        assert!(read(&codex).is_empty());
        assert!(read(&cursor).is_empty());
    }

    #[test]
    fn account_groups_seed_only_fills_absence_and_uses_valid_read_only_backup() {
        let dir = tempfile::tempdir().unwrap();
        let store = GroupStore::new(dir.path().into(), filename(ANTIGRAVITY));
        fs::write(store.legacy(1), "invalid").unwrap();
        let backup = encode(&vec![sample(7)]).unwrap();
        fs::write(store.legacy(2), &backup).unwrap();
        finish(&store);
        assert_eq!(read(&store), vec![sample(7)]);
        store.seed_json(&encode(&vec![sample(8)]).unwrap()).unwrap();
        assert_eq!(read(&store), vec![sample(7)]);
        assert_eq!(fs::read_to_string(store.legacy(1)).unwrap(), "invalid");
        assert_eq!(fs::read_to_string(store.legacy(2)).unwrap(), backup);

        let absent = GroupStore::new(dir.path().into(), filename("windsurf"));
        finish(&absent);
        absent
            .seed_json(&encode(&vec![sample(8)]).unwrap())
            .unwrap();
        assert_eq!(read(&absent), vec![sample(8)]);
        absent.replace_json("[]", None).unwrap();
        absent
            .seed_json(&encode(&vec![sample(9)]).unwrap())
            .unwrap();
        assert!(read(&absent).is_empty());
    }
}
