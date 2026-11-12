//! Rules belong to the user, while account execution state belongs to the scheduler.
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::ErrorKind,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use super::{
    atomic_write, background_maintenance,
    legacy_json_input::{Input, ReadError},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSchedule {
    pub scheduled_date: String,
    pub scheduled_minute: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_checked_date: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckinConfig {
    pub enabled: bool,
    pub start_time: String,
    pub end_time: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_checked_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_schedules: Option<HashMap<String, AccountSchedule>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_ids: Option<HashMap<String, String>>,
}

impl Default for CheckinConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            start_time: "06:00".into(),
            end_time: "12:00".into(),
            last_checked_date: None,
            account_schedules: None,
            device_ids: None,
        }
    }
}

#[derive(Clone)]
pub(super) struct CheckinStore {
    legacy: PathBuf,
    platform: &'static str,
}

const DATABASE: &str = "checkin_state.sqlite";

impl CheckinStore {
    pub(super) fn new(legacy: PathBuf, platform: &'static str) -> Self {
        Self { legacy, platform }
    }

    fn rules_path(&self) -> PathBuf {
        self.legacy.with_extension("rules.json")
    }

    fn open(&self) -> Result<Connection, String> {
        let directory = self.legacy.parent().ok_or("无法定位自动签到目录")?;
        fs::create_dir_all(directory).map_err(storage_error)?;
        let conn = Connection::open(directory.join(DATABASE)).map_err(storage_error)?;
        conn.busy_timeout(Duration::from_millis(100))
            .map_err(storage_error)?;
        let version: u32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(storage_error)?;
        match version {
            1 => {}
            0 => conn
                .execute_batch(
                    "BEGIN IMMEDIATE;
                     CREATE TABLE IF NOT EXISTS schedules (
                        platform TEXT NOT NULL, account_id TEXT NOT NULL,
                        scheduled_date TEXT NOT NULL, scheduled_minute INTEGER NOT NULL,
                        last_checked_date TEXT,
                        PRIMARY KEY(platform, account_id)
                     );
                     CREATE TABLE IF NOT EXISTS devices (
                        platform TEXT NOT NULL, account_id TEXT NOT NULL, device_id TEXT NOT NULL,
                        PRIMARY KEY(platform, account_id)
                     );
                     CREATE INDEX IF NOT EXISTS devices_by_id ON devices(platform, device_id);
                     CREATE TABLE IF NOT EXISTS migration (
                        platform TEXT PRIMARY KEY, stamp TEXT NOT NULL,
                        cursor TEXT NOT NULL, complete INTEGER NOT NULL,
                        last_checked_date TEXT
                     );
                     PRAGMA user_version=1;
                     COMMIT;",
                )
                .map_err(storage_error)?,
            _ => return Err("本地签到状态数据库版本不受支持".into()),
        }
        Ok(conn)
    }

