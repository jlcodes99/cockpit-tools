//! Headless continuity worker supervised by the Go gateway, independent of GUI lifetime.
use std::io::{Read, Write};

pub(crate) fn try_run() -> Option<i32> {
    let args: Vec<_> = std::env::args_os().collect();
    if args
        .get(1)
        .is_none_or(|value| value != "--codex-continuity-bridge")
    {
        return None;
    }
    let result = (|| -> Result<(), String> {
        if args.len() != 3 {
            return Err("Invalid continuity worker arguments".into());
        }
        let path = std::path::PathBuf::from(&args[2]);
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|_| "Continuity worker configuration is unavailable")?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 8 << 20 {
            return Err("Invalid continuity worker configuration".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o077 != 0 {
                return Err("Continuity worker configuration must be private".into());
            }
        }
        let config = serde_json::from_slice(
            &std::fs::read(path).map_err(|_| "Continuity worker configuration is unavailable")?,
        )
        .map_err(|_| "Invalid continuity worker configuration")?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|_| "Continuity runtime is unavailable")?;
        let result = runtime.block_on(async move {
            let handle = super::codex_native_api::continuity_bridge::start(config)?;
            println!(
                "{}",
                serde_json::json!({"protocol":1,"endpoint":format!("http://{}",handle.addr)})
            );
            std::io::stdout()
                .flush()
                .map_err(|_| "Continuity worker ready pipe closed")?;
            // The parent holds stdin open. Its exit closes the pipe even after
            // Cockpit's GUI exits, without PID reuse or process-scanning races.
            let (sender, receiver) = tokio::sync::oneshot::channel();
            std::thread::spawn(move || {
                let _ = std::io::stdin().read(&mut [0u8; 1]);
                let _ = sender.send(());
            });
            let _ = receiver.await;
            handle.stop().await?;
            Ok::<(), String>(())
        });
        runtime.shutdown_timeout(std::time::Duration::from_secs(2));
        result
    })();
    if result.is_err() {
        // Never print the configuration, token, request body or filesystem cause.
        eprintln!("Continuity worker failed to start or shut down");
    }
    Some(if result.is_ok() { 0 } else { 1 })
}
