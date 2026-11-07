//! Incrementally maintained numeric minute buckets. Source rows remain the
//! authority. Membership makes live writes and historical backfill exactly-once;
//! SQLite triggers cover updates, repricing, deletes and interrupted transactions.
use rusqlite::{Connection, OptionalExtension};

pub(crate) struct Metric {
    pub name: &'static str,
    pub expression: &'static str,
    pub real: bool,
}
pub(crate) struct Rollup {
    pub source: &'static str,
    pub prefix: &'static str,
    pub width: i64,
    pub dimensions: &'static [&'static str],
    pub metrics: &'static [Metric],
}

impl Rollup {
    pub(crate) fn ensure(&self, conn: &Connection) -> rusqlite::Result<()> {
        let p = self.prefix;
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
            [format!("{p}_state")],
            |r| r.get(0),
        )?;
        if exists {
            return Ok(());
        }
        let tx = conn.unchecked_transaction()?;
        let dims = self.dimensions.join(",");
        let dimension_defs = self
            .dimensions
            .iter()
            .map(|d| format!("{d} TEXT NOT NULL"))
            .collect::<Vec<_>>()
            .join(",");
        let metric_defs = self
            .metrics
            .iter()
            .map(|m| {
                format!(
                    "{} {} NOT NULL",
                    m.name,
                    if m.real { "REAL" } else { "INTEGER" }
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let names = self
            .metrics
            .iter()
            .map(|m| m.name)
            .collect::<Vec<_>>()
            .join(",");
        let values = self
            .metrics
            .iter()
            .map(|m| m.expression.replace('$', "s."))
            .collect::<Vec<_>>()
            .join(",");
        let additions = self
            .metrics
            .iter()
            .map(|m| format!("{0}={0}+excluded.{0}", m.name))
            .collect::<Vec<_>>()
            .join(",");
        let source = self.source;
        let bucket = format!("(s.timestamp / {} * {})", self.width, self.width);
        let dimension_values = self
            .dimensions
            .iter()
            .map(|d| format!("trim(s.{d})"))
            .collect::<Vec<_>>()
            .join(",");
        let group_match = self
            .dimensions
            .iter()
            .map(|d| format!("{p}_buckets.{d}=trim(s.{d})"))
            .collect::<Vec<_>>()
            .join(" AND ");
        let bucket_key = format!("(timestamp,{dims})=(SELECT {bucket},{dimension_values} FROM {source} s WHERE s.rowid=OLD.log_id)");
        let subtract = self
            .metrics
            .iter()
            .map(|m| {
                format!(
                    "{0}={0}-(SELECT {1} FROM {source} s WHERE s.rowid=OLD.log_id)",
                    m.name,
                    m.expression.replace('$', "s.")
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let update_columns = std::iter::once("timestamp")
            .chain(self.dimensions.iter().copied())
            .chain(self.metrics.iter().flat_map(|m| {
                m.expression
                    .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$'))
                    .filter_map(|s| s.strip_prefix('$'))
            }))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
            .join(",");
        tx.execute_batch(&format!("CREATE TABLE {p}_buckets (
            timestamp INTEGER NOT NULL,{dimension_defs},request_count INTEGER NOT NULL,updated_at INTEGER NOT NULL,last_id INTEGER NOT NULL,{metric_defs},PRIMARY KEY(timestamp,{dims}));
            CREATE TABLE {p}_members(log_id INTEGER PRIMARY KEY);
            CREATE TABLE {p}_state(id INTEGER PRIMARY KEY CHECK(id=1),cursor INTEGER NOT NULL DEFAULT 0,complete INTEGER NOT NULL DEFAULT 0);
            INSERT INTO {p}_state(id) VALUES(1);
            CREATE TRIGGER {p}_member_insert AFTER INSERT ON {p}_members BEGIN
                INSERT INTO {p}_buckets(timestamp,{dims},request_count,updated_at,last_id,{names})
                SELECT {bucket},{dimension_values},1,s.timestamp,s.rowid,{values} FROM {source} s WHERE s.rowid=NEW.log_id
                ON CONFLICT(timestamp,{dims}) DO UPDATE SET request_count=request_count+1,
                    last_id=CASE WHEN excluded.updated_at>updated_at THEN excluded.last_id WHEN excluded.updated_at=updated_at THEN MAX(last_id,excluded.last_id) ELSE last_id END,
                    updated_at=MAX(updated_at,excluded.updated_at),{additions};
            END;
            CREATE TRIGGER {p}_member_delete BEFORE DELETE ON {p}_members BEGIN
                UPDATE {p}_buckets SET request_count=request_count-1,{subtract}
                WHERE {bucket_key};
                DELETE FROM {p}_buckets WHERE request_count=0 AND {bucket_key};
                UPDATE {p}_buckets SET updated_at=(SELECT MAX(s.timestamp) FROM {source} s
                    WHERE s.timestamp>={p}_buckets.timestamp AND s.timestamp<{p}_buckets.timestamp+{width}
                    AND s.rowid!=OLD.log_id AND {group_match} AND EXISTS(SELECT 1 FROM {p}_members m WHERE m.log_id=s.rowid))
                WHERE updated_at=(SELECT timestamp FROM {source} WHERE rowid=OLD.log_id)
                    AND {bucket_key};
                UPDATE {p}_buckets SET last_id=(SELECT s.rowid FROM {source} s
                    WHERE s.timestamp={p}_buckets.updated_at AND s.rowid!=OLD.log_id AND {group_match}
                    AND EXISTS(SELECT 1 FROM {p}_members m WHERE m.log_id=s.rowid) ORDER BY s.rowid DESC LIMIT 1)
                WHERE last_id=OLD.log_id AND {bucket_key};
            END;
            CREATE TRIGGER {p}_insert AFTER INSERT ON {source} BEGIN INSERT OR IGNORE INTO {p}_members(log_id) VALUES(NEW.rowid); END;
            CREATE TRIGGER {p}_before_update BEFORE UPDATE OF {update_columns} ON {source} BEGIN DELETE FROM {p}_members WHERE log_id=OLD.rowid; END;
            CREATE TRIGGER {p}_after_update AFTER UPDATE OF {update_columns} ON {source} BEGIN INSERT OR IGNORE INTO {p}_members(log_id) VALUES(NEW.rowid); END;
            CREATE TRIGGER {p}_delete BEFORE DELETE ON {source} BEGIN DELETE FROM {p}_members WHERE log_id=OLD.rowid; END;", width=self.width))?;
        tx.commit()
    }

    pub(crate) fn ready(&self, conn: &Connection) -> bool {
        conn.query_row(
            &format!("SELECT complete FROM {}_state WHERE id=1", self.prefix),
            [],
            |r| r.get(0),
        )
        .unwrap_or(false)
    }

    pub(crate) fn batch(&self, conn: &Connection) -> rusqlite::Result<bool> {
        let p = self.prefix;
        let tx = conn.unchecked_transaction()?;
        let (cursor, complete): (i64, bool) = tx.query_row(
            &format!("SELECT cursor,complete FROM {p}_state WHERE id=1"),
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if complete {
            return Ok(true);
        }
        let next: Option<i64> = tx.query_row(&format!("SELECT MAX(source_rowid) FROM (SELECT rowid AS source_rowid FROM {} WHERE rowid>?1 ORDER BY rowid LIMIT 32)",self.source), [cursor], |r| r.get(0)).optional()?.flatten();
        if let Some(next) = next {
            tx.execute(&format!("INSERT OR IGNORE INTO {p}_members(log_id) SELECT rowid FROM {} WHERE rowid>?1 AND rowid<=?2",self.source), [cursor,next])?;
            tx.execute(
                &format!("UPDATE {p}_state SET cursor=?1 WHERE id=1"),
                [next],
            )?;
        } else {
            tx.execute(&format!("UPDATE {p}_state SET complete=1 WHERE id=1"), [])?;
        }
        tx.commit()?;
        Ok(next.is_none())
    }

    /// Exact endpoints use source rows; only fully enclosed UTC minutes use
    /// buckets. A timezone change requires no rewrite of UTC projections.
    pub(crate) fn select(&self, conn: &Connection, start: i64, end: i64) -> String {
        let p = self.prefix;
        let first = start.saturating_add(self.width - 1) / self.width * self.width;
        let last = end.saturating_add(1) / self.width * self.width;
        let dims = self.dimensions.join(",");
        let names = self
            .metrics
            .iter()
            .map(|m| m.name)
            .collect::<Vec<_>>()
            .join(",");
        let expressions = self
            .metrics
            .iter()
            .map(|m| format!("{} AS {}", m.expression.replace('$', "s."), m.name))
            .collect::<Vec<_>>()
            .join(",");
        let dimension_values = self
            .dimensions
            .iter()
            .map(|d| format!("trim(s.{d}) AS {d}"))
            .collect::<Vec<_>>()
            .join(",");
        let pending = if self.ready(conn) {
            String::new()
        } else {
            format!(" OR NOT EXISTS(SELECT 1 FROM {p}_members m WHERE m.log_id=s.rowid)")
        };
        format!("SELECT timestamp,{dims},request_count,updated_at,last_id,{names} FROM {p}_buckets WHERE timestamp>={first} AND timestamp<{last}
            UNION ALL SELECT s.timestamp,{dimension_values},1 AS request_count,s.timestamp AS updated_at,s.rowid AS last_id,{expressions} FROM {source} s
            WHERE s.timestamp>={start} AND s.timestamp<={end} AND (s.timestamp<{first} OR s.timestamp>={last}{pending})",source=self.source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const TEST: Rollup = Rollup {
        source: "events",
        prefix: "test_rollup",
        width: 60,
        dimensions: &["model"],
        metrics: &[Metric {
            name: "tokens",
            expression: "MAX($tokens,0)",
            real: false,
        }],
    };
    fn total(c: &Connection, start: i64, end: i64) -> (i64, i64) {
        c.query_row(
            &format!(
                "SELECT COALESCE(SUM(request_count),0),COALESCE(SUM(tokens),0) FROM ({})",
                TEST.select(c, start, end)
            ),
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    }
    #[test]
    fn backfill_live_update_delete_boundaries_and_restart_are_exact() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE events(id INTEGER PRIMARY KEY,timestamp INTEGER,model TEXT,tokens INTEGER);CREATE INDEX time ON events(timestamp);
            INSERT INTO events VALUES(1,60,'m',10),(2,80,'m',20),(3,121,'n',30);").unwrap();
        TEST.ensure(&c).unwrap();
        assert_eq!(total(&c, 0, 179), (3, 60));
        c.execute("UPDATE events SET tokens=25 WHERE id=2", [])
            .unwrap();
        assert_eq!(total(&c, 0, 179), (3, 65));
        assert!(!TEST.batch(&c).unwrap());
        TEST.ensure(&c).unwrap();
        assert!(TEST.batch(&c).unwrap());
        assert_eq!(total(&c, 70, 121), (2, 55));
        c.execute("DELETE FROM events WHERE id=2", []).unwrap();
        assert_eq!(total(&c, 0, 179), (2, 40));
        assert_eq!(
            c.query_row(
                "SELECT updated_at FROM test_rollup_buckets WHERE model='m'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            60
        );
        c.execute_batch("BEGIN; UPDATE events SET tokens=99; ROLLBACK;")
            .unwrap();
        assert_eq!(total(&c, 0, 179), (2, 40));
        c.execute("DELETE FROM events", []).unwrap();
        assert_eq!(total(&c, 0, 179), (0, 0));
        c.execute("INSERT INTO events VALUES(1,60,'m',7)", [])
            .unwrap();
        assert_eq!(total(&c, 0, 179), (1, 7));
    }
}