    fn progress(&self, conn: &Connection) -> Result<Option<Progress>, String> {
        let stored: Option<(String, String, bool, Option<String>)> = conn
            .query_row(
                "SELECT stamp,cursor,complete,last_checked_date FROM migration WHERE platform=?1",
                [self.platform],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()
            .map_err(storage_error)?;
        stored
            .map(|(stamp, cursor, complete, last_checked_date)| {
                Ok(Progress {
                    stamp,
                    cursor: serde_json::from_str(&cursor).map_err(storage_error)?,
                    complete,
                    last_checked_date,
                })
            })
            .transpose()
    }

    fn legacy_config(&self) -> Result<Option<(PathBuf, CheckinConfig)>, String> {
        match read_config(&self.legacy) {
            Ok(config) => Ok(config.map(|config| (self.legacy.clone(), config))),
            Err(original_error) => {
                let backup = self.legacy.with_extension("json.bak");
                match read_config(&backup) {
                    Ok(Some(config)) => Ok(Some((backup, config))),
                    _ => Err(original_error),
                }
            }
        }
    }

    pub(super) fn load(&self) -> Result<CheckinConfig, String> {
        let mut conn = self.open()?;
        let complete = self.progress(&conn)?.is_some_and(|state| state.complete);
        let legacy = if complete {
            None
        } else {
            self.legacy_config()?
        };
        let rules = read_rules(&self.rules_path())?;
        let (mut config, source) = match legacy {
            Some((source, config)) => {
                let rules = Rules::from(&config);
                (config, Some((source, rules)))
            }
            None => (CheckinConfig::default(), None),
        };
        if let Some(rules) = rules {
            config.enabled = rules.enabled;
            config.start_time = rules.start_time;
            config.end_time = rules.end_time;
        }
        let tx = conn.transaction().map_err(storage_error)?;
        let progress = self.progress(&tx)?;
        if let Some(state) = &progress {
            if state.complete {
                config.last_checked_date = state.last_checked_date.clone();
            }
        }
        {
            let mut query = tx
                .prepare(
                    "SELECT account_id,scheduled_date,scheduled_minute,last_checked_date
                       FROM schedules WHERE platform=?1",
                )
                .map_err(storage_error)?;
            let stored = query
                .query_map([self.platform], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        AccountSchedule {
                            scheduled_date: row.get(1)?,
                            scheduled_minute: row.get(2)?,
                            last_checked_date: row.get(3)?,
                        },
                    ))
                })
                .map_err(storage_error)?;
            for row in stored {
                let (id, schedule) = row.map_err(storage_error)?;
                config
                    .account_schedules
                    .get_or_insert_with(HashMap::new)
                    .insert(id, schedule);
            }
        }
        if self.platform == "trae-cn" {
            let mut query = tx
                .prepare("SELECT account_id,device_id FROM devices WHERE platform=?1")
                .map_err(storage_error)?;
            let stored = query
                .query_map([self.platform], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(storage_error)?;
            for row in stored {
                let (id, device) = row.map_err(storage_error)?;
                config
                    .device_ids
                    .get_or_insert_with(HashMap::new)
                    .insert(id, device);
            }
        }
        tx.commit().map_err(storage_error)?;
        if let Some((source, rules)) = source {
            if progress.is_none_or(|state| !state.complete) {
                self.schedule(source, rules);
            }
        }
        Ok(config)
    }

    pub(super) fn save_rules(&self, config: &CheckinConfig) -> Result<(), String> {
        let rules = Rules::from(config);
        rules.validate()?;
        atomic_write::write_string_atomic(&self.rules_path(), &to_json(&rules)?)
    }

    pub(super) fn seed_webview(&self, config: &CheckinConfig) -> Result<CheckinConfig, String> {
        // A backend file or a newer rules edit wins over the old WebView snapshot.
        if read_rules(&self.rules_path())?.is_none()
            && !self
                .progress(&self.open()?)?
                .is_some_and(|state| state.complete)
            && self.legacy_config()?.is_none()
        {
            validate_config(config)?;
            atomic_write::replace_secret_string_if_matches(
                &self.legacy,
                None,
                Some(&to_json(config)?),
            )?;
        }
        self.load()
    }

    pub(super) fn save_schedules(
        &self,
        before: Option<&HashMap<String, AccountSchedule>>,
        schedules: Option<HashMap<String, AccountSchedule>>,
    ) -> Result<(), String> {
        let mut changed = schedules
            .into_iter()
            .flatten()
            .filter(|(id, schedule)| before.and_then(|old| old.get(id)) != Some(schedule))
            .peekable();
        if changed.peek().is_none() {
            return Ok(());
        }
        let mut conn = self.open()?;
        loop {
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(storage_error)?;
            for (id, state) in changed.by_ref().take(background_maintenance::BATCH_ROWS) {
                put_schedule(&tx, self.platform, &id, &state, true)?;
            }
            tx.commit().map_err(storage_error)?;
            if changed.peek().is_none() {
                return Ok(());
            }
            std::thread::yield_now();
        }
    }

    pub(super) fn device_id(
        &self,
        account_id: &str,
        valid: impl Fn(&str) -> bool,
        candidate: impl Fn() -> String,
    ) -> Result<String, String> {
        let mut conn = self.open()?;
        let find = |conn: &Connection| -> Result<Option<String>, String> {
            conn.query_row(
                "SELECT device_id FROM devices WHERE platform=?1 AND account_id=?2",
                params![self.platform, account_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(storage_error)
        };
        if let Some(id) = find(&conn)?.filter(|id| valid(id)) {
            return Ok(id);
        }
        let legacy = if self.progress(&conn)?.is_some_and(|state| state.complete) {
            None
        } else {
            self.legacy_config()?
        };
        let ids = legacy
            .as_ref()
            .and_then(|(_, config)| config.device_ids.as_ref());
        let occupied: HashSet<&str> = ids
            .into_iter()
            .flat_map(|ids| ids.values().map(String::as_str))
            .collect();
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage_error)?;
        if let Some(id) = find(&tx)?.filter(|id| valid(id)) {
            return Ok(id);
        }
        let id = if let Some(id) = ids
            .and_then(|ids| ids.get(account_id))
            .filter(|id| valid(id))
        {
            id.clone()
        } else {
            let mut available = None;
            for _ in 0..32 {
                let next = candidate();
                let exists: bool = tx
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM devices WHERE platform=?1 AND device_id=?2)",
                        params![self.platform, next],
                        |row| row.get(0),
                    )
                    .map_err(storage_error)?;
                if !exists && !occupied.contains(next.as_str()) {
                    available = Some(next);
                    break;
                }
            }
            available.ok_or("无法生成未占用的签到设备标识")?
        };
        tx.execute(
            "INSERT INTO devices(platform,account_id,device_id) VALUES (?1,?2,?3)
             ON CONFLICT(platform,account_id) DO UPDATE SET device_id=excluded.device_id",
            params![self.platform, account_id, id],
        )
        .map_err(storage_error)?;
        tx.commit().map_err(storage_error)?;
        Ok(id)
    }

    fn schedule(&self, source: PathBuf, rules: Rules) {
        let store = self.clone();
        background_maintenance::schedule(
            format!("checkin-state:{}:{}", self.platform, self.legacy.display()),
            move || store.migrate_batch(&source, &rules),
        );
    }

    fn migrate_batch(&self, source: &Path, rules: &Rules) -> Result<bool, String> {
        let mut conn = self.open()?;
        let before = self.progress(&conn)?;
        if before.as_ref().is_some_and(|state| state.complete) {
            return Ok(true);
        }
        let file = File::open(source).map_err(storage_error)?;
        let metadata = file.metadata().map_err(storage_error)?;
        let stamp = format!(
            "{}:{}:{:?}",
            source.display(),
            metadata.len(),
            metadata.modified().map_err(storage_error)?
        );
        if before.as_ref().is_some_and(|state| state.stamp != stamp) {
            return Err("迁移期间旧签到配置发生变化，原文件已保留".into());
        }
        let mut state = before.clone().unwrap_or(Progress {
            stamp,
            cursor: Cursor::default(),
            complete: false,
            last_checked_date: None,
        });
        let mut input = Input::new(
            file,
            state.cursor.offset,
            background_maintenance::BATCH_BYTES,
        )
        .map_err(storage_error)?;
        let items = read_batch(&mut input, &mut state)?;
        if state.complete {
            // A CAS creation lets newer user settings win and makes a crash here retryable.
            atomic_write::replace_secret_string_if_matches(
                &self.rules_path(),
                None,
                Some(&to_json(rules)?),
            )?;
        }
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage_error)?;
        if self.progress(&tx)? != before {
            return Ok(false);
        }
        for item in items {
            match item {
                LegacyItem::Schedule(id, schedule) => {
                    put_schedule(&tx, self.platform, &id, &schedule, false)?;
                }
                LegacyItem::Device(id, device) => {
                    tx.execute(
                        "INSERT OR IGNORE INTO devices(platform,account_id,device_id) VALUES (?1,?2,?3)",
                        params![self.platform, id, device],
                    )
                    .map_err(storage_error)?;
                }
            }
        }
        tx.execute(
            "INSERT INTO migration(platform,stamp,cursor,complete,last_checked_date) VALUES (?1,?2,?3,?4,?5)
             ON CONFLICT(platform) DO UPDATE SET stamp=excluded.stamp,cursor=excluded.cursor,
                complete=excluded.complete,last_checked_date=excluded.last_checked_date",
            params![self.platform, state.stamp, to_json(&state.cursor)?, state.complete, state.last_checked_date],
        )
        .map_err(storage_error)?;
        tx.commit().map_err(storage_error)?;
        Ok(state.complete)
    }
}

