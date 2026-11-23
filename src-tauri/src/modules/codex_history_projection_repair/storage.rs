use super::*;
use rusqlite::types::{ToSqlOutput, ValueRef};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
};

pub(super) const TABLES: [&str; 4] = [
    "thread_items",
    "thread_realtime_items",
    "thread_turns",
    "thread_history_projection_state",
];

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Schema {
    tables: Vec<Table>,
    objects: Vec<(String, String, String)>,
    migrations: Vec<(i64, i64)>,
}
#[derive(Debug, PartialEq, Eq)]
struct Table {
    name: String,
    columns: Vec<Column>,
    sql: String,
}
#[derive(Debug, PartialEq, Eq, serde::Serialize)]
struct Column {
    name: String,
    kind: String,
    not_null: bool,
    default: Option<String>,
    pk: u32,
}

impl Schema {
    pub fn empty() -> Self {
        Self {
            tables: vec![],
            objects: vec![],
            migrations: vec![],
        }
    }
}

fn expected(table: &str) -> (&[(&str, &str, bool, u32)], &[(&str, &str, bool)]) {
    match table {
        "thread_items" => (
            &[
                ("thread_id", "TEXT", true, 1),
                ("turn_id", "TEXT", true, 2),
                ("item_id", "TEXT", true, 3),
                ("rollout_ordinal", "INTEGER", true, 0),
                ("created_at_ms", "INTEGER", true, 0),
                ("item_json", "TEXT", true, 0),
            ],
            &[
                ("item_type", "TEXT", true),
                ("updated_at_ordinal", "INTEGER", true),
                ("started_at_ms", "INTEGER", false),
                ("completed_at_ms", "INTEGER", false),
            ],
        ),
        "thread_realtime_items" => (
            &[
                ("thread_id", "TEXT", true, 1),
                ("item_id", "TEXT", true, 2),
                ("rollout_ordinal", "INTEGER", true, 0),
                ("created_at_ms", "INTEGER", true, 0),
                ("item_type", "TEXT", true, 0),
                ("item_json", "TEXT", true, 0),
            ],
            &[],
        ),
        "thread_turns" => (
            &[
                ("thread_id", "TEXT", true, 1),
                ("turn_id", "TEXT", true, 2),
                ("rollout_ordinal", "INTEGER", true, 0),
                ("status", "TEXT", true, 0),
                ("error_json", "TEXT", false, 0),
                ("started_at", "INTEGER", false, 0),
                ("completed_at", "INTEGER", false, 0),
                ("duration_ms", "INTEGER", false, 0),
                ("first_user_item_id", "TEXT", false, 0),
                ("final_agent_item_id", "TEXT", false, 0),
            ],
            &[
                ("rollout_byte_offset", "INTEGER", false),
                ("rollout_end_ordinal", "INTEGER", false),
                ("rollout_end_byte_offset", "INTEGER", false),
                ("root_turn_id", "TEXT", false),
            ],
        ),
        _ => (
            &[
                ("thread_id", "TEXT", false, 1),
                ("next_rollout_byte_offset", "INTEGER", true, 0),
                ("next_rollout_ordinal", "INTEGER", true, 0),
            ],
            &[],
        ),
    }
}

const REALTIME_OBSERVATION_INDEXES: [(&str, &str); 3] = [
    (
        "idx_thread_realtime_observed",
        "CREATE INDEX idx_thread_realtime_observed
         ON thread_realtime_items(thread_id, json_extract(item_json, '$.observed_at_ms'), rollout_ordinal, item_id)
         WHERE json_extract(item_json, '$.observed_at_ms') IS NOT NULL",
    ),
    (
        "idx_thread_realtime_session_observed",
        "CREATE INDEX idx_thread_realtime_session_observed
         ON thread_realtime_items(thread_id, json_extract(item_json, '$.realtime_session_id'), json_extract(item_json, '$.observed_at_ms'), rollout_ordinal, item_id)
         WHERE json_extract(item_json, '$.observed_at_ms') IS NOT NULL",
    ),
    (
        "idx_thread_realtime_session_position",
        "CREATE INDEX idx_thread_realtime_session_position
         ON thread_realtime_items(thread_id, json_extract(item_json, '$.realtime_session_id'), rollout_ordinal, item_id)
         WHERE json_extract(item_json, '$.observed_at_ms') IS NOT NULL",
    ),
];

