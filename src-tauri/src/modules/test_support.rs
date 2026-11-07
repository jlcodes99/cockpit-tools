use std::sync::{Mutex, OnceLock};

pub fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Audit fixture declarations in test source without modifying process env.
/// A single holder must precede the data guard and use lexical RAII cleanup.
/// TestCacheGuard owns this same mutex; taking both holders would deadlock.
pub fn env_guard_order_violations(source: &str, constructor: &str) -> (usize, Vec<String>) {
    let lines: Vec<_> = source.lines().collect();
    let mut functions: Vec<_> = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            let line = line.trim();
            let signature = line
                .strip_prefix("async fn ")
                .or_else(|| line.strip_prefix("fn "))?;
            Some((
                index,
                signature.split('(').next().unwrap().trim().to_string(),
            ))
        })
        .collect();
    functions.push((lines.len(), String::new()));
    let mut checked = 0;
    let mut violations = Vec::new();
    for pair in functions.windows(2) {
        let (start, name) = &pair[0];
        let scope = &lines[*start..pair[1].0];
        for (index, line) in scope.iter().enumerate() {
            if !line.trim_start().starts_with("let ") || !line.contains(constructor) {
                continue;
            }
            checked += 1;
            let mut holders = Vec::new();
            for (declaration_index, declaration) in scope[..index].iter().enumerate() {
                let Some(binding) = declaration.trim().strip_prefix("let ") else {
                    continue;
                };
                let direct = declaration.contains("test_support::env_lock()");
                let cached = declaration.contains("codex_unified_proxy::TestCacheGuard::new()");
                if !direct && !cached {
                    continue;
                }
                let before = scope[declaration_index..index].join("\n");
                let statement = before.split(';').next().unwrap();
                if cached || statement.contains(".lock()") {
                    holders.push(binding.split('=').next().unwrap().trim().to_string());
                }
            }
            let early_release = holders.iter().any(|holder| {
                scope
                    .iter()
                    .any(|line| line.contains(&format!("drop({holder})")))
            });
            if holders.len() != 1 || early_release {
                violations.push(format!("{name}:{}", start + index + 1));
            }
        }
    }
    (checked, violations)
}

pub fn fallback_data_dir() -> std::path::PathBuf {
    static ROOT: OnceLock<std::path::PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        std::env::temp_dir().join(format!(
            "cockpit-unit-test-data-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ))
    })
    .clone()
}