fn put_schedule(
    conn: &Connection,
    platform: &str,
    id: &str,
    state: &AccountSchedule,
    live: bool,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO schedules(platform,account_id,scheduled_date,scheduled_minute,last_checked_date)
            VALUES (?1,?2,?3,?4,?5)
         ON CONFLICT(platform,account_id) DO UPDATE SET scheduled_date=excluded.scheduled_date,
             scheduled_minute=excluded.scheduled_minute,
             last_checked_date=CASE
                 WHEN excluded.scheduled_date=schedules.scheduled_date
                      AND schedules.last_checked_date=excluded.scheduled_date
                 THEN schedules.last_checked_date ELSE excluded.last_checked_date END
         WHERE ?6",
        params![platform, id, state.scheduled_date, state.scheduled_minute, state.last_checked_date, live],
    )
    .map_err(storage_error)?;
    Ok(())
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Rules {
    enabled: bool,
    start_time: String,
    end_time: String,
}

impl From<&CheckinConfig> for Rules {
    fn from(config: &CheckinConfig) -> Self {
        Self {
            enabled: config.enabled,
            start_time: config.start_time.clone(),
            end_time: config.end_time.clone(),
        }
    }
}

impl Rules {
    fn validate(&self) -> Result<(), String> {
        let start = validate_time(&self.start_time)
            .ok_or_else(|| format!("自动签到开始时间无效: {}", self.start_time))?;
        let end = validate_time(&self.end_time)
            .ok_or_else(|| format!("自动签到结束时间无效: {}", self.end_time))?;
        if start > end {
            return Err("自动签到开始时间不能晚于结束时间".into());
        }
        Ok(())
    }
}