fn normalize_sql(sql: &str) -> String {
    sql.chars()
        .filter(|c| !c.is_ascii_whitespace())
        .collect::<String>()
        .trim_end_matches(';')
        .to_ascii_lowercase()
}

pub(super) fn schema(db: &Connection) -> Result<Schema, String> {
    let objects = {
        let mut statement = db.prepare("SELECT type,name,COALESCE(sql,'') FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY type,name").map_err(|e| error("SCHEMA",e))?;
        let rows = statement
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| error("SCHEMA", e))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| error("SCHEMA", e))?
    };
    let cleanup = normalize_sql("CREATE TRIGGER thread_realtime_items_projection_cleanup AFTER DELETE ON thread_history_projection_state BEGIN DELETE FROM thread_realtime_items WHERE thread_id = OLD.thread_id; END");
    for (kind, name, sql) in &objects {
        match kind.as_str() {
            "table" if TABLES.contains(&name.as_str()) || name == "_sqlx_migrations" => {}
            "index" => {}
            "trigger"
                if name == "thread_realtime_items_projection_cleanup"
                    && normalize_sql(sql) == cleanup => {}
            _ => {
                return Err(error(
                    "SCHEMA",
                    format!("unsupported history schema object {name}"),
                ))
            }
        }
    }
    let mut migrations = Vec::new();
    if objects
        .iter()
        .any(|(kind, name, _)| kind == "table" && name == "_sqlx_migrations")
    {
        let mut statement = db
            .prepare("SELECT version,success FROM _sqlx_migrations ORDER BY version")
            .map_err(|e| error("SCHEMA", e))?;
        let rows = statement
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))
            .map_err(|e| error("SCHEMA", e))?;
        let mut expected_version = 1;
        for row in rows {
            let (version, success) = row.map_err(|e| error("SCHEMA", e))?;
            if version != expected_version || version > 9 || success != 1 {
                return Err(error(
                    "SCHEMA",
                    "incomplete or unsupported history migrations",
                ));
            }
            expected_version += 1;
            migrations.push((version, success));
        }
    }
    let observation_indexes_migrated = migrations.last().is_some_and(|&(version, _)| version == 9);
    if observation_indexes_migrated {
        for (name, expected_sql) in REALTIME_OBSERVATION_INDEXES {
            // index_info does not expose the expressions or the partial-index predicate.
            let matches = objects.iter().any(|(kind, actual_name, sql)| {
                kind == "index"
                    && actual_name == name
                    && sql.split_ascii_whitespace().eq(expected_sql.split_ascii_whitespace())
            });
            if !matches {
                return Err(error("SCHEMA", format!("unsupported history index {name}")));
            }
        }
    }
    let mut tables = Vec::new();
    for name in TABLES {
        let sql = objects
            .iter()
            .find(|(kind, n, _)| kind == "table" && n == name)
            .map(|(_, _, sql)| sql.clone())
            .ok_or_else(|| error("SCHEMA", format!("missing table {name}")))?;
        if !sql
            .trim_start()
            .to_ascii_uppercase()
            .starts_with("CREATE TABLE ")
        {
            return Err(error("SCHEMA", "virtual history tables are unsupported"));
        }
        let hidden: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_xinfo(?1) WHERE hidden<>0)",
                [name],
                |r| r.get(0),
            )
            .map_err(|e| error("SCHEMA", e))?;
        if hidden {
            return Err(error(
                "SCHEMA",
                "generated or hidden history columns are unsupported",
            ));
        }
        let columns = {
            let mut statement = db
                .prepare(&format!("PRAGMA table_info({name})"))
                .map_err(|e| error("SCHEMA", e))?;
            let rows = statement
                .query_map([], |r| {
                    Ok(Column {
                        name: r.get(1)?,
                        kind: r.get::<_, String>(2)?.to_ascii_uppercase(),
                        not_null: r.get::<_, i32>(3)? != 0,
                        default: r.get(4)?,
                        pk: r.get(5)?,
                    })
                })
                .map_err(|e| error("SCHEMA", e))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| error("SCHEMA", e))?
        };
        let (required, optional) = expected(name);
        for &(field, kind, not_null, pk) in required {
            if !columns
                .iter()
                .any(|c| c.name == field && c.kind == kind && c.not_null == not_null && c.pk == pk)
            {
                return Err(error(
                    "SCHEMA",
                    format!("unsupported {name}.{field} definition"),
                ));
            }
        }
        for column in &columns {
            if required
                .iter()
                .any(|(field, _, _, _)| *field == column.name)
            {
                continue;
            }
            if !optional.iter().any(|(field, kind, not_null)| {
                *field == column.name
                    && *kind == column.kind
                    && *not_null == column.not_null
                    && column.pk == 0
            }) {
                return Err(error(
                    "SCHEMA",
                    format!("unsupported history column {}", column.name),
                ));
            }
        }
        let mut statement = db
            .prepare(&format!("PRAGMA foreign_key_list({name})"))
            .map_err(|e| error("SCHEMA", e))?;
        if statement
            .query([])
            .map_err(|e| error("SCHEMA", e))?
            .next()
            .map_err(|e| error("SCHEMA", e))?
            .is_some()
        {
            return Err(error("SCHEMA", "history foreign keys are unsupported"));
        }
        validate_indexes(db, name, observation_indexes_migrated)?;
        tables.push(Table {
            name: name.to_owned(),
            columns,
            sql,
        });
    }
    Ok(Schema {
        tables,
        objects,
        migrations,
    })
}

