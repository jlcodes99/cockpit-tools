use super::*;
use std::future::Future;

fn assert_small_command_future<Args, F: Future>(name: &str, _command: impl FnOnce(Args) -> F) {
    // Release IPC moves futures several times before polling, on a 1 MiB Windows UI stack.
    // Inspect the type without constructing an AppHandle or touching real account data.
    let size = std::mem::size_of::<F>();
    println!("{name}: {size} bytes");
    assert!(
        size <= 16 * 1024,
        "{name} future is too large: {size} bytes"
    );
}

#[test]
fn current_quota_refresh_future_stays_small() {
    assert_small_command_future("refresh_current_codex_quota", refresh_current_codex_quota);
}

#[test]
fn single_quota_refresh_future_stays_small() {
    assert_small_command_future("refresh_codex_quota", |(app, account_id)| {
        refresh_codex_quota(app, account_id)
    });
}

#[test]
fn all_quota_refresh_future_stays_small() {
    assert_small_command_future("refresh_all_codex_quotas", refresh_all_codex_quotas);
}

#[test]
fn batch_quota_refresh_future_stays_small() {
    assert_small_command_future(
        "refresh_codex_quotas_batch",
        |(app, account_ids, respect)| refresh_codex_quotas_batch(app, account_ids, respect),
    );
}

#[test]
fn post_refresh_checks_future_stays_small() {
    assert_small_command_future(
        "run_codex_post_refresh_checks",
        run_codex_post_refresh_checks,
    );
}
