use crate::modules::logger;
use std::fs;
use std::io;
use std::path::Path;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const RESTORE_MODIFIED_TIME_RETRY_DELAYS: [Duration; 3] = [
    Duration::from_millis(50),
    Duration::from_millis(150),
    Duration::from_millis(300),
];

pub fn read_modified_time(path: &Path) -> Option<SystemTime> {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
}

pub fn restore_modified_time(path: &Path, modified_at: Option<SystemTime>) -> Result<(), String> {
    let Some(modified_at) = modified_at else {
        return Ok(());
    };
    if let Err(error) = restore_modified_time_with_retry(path, modified_at) {
        logger::log_warn(&format!(
            "恢复文件修改时间失败，已忽略 ({}): {}",
            path.display(),
            error
        ));
    }
    Ok(())
}

fn restore_modified_time_with_retry(path: &Path, modified_at: SystemTime) -> io::Result<()> {
    for delay in [None]
        .into_iter()
        .chain(RESTORE_MODIFIED_TIME_RETRY_DELAYS.into_iter().map(Some))
    {
        if let Some(delay) = delay {
            thread::sleep(delay);
        }
        match open_modified_time_handle(path).and_then(|file| file.set_modified(modified_at)) {
            Ok(()) => return Ok(()),
            Err(error) => {
                if delay == Some(RESTORE_MODIFIED_TIME_RETRY_DELAYS[2]) {
                    return Err(error);
                }
            }
        }
    }
    Ok(())
}

fn open_modified_time_handle(path: &Path) -> io::Result<fs::File> {
    #[cfg(windows)]
    {
        use std::fs::OpenOptions;
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_WRITE_ATTRIBUTES: u32 = 0x0100;
        OpenOptions::new()
            .access_mode(FILE_WRITE_ATTRIBUTES)
            .open(path)
    }

    #[cfg(not(windows))]
    {
        fs::File::open(path)
    }
}

pub fn system_time_from_unix_millis(timestamp_ms: i128) -> Option<SystemTime> {
    if timestamp_ms < 0 || timestamp_ms > u64::MAX as i128 {
        return None;
    }
    UNIX_EPOCH.checked_add(Duration::from_millis(timestamp_ms as u64))
}

pub fn same_modified_time_millis(left: Option<SystemTime>, right: Option<SystemTime>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => {
            let left = left
                .duration_since(UNIX_EPOCH)
                .ok()
                .map(|value| value.as_millis());
            let right = right
                .duration_since(UNIX_EPOCH)
                .ok()
                .map(|value| value.as_millis());
            left == right
        }
        (None, None) => true,
        _ => false,
    }
}