fn validate_time(value: &str) -> Option<i32> {
    if value.len() != 5 || value.as_bytes().get(2) != Some(&b':') {
        return None;
    }
    let hour = value.get(0..2)?.parse::<i32>().ok()?;
    let minute = value.get(3..5)?.parse::<i32>().ok()?;
    if (0..=23).contains(&hour) && (0..=59).contains(&minute) {
        Some(hour * 60 + minute)
    } else {
        None
    }
}

pub(super) fn validate_config(config: &CheckinConfig) -> Result<(), String> {
    Rules::from(config).validate()?;
    for (id, schedule) in config.account_schedules.iter().flatten() {
        if !(0..=1439).contains(&schedule.scheduled_minute) {
            return Err(format!(
                "账号 {id} 的自动签到分钟无效: {}",
                schedule.scheduled_minute
            ));
        }
    }
    Ok(())
}

fn read_rules(path: &Path) -> Result<Option<Rules>, String> {
    let rules = read_json::<Rules>(path)?;
    if let Some(rules) = &rules {
        rules.validate()?;
    }
    Ok(rules)
}

fn read_config(path: &Path) -> Result<Option<CheckinConfig>, String> {
    let config = read_json::<CheckinConfig>(path)?;
    if let Some(config) = &config {
        validate_config(config)?;
    }
    Ok(config)
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(storage_error(error)),
    };
    serde_json::from_reader(std::io::BufReader::new(file))
        .map(Some)
        .map_err(storage_error)
}

fn to_json(value: &impl Serialize) -> Result<String, String> {
    serde_json::to_string(value).map_err(storage_error)
}

fn storage_error(error: impl std::fmt::Display) -> String {
    format!("读写本地签到状态失败: {error}")
}

#[derive(Clone, PartialEq)]
struct Progress {
    stamp: String,
    cursor: Cursor,
    complete: bool,
    last_checked_date: Option<String>,
}

#[derive(Clone, Default, Deserialize, PartialEq, Serialize)]
struct Cursor {
    offset: u64,
    section: Section,
    next_item: bool,
}

#[derive(Clone, Copy, Default, Deserialize, PartialEq, Serialize)]
enum Section {
    #[default]
    Root,
    Schedules,
    Devices,
}

enum LegacyItem {
    Schedule(String, AccountSchedule),
    Device(String, String),
}

fn read_batch(input: &mut Input, state: &mut Progress) -> Result<Vec<LegacyItem>, String> {
    let start = Instant::now();
    let start_offset = state.cursor.offset;
    let mut rows = Vec::new();
    if start_offset == 0 {
        input.expect(b'{').map_err(legacy_error)?;
    }
    loop {
        match read_item(input, state) {
            Ok(Some(row)) => rows.push(row),
            Ok(None) => {}
            Err(ReadError::Budget) if state.cursor.offset > start_offset => break,
            Err(error) => return Err(legacy_error(error)),
        }
        state.cursor.offset = input.offset;
        if state.complete
            || rows.len() >= background_maintenance::BATCH_ROWS
            || input.budget == 0
            || start.elapsed() >= background_maintenance::BATCH_TIME
        {
            break;
        }
    }
    Ok(rows)
}