fn validate_indexes(
    db: &Connection,
    table: &str,
    observation_indexes_migrated: bool,
) -> Result<(), String> {
    let mut statement = db
        .prepare(&format!("PRAGMA index_list({table})"))
        .map_err(|e| error("SCHEMA", e))?;
    let indexes = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(1)?,
                r.get::<_, bool>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, bool>(4)?,
            ))
        })
        .map_err(|e| error("SCHEMA", e))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| error("SCHEMA", e))?;
    for (name, unique, origin, partial) in indexes {
        if origin == "pk" {
            continue;
        }
        if observation_indexes_migrated
            && REALTIME_OBSERVATION_INDEXES.iter().any(|&(index, _)| index == name)
        {
            continue;
        }
        let (owner, expected_unique, expected_partial, columns): (&str, bool, bool, &[&str]) =
            match name.as_str() {
                "idx_thread_turns_page" => (
                    "thread_turns",
                    true,
                    false,
                    &["thread_id", "rollout_ordinal"],
                ),
                "idx_thread_turns_end_page" => (
                    "thread_turns",
                    false,
                    true,
                    &["thread_id", "rollout_end_ordinal", "turn_id"],
                ),
                "idx_thread_items_page" => (
                    "thread_items",
                    true,
                    false,
                    &["thread_id", "rollout_ordinal"],
                ),
                "idx_thread_items_by_turn_page" => (
                    "thread_items",
                    false,
                    false,
                    &["thread_id", "turn_id", "rollout_ordinal"],
                ),
                "idx_thread_items_user_messages" => (
                    "thread_items",
                    false,
                    true,
                    &["thread_id", "rollout_ordinal"],
                ),
                "idx_thread_items_updated_page" => (
                    "thread_items",
                    false,
                    false,
                    &["thread_id", "updated_at_ordinal"],
                ),
                "idx_thread_items_by_turn_updated_page" => (
                    "thread_items",
                    false,
                    false,
                    &["thread_id", "turn_id", "updated_at_ordinal"],
                ),
                "idx_thread_realtime_items_page" => (
                    "thread_realtime_items",
                    true,
                    false,
                    &["thread_id", "rollout_ordinal"],
                ),
                "idx_thread_realtime_items_boundary" => (
                    "thread_realtime_items",
                    false,
                    true,
                    &["thread_id", "rollout_ordinal"],
                ),
                _ => return Err(error("SCHEMA", format!("unsupported history index {name}"))),
            };
        if owner != table
            || origin != "c"
            || unique != expected_unique
            || partial != expected_partial
        {
            return Err(error("SCHEMA", "unsupported history index definition"));
        }
        let mut columns_query = db
            .prepare(&format!("PRAGMA index_info({name})"))
            .map_err(|e| error("SCHEMA", e))?;
        let actual = columns_query
            .query_map([], |r| r.get::<_, String>(2))
            .map_err(|e| error("SCHEMA", e))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| error("SCHEMA", e))?;
        if actual != columns {
            return Err(error("SCHEMA", "unsupported history index columns"));
        }
    }
    Ok(())
}

