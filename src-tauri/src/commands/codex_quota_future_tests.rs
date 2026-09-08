//! Stack-safety regressions: inspect Future types without constructing AppHandle,
//! invoking a command, polling a Future, or reading/writing any account data.

use super::*;
use std::future::Future;
use std::mem::size_of;

// The Windows dump had a 212,152-byte command Future. Leave room for platform
// layout differences, but fail long before a ~200 KiB child becomes inline again.
const MAX_REFRESH_FUTURE_BYTES: usize = 16 * 1024;

fn assert_small_future<A, F: Future + Send>(name: &str, _factory: impl FnOnce(A) -> F) {
    // Deliberately never call the factory. Even current/all account commands are
    // safe to check this way, and no Tauri runtime or mock application is needed.
    let bytes = size_of::<F>();
    println!("{name} Future: {bytes} bytes");
    assert!(
        bytes <= MAX_REFRESH_FUTURE_BYTES,
        "{name} Future is {bytes} bytes (limit {MAX_REFRESH_FUTURE_BYTES}); \
         box quota/post-refresh child Futures before awaiting them"
    );
}

#[test]
fn quota_refresh_command_futures_stay_small() {
    assert_small_future("single", |(app, account_id)| {
        refresh_codex_quota(app, account_id)
    });
    assert_small_future("current", refresh_current_codex_quota);
    assert_small_future("all", refresh_all_codex_quotas);
    assert_small_future("batch", |(app, account_ids, respect, background)| {
        refresh_codex_quotas_batch(app, account_ids, respect, background)
    });
}

#[test]
fn quota_refresh_worker_futures_stay_small() {
    assert_small_future("post-refresh", |app: &'static AppHandle| {
        run_codex_post_refresh_checks(app)
    });
    assert_small_future(
        "import-refresh",
        |(app, accounts): (&'static AppHandle, Vec<CodexAccount>)| {
            refresh_imported_codex_accounts(app, accounts)
        },
    );
}

fn compact(source: &str) -> String {
    source.chars().filter(|c| !c.is_whitespace()).collect()
}

fn command_body(name: &str) -> &'static str {
    let source = include_str!("codex_account_commands.rs");
    let start = source.find(&format!("fn {name}(")).unwrap();
    let source = &source[start..];
    let end = source.find("\n}").unwrap();
    &source[..end]
}

#[test]
fn quota_refresh_keeps_early_boxing_boundaries_and_async_commands() {
    // A size check alone can miss a removed boundary when another child remains
    // boxed. Check each boundary as well, including the auto-switch worker.
    let source = include_str!("codex_account_commands.rs");
    for (name, refresh) in [
        (
            "refresh_codex_quota",
            "Box::pin(codex_quota::refresh_account_quota(&account_id)).await",
        ),
        (
            "refresh_current_codex_quota",
            "Box::pin(codex_quota::refresh_account_quota(&account.id)).await",
        ),
        (
            "refresh_all_codex_quotas",
            "Box::pin(codex_quota::refresh_all_quotas()).await?",
        ),
        (
            "refresh_codex_quotas_batch",
            "Box::pin(codex_quota::refresh_quotas_for_account_ids_with_options(&account_ids,respect,)).await?",
        ),
    ] {
        assert!(
            source.contains(&format!("#[tauri::command]\npub async fn {name}(")),
            "{name}: preserve Tauri's async dispatch contract"
        );
        let body = compact(command_body(name));
        assert!(body.contains(refresh), "{name}: missing quota boundary");
        assert!(
            body.contains("Box::pin(run_codex_post_refresh_checks(&app)).await"),
            "{name}: missing post-refresh boundary"
        );
    }
    assert!(
        compact(command_body("run_codex_post_refresh_checks")).contains(
            "Box::pin(switch_codex_account(app.clone(),target_id.clone(),None,None,None,)).await"
        )
    );
    let imported = compact(command_body("refresh_imported_codex_accounts"));
    assert!(imported.contains("Box::pin(codex_quota::refresh_account_quota(&account.id)).await"));
    assert!(imported.contains("Box::pin(run_codex_post_refresh_checks(app)).await"));
}

#[test]
fn windows_stack_reserve_is_target_based_and_scoped_to_app_binary() {
    let build = compact(include_str!("../../build.rs"));
    let expected = compact(concat!(
        "let target = std::env::var(\"TARGET\").expect(\"TARGET is required\");",
        "if target.ends_with(\"-windows-msvc\") {",
        "println!(\"cargo:rustc-link-arg-bin=cockpit-tools=/STACK:8388608\");",
        "}",
    ));
    assert!(build.contains(&expected));
    // The package's implicit src/main.rs binary must match the linker directive.
    assert!(include_str!("../../Cargo.toml").contains("[package]\nname = \"cockpit-tools\""));
}