fn read_item(input: &mut Input, state: &mut Progress) -> Result<Option<LegacyItem>, ReadError> {
    if input.peek()? == Some(b'}') {
        input.take()?;
        if state.cursor.section == Section::Root {
            if input.peek()?.is_some() {
                return Err(ReadError::Format);
            }
            state.complete = true;
        } else {
            state.cursor.section = Section::Root;
            state.cursor.next_item = true;
        }
        return Ok(None);
    }
    if state.cursor.next_item {
        input.expect(b',')?;
    }
    let key = input.value::<String>()?;
    input.expect(b':')?;
    let item = match state.cursor.section {
        Section::Root => {
            let section = match key.as_str() {
                "accountSchedules" => Some(Section::Schedules),
                "deviceIds" => Some(Section::Devices),
                _ => None,
            };
            if let Some(section) = section {
                if input.peek()? == Some(b'{') {
                    input.expect(b'{')?;
                    state.cursor.section = section;
                    state.cursor.next_item = false;
                    return Ok(None);
                }
            }
            if key == "lastCheckedDate" {
                state.last_checked_date = input.value()?;
            } else {
                input.value::<serde::de::IgnoredAny>()?;
            }
            None
        }
        Section::Schedules => Some(LegacyItem::Schedule(key, input.value()?)),
        Section::Devices => Some(LegacyItem::Device(key, input.value()?)),
    };
    state.cursor.next_item = true;
    Ok(item)
}