pub(super) fn fingerprint(
    db: &Connection,
    schema: &Schema,
    candidates: Option<&[Checkpoint]>,
) -> Result<String, String> {
    let mut digest = Sha256::new();
    for table in &schema.tables {
        hash_table(db, table, candidates, &mut digest, |_| Ok(()))?;
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn hash_table(
    db: &Connection,
    table: &Table,
    candidates: Option<&[Checkpoint]>,
    digest: &mut Sha256,
    mut visit: impl FnMut(&[ValueRef<'_>]) -> Result<(), String>,
) -> Result<(), String> {
    let names = table.columns.iter()
        .map(|column| format!("\"{}\"", column.name))
        .collect::<Vec<_>>().join(",");
    let sql = if candidates.is_some() {
        let mut keys: Vec<_> = table.columns.iter().filter(|column| column.pk > 0).collect();
        keys.sort_by_key(|column| column.pk);
        let order = keys.iter().map(|column| format!("\"{}\"", column.name))
            .collect::<Vec<_>>().join(",");
        format!("SELECT {names} FROM {} WHERE thread_id=?1 ORDER BY {order}", table.name)
    } else {
        // The backup is a rowid table populated in source order.
        format!("SELECT {names} FROM {} ORDER BY rowid", table.name)
    };
    let mut statement = db.prepare(&sql).map_err(|e| error("DATABASE", e))?;
    digest.update(table.name.as_bytes());
    let mut read_rows = |mut rows: rusqlite::Rows<'_>| -> Result<(), String> {
        let mut count = 0usize;
        while let Some(row) = rows.next().map_err(|e| error("DATABASE", e))? {
            let mut values = Vec::with_capacity(table.columns.len());
            for index in 0..table.columns.len() {
                let value = row.get_ref(index).map_err(|e| error("DATABASE", e))?;
                match value {
                    ValueRef::Null => digest.update([0]),
                    ValueRef::Integer(value) => {
                        digest.update([1]);
                        digest.update(value.to_be_bytes());
                    }
                    ValueRef::Real(value) => {
                        digest.update([2]);
                        digest.update(value.to_bits().to_be_bytes());
                    }
                    ValueRef::Text(bytes) | ValueRef::Blob(bytes) => {
                        digest.update([if matches!(value, ValueRef::Text(_)) { 3 } else { 4 }]);
                        digest.update((bytes.len() as u64).to_be_bytes());
                        digest.update(bytes);
                    }
                }
                values.push(value);
            }
            visit(&values)?;
            count += 1;
            if count % 64 == 0 {
                std::thread::yield_now();
            }
        }
        Ok(())
    };
    if let Some(candidates) = candidates {
        for candidate in candidates {
            read_rows(statement.query([&candidate.id]).map_err(|e| error("DATABASE", e))?)?;
        }
    } else {
        read_rows(statement.query([]).map_err(|e| error("BACKUP", e))?)?;
    }
    Ok(())
}

fn create_private(path: &Path) -> Result<fs::File, String> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map_err(|e| error("BACKUP", e))
}

pub(super) fn backup(
    directory: &Path,
    plan: &Plan,
    source: rusqlite::Transaction<'_>,
) -> Result<String, String> {
    fs::create_dir_all(directory).map_err(|e| error("BACKUP", e))?;
    if fs::symlink_metadata(directory)
        .map_err(|e| error("BACKUP", e))?
        .file_type()
        .is_symlink()
    {
        return Err(error("BACKUP", "backup directory is a symlink"));
    }
    let path = directory.join("history-projection.sqlite");
    drop(create_private(&path)?);
    let mut db = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(|e| error("BACKUP", e))?;
    db.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; CREATE TABLE repair_metadata(key TEXT PRIMARY KEY, value TEXT NOT NULL);").map_err(|e|error("BACKUP",e))?;
    let tx = db.transaction().map_err(|e| error("BACKUP", e))?;
    let mut digest = Sha256::new();
    for table in &plan.schema.tables {
        // No declared affinity: SQLite stores original NULL/integer/real/text/blob types unchanged.
        let columns = table
            .columns
            .iter()
            .map(|c| format!("\"{}\"", c.name))
            .collect::<Vec<_>>()
            .join(",");
        tx.execute_batch(&format!("CREATE TABLE {} ({columns});", table.name))
            .map_err(|e| error("BACKUP", e))?;
        let placeholders = (0..table.columns.len())
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(",");
        let mut insert = tx
            .prepare(&format!(
                "INSERT INTO {} VALUES ({placeholders})",
                table.name
            ))
            .map_err(|e| error("BACKUP", e))?;
        hash_table(&source, table, Some(&plan.candidates), &mut digest, |row| {
            insert.execute(rusqlite::params_from_iter(row.iter().copied().map(ToSqlOutput::Borrowed)))
                .map(|_| ())
                .map_err(|e| error("BACKUP", e))
        })?;
        tx.execute(
            "INSERT INTO repair_metadata VALUES (?1,?2)",
            [
                format!("schema:{}", table.name),
                serde_json::to_string(&table.columns).map_err(|e| error("BACKUP", e))?,
            ],
        )
        .map_err(|e| error("BACKUP", e))?;
        tx.execute(
            "INSERT INTO repair_metadata VALUES (?1,?2)",
            [format!("ddl:{}", table.name), table.sql.clone()],
        )
        .map_err(|e| error("BACKUP", e))?;
    }
    source.commit().map_err(|e| error("DATABASE", e))?;
    let digest = format!("{:x}", digest.finalize());
    let source_metadata = plan
        .sources
        .iter()
        .map(|(id, source)| (id.clone(), source.backup_metadata()))
        .collect::<std::collections::BTreeMap<_, _>>();
    let metadata = serde_json::json!({"formatVersion":1,"profile":plan.profile,"sourceSchema":plan.schema.objects,"sourceMigrations":plan.schema.migrations,"sourceFiles":source_metadata,"projectionIds":plan.candidates.iter().map(|c|&c.id).collect::<Vec<_>>(),"typedRowsSha256":digest});
    tx.execute(
        "INSERT INTO repair_metadata VALUES ('repair',?1)",
        [metadata.to_string()],
    )
    .map_err(|e| error("BACKUP", e))?;
    tx.commit().map_err(|e| error("BACKUP", e))?;
    db.close().map_err(|(_, e)| error("BACKUP", e))?;
    // Windows FlushFileBuffers requires write access; File::open is read-only.
    // Reopen the completed backup without creating or truncating it.
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .and_then(|f| f.sync_all())
        .map_err(|e| error("BACKUP", e))?;
    let verified = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| error("BACKUP", e))?;
    let integrity: String = verified
        .query_row("PRAGMA quick_check(1)", [], |r| r.get(0))
        .map_err(|e| error("BACKUP", e))?;
    if integrity != "ok" || fingerprint(&verified, &plan.schema, None)? != digest
    {
        return Err(error("BACKUP", "typed history backup verification failed"));
    }
    drop(verified);
    let mut hash = Sha256::new();
    let mut reader = fs::File::open(&path).map_err(|e| error("BACKUP", e))?;
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = reader.read(&mut buffer).map_err(|e| error("BACKUP", e))?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    let mut checksum = create_private(&directory.join("history-projection.sqlite.sha256"))?;
    writeln!(checksum, "{:x}  history-projection.sqlite", hash.finalize())
        .map_err(|e| error("BACKUP", e))?;
    checksum.sync_all().map_err(|e| error("BACKUP", e))?;
    #[cfg(unix)]
    fs::File::open(directory)
        .and_then(|f| f.sync_all())
        .map_err(|e| error("BACKUP", e))?;
    Ok(digest)
}
