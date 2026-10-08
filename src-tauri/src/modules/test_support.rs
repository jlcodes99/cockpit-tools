use std::sync::{Mutex, OnceLock};

pub fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
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