fn legacy_error(error: ReadError) -> String {
    match error {
        ReadError::Budget => "旧签到状态单条记录超出迁移预算".into(),
        ReadError::Format => "旧签到状态格式无效，原文件已保留".into(),
        ReadError::Io(error) => storage_error(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn fixture(legacy: &Path, records: usize) -> CheckinConfig {
        let schedules = (0..records)
            .map(|i| {
                (
                    format!("account-{i:03}"),
                    AccountSchedule {
                        scheduled_date: "2026-11-10".into(),
                        scheduled_minute: 420,
                        last_checked_date: None,
                    },
                )
            })
            .collect();
        let config = CheckinConfig {
            enabled: true,
            last_checked_date: Some("2026-11-09".into()),
            account_schedules: Some(schedules),
            ..CheckinConfig::default()
        };
        fs::write(legacy, to_json(&config).unwrap()).unwrap();
        config
    }

    #[test]
    fn resumed_checkin_import_keeps_new_rules_and_completed_accounts() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("legacy.json");
        let original = fixture(&path, 75);
        let legacy_bytes = fs::read(&path).unwrap();
        let store = CheckinStore::new(path.clone(), "workbuddy");
        let rules = Rules::from(&original);
        assert!(!store.migrate_batch(&path, &rules).unwrap());
        let progress = store.progress(&store.open().unwrap()).unwrap().unwrap();
        assert!(progress.cursor.offset > 0);
        let mut ui_snapshot = original.clone();
        ui_snapshot.enabled = false;
        ui_snapshot.start_time = "08:00".into();
        ui_snapshot.account_schedules = None;
        store.save_rules(&ui_snapshot).unwrap();

        let account = "account-074".to_string();
        let mut newer = original.account_schedules.as_ref().unwrap()[&account].clone();
        newer.last_checked_date = Some(newer.scheduled_date.clone());
        store
            .save_schedules(
                original.account_schedules.as_ref(),
                Some(HashMap::from([(account.clone(), newer.clone())])),
            )
            .unwrap();

        let restarted = CheckinStore::new(path.clone(), "workbuddy");
        assert!(
            restarted
                .progress(&restarted.open().unwrap())
                .unwrap()
                .unwrap()
                == progress
        );
        while !restarted.migrate_batch(&path, &rules).unwrap() {}
        let config = restarted.load().unwrap();
        assert!(!config.enabled);
        assert_eq!(config.start_time, "08:00");
        assert_eq!(config.last_checked_date, original.last_checked_date);
        let schedules = config.account_schedules.unwrap();
        assert_eq!(schedules.len(), 75);
        assert_eq!(schedules[&account], newer);
        assert_eq!(fs::read(&path).unwrap(), legacy_bytes);
        // An older same-day cycle can adjust the minute but cannot make the
        // already completed account eligible again.
        let mut stale = newer;
        stale.scheduled_minute += 1;
        stale.last_checked_date = None;
        restarted
            .save_schedules(None, Some(HashMap::from([(account.clone(), stale)])))
            .unwrap();
        assert_eq!(
            restarted.load().unwrap().account_schedules.unwrap()[&account]
                .last_checked_date
                .as_deref(),
            Some("2026-11-10")
        );

        // Once the checkpoint is complete, even a replaced old snapshot is ignored.
        fs::write(&path, "invalid legacy content").unwrap();
        assert_eq!(
            restarted.load().unwrap().account_schedules.unwrap().len(),
            75
        );
        assert!(restarted.migrate_batch(&path, &rules).unwrap());
    }

    #[test]
    fn checkin_device_ids_reserve_legacy_ids_before_their_batch() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("legacy.json");
        let mut original = fixture(&path, 75);
        let existing = "1794300000000001";
        let available = "1794300000000002";
        original.device_ids = Some(HashMap::from([("existing".into(), existing.into())]));
        fs::write(&path, to_json(&original).unwrap()).unwrap();
        let store = CheckinStore::new(path.clone(), "trae-cn");
        let rules = Rules::from(&original);
        assert!(!store.migrate_batch(&path, &rules).unwrap());
        let valid = |id: &str| id.len() == 16 && id.bytes().all(|byte| byte.is_ascii_digit());
        assert_eq!(
            store
                .device_id("existing", valid, || panic!("legacy device must be reused"))
                .unwrap(),
            existing
        );
        let candidates = Cell::new(0);
        assert_eq!(
            store
                .device_id("new", valid, || {
                    let next = candidates.get();
                    candidates.set(next + 1);
                    [existing, available][next].into()
                })
                .unwrap(),
            available
        );
        while !store.migrate_batch(&path, &rules).unwrap() {}
        let restarted = CheckinStore::new(path, "trae-cn");
        assert_eq!(
            restarted
                .device_id("new", valid, || panic!("new device must be reused"))
                .unwrap(),
            available
        );
        assert_eq!(restarted.load().unwrap().device_ids.unwrap().len(), 2);
    }

    #[test]
    fn checkin_webview_seed_preserves_existing_rules_and_legacy_source() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("legacy.json");
        let store = CheckinStore::new(path.clone(), "minimaxcode");
        let webview = CheckinConfig {
            enabled: true,
            last_checked_date: Some("2026-11-09".into()),
            ..CheckinConfig::default()
        };
        assert!(store.seed_webview(&webview).unwrap().enabled);
        let source = fs::read(&path).unwrap();

        let edited = CheckinConfig {
            start_time: "07:00".into(),
            ..CheckinConfig::default()
        };
        store.save_rules(&edited).unwrap();
        let reloaded = store.seed_webview(&webview).unwrap();
        assert!(!reloaded.enabled);
        assert_eq!(reloaded.start_time, "07:00");
        assert_eq!(reloaded.last_checked_date, webview.last_checked_date);
        assert_eq!(fs::read(&path).unwrap(), source);
    }

    #[test]
    fn checkin_reads_existing_backup_without_overwriting_invalid_primary() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("legacy.json");
        let backup = path.with_extension("json.bak");
        let old = fixture(&backup, 2);
        fs::write(&path, "{invalid").unwrap();
        let store = CheckinStore::new(path.clone(), "workbuddy");
        let (source, _) = store.legacy_config().unwrap().unwrap();
        assert_eq!(source, backup);
        assert!(store.migrate_batch(&source, &Rules::from(&old)).unwrap());
        assert_eq!(store.load().unwrap().account_schedules.unwrap().len(), 2);
        assert_eq!(fs::read_to_string(&path).unwrap(), "{invalid");
    }

    #[test]
    fn checkin_import_retries_after_write_lock_without_advancing_progress() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("legacy.json");
        let old = fixture(&path, 40);
        let store = CheckinStore::new(path.clone(), "workbuddy");
        let mut other = store.open().unwrap();
        let writer = other
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        assert!(store.migrate_batch(&path, &Rules::from(&old)).is_err());
        assert!(store.progress(&writer).unwrap().is_none());
        writer.rollback().unwrap();
        assert!(!store.migrate_batch(&path, &Rules::from(&old)).unwrap());
        while !store.migrate_batch(&path, &Rules::from(&old)).unwrap() {}
        assert_eq!(store.load().unwrap().account_schedules.unwrap().len(), 40);
    }
}
