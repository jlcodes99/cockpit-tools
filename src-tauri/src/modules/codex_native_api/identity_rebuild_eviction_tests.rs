//! Regression coverage for the single-pass hot-cache eviction used by
//! `commit_rebuild`. Eviction removes the oldest rows in one SQL pass so
//! `native_rebuild_checkpoints_v1` never exceeds its count or byte budget.
use super::*;

fn open() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    super::initialize(&connection).unwrap();
    connection
}

fn seed_checkpoint(connection: &Connection, response: &str, bytes: i64, created: i64) {
    connection
        .execute(
            "INSERT INTO native_rebuild_checkpoints_v1
               (caller,response,thread,account,generation,transcript,bytes,created)
             VALUES('caller',?1,'thread','account','generation','[]',?2,?3)",
            params![response, bytes, created],
        )
        .unwrap();
}

fn checkpoint_count(connection: &Connection) -> i64 {
    connection
        .query_row("SELECT COUNT(*) FROM native_rebuild_checkpoints_v1", [], |row| {
            row.get(0)
        })
        .unwrap()
}

fn total_bytes(connection: &Connection) -> i64 {
    connection
        .query_row(
            "SELECT COALESCE(SUM(bytes),0) FROM native_rebuild_checkpoints_v1",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

fn present(connection: &Connection, response: &str) -> bool {
    connection
        .query_row(
            "SELECT 1 FROM native_rebuild_checkpoints_v1 WHERE response=?1",
            params![response],
            |_| Ok(()),
        )
        .optional()
        .unwrap()
        .is_some()
}

fn evict(connection: &mut Connection) {
    let transaction = connection.transaction().unwrap();
    super::evict_hot_cache(&transaction).unwrap();
    transaction.commit().unwrap();
}

#[test]
fn count_budget_evicts_oldest_rows_in_one_pass() {
    let mut connection = open();
    for index in 0..130 {
        let response = format!("r{index:03}");
        seed_checkpoint(&connection, &response, 0, index);
    }
    evict(&mut connection);
    assert_eq!(checkpoint_count(&connection), MAX_CHECKPOINTS);
    assert!(!present(&connection, "r000"));
    assert!(!present(&connection, "r001"));
    assert!(present(&connection, "r002"));
    assert!(present(&connection, "r129"));
}

#[test]
fn byte_budget_evicts_oldest_rows_until_total_fits() {
    let mut connection = open();
    // Three rows each at exactly half the budget: removing the oldest leaves
    // two rows at the limit, so a single eviction satisfies the byte budget.
    let row_bytes = MAX_CACHE_BYTES / 2;
    for index in 0..3 {
        let response = format!("b{index}");
        seed_checkpoint(&connection, &response, row_bytes, index);
    }
    evict(&mut connection);
    assert_eq!(checkpoint_count(&connection), 2);
    assert!(!present(&connection, "b0"));
    assert!(present(&connection, "b1"));
    assert!(present(&connection, "b2"));
    assert!(total_bytes(&connection) <= MAX_CACHE_BYTES);
}

#[test]
fn count_and_byte_budgets_combine_into_one_removal_prefix() {
    let mut connection = open();
    // 130 rows at 1 MiB each: the byte budget needs 66 removals, the count
    // budget only 2, so the union removes the oldest 66 rows at once.
    let row_bytes = 1024 * 1024;
    for index in 0..130 {
        let response = format!("c{index:03}");
        seed_checkpoint(&connection, &response, row_bytes, index);
    }
    evict(&mut connection);
    assert_eq!(checkpoint_count(&connection), 64);
    assert!(!present(&connection, "c000"));
    assert!(!present(&connection, "c065"));
    assert!(present(&connection, "c066"));
    assert!(present(&connection, "c129"));
    assert!(total_bytes(&connection) <= MAX_CACHE_BYTES);
}
