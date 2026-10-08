//! Early, windowless package launcher. It never initializes Tauri or account data.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const HELPER_ARG: &str = "--cockpit-codex-package-launch";
pub const PAYLOAD_ENV: &str = "COCKPIT_CODEX_PACKAGE_LAUNCH_PAYLOAD";

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct LaunchRequest {
    pub family_name: String,
    pub app_id: String,
    pub executable: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub codex_home: Option<String>,
    pub app_user_data_dir: Option<String>,
    pub result_path: PathBuf,
    pub nonce: String,
    #[serde(default)]
    pub verify_only: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum LaunchReply {
    Spawned {
        pid: u32,
        validation_us: u64,
        helper_has_console: bool,
    },
    Validated {
        validation_us: u64,
        helper_has_console: bool,
    },
    EntryStale,
    Failed {
        message: String,
    },
}

#[derive(Serialize, Deserialize)]
pub(super) struct Receipt {
    pub nonce: String,
    pub reply: LaunchReply,
}

/// Only non-secret status is written to this unique temporary directory. The
/// environment/arguments stay in process memory, and every attempt owns its receipt.
pub(super) struct PendingReceipt {
    directory: PathBuf,
    pub path: PathBuf,
    pub nonce: String,
}

impl PendingReceipt {
    pub fn new() -> std::io::Result<Self> {
        let directory =
            std::env::temp_dir().join(format!("cockpit-codex-launch-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory)?;
        Ok(Self {
            path: directory.join("result.json"),
            directory,
            nonce: uuid::Uuid::new_v4().to_string(),
        })
    }

    pub fn wait(&self, timeout: Duration) -> Option<LaunchReply> {
        let started = Instant::now();
        loop {
            if let Ok(bytes) = std::fs::read(&self.path) {
                if let Ok(receipt) = serde_json::from_slice::<Receipt>(&bytes) {
                    if receipt.nonce == self.nonce {
                        return Some(receipt.reply);
                    }
                }
            }
            if started.elapsed() >= timeout {
                return None;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for PendingReceipt {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_file(self.path.with_extension("pending"));
        let _ = std::fs::remove_dir(&self.directory);
    }
}

impl LaunchRequest {
    pub(super) fn child_command(&self) -> Command {
        let mut command = Command::new(&self.executable);
        command.args(&self.args).envs(self.env.iter().cloned());
        // No launch payload or inherited Codex profile reaches the official client.
        command.env_remove(PAYLOAD_ENV);
        for (key, value) in [
            ("CODEX_HOME", self.codex_home.as_deref()),
            (
                "CODEX_ELECTRON_USER_DATA_PATH",
                self.app_user_data_dir.as_deref(),
            ),
        ] {
            match value {
                Some(value) => {
                    command.env(key, value);
                }
                None => {
                    command.env_remove(key);
                }
            }
        }
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }

    fn reply(&self, reply: LaunchReply) -> std::io::Result<()> {
        let bytes = serde_json::to_vec(&Receipt {
            nonce: self.nonce.clone(),
            reply,
        })?;
        let pending = self.result_path.with_extension("pending");
        std::fs::write(&pending, bytes)?;
        std::fs::rename(pending, &self.result_path)
    }
}

#[cfg(target_os = "windows")]
fn current_package_string(family: bool) -> Result<String, String> {
    use windows::core::PWSTR;
    use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS};
    use windows::Win32::Storage::Packaging::Appx::{
        GetCurrentPackageFamilyName, GetCurrentPackagePath,
    };
    unsafe {
        let mut length = 0;
        let call = |length: &mut u32, buffer| {
            if family {
                GetCurrentPackageFamilyName(length, buffer)
            } else {
                GetCurrentPackagePath(length, buffer)
            }
        };
        let status = call(&mut length, PWSTR::null());
        if status != ERROR_INSUFFICIENT_BUFFER || length == 0 || length > 32768 {
            return Err(format!("Package context query failed: win32={}", status.0));
        }
        let mut buffer = vec![0u16; length as usize];
        let status = call(&mut length, PWSTR(buffer.as_mut_ptr()));
        if status != ERROR_SUCCESS {
            return Err(format!("Package context query failed: win32={}", status.0));
        }
        Ok(String::from_utf16_lossy(
            &buffer[..buffer
                .iter()
                .position(|unit| *unit == 0)
                .unwrap_or(buffer.len())],
        ))
    }
}

pub(super) fn matches_package_entry(
    request: &LaunchRequest,
    family: &str,
    root: &Path,
    manifest: &str,
) -> bool {
    request.family_name.eq_ignore_ascii_case(family)
        && super::codex_manifest_app_id(root, Path::new(&request.executable), manifest).as_deref()
            == Some(request.app_id.as_str())
}

#[cfg(target_os = "windows")]
fn launch(request: &LaunchRequest) -> LaunchReply {
    let started = Instant::now();
    let context = current_package_string(true)
        .and_then(|family| Ok((family, PathBuf::from(current_package_string(false)?))));
    let (family, root) = match context {
        Ok(context) => context,
        Err(message) => return LaunchReply::Failed { message },
    };
    let manifest = match std::fs::read_to_string(root.join("AppxManifest.xml")) {
        Ok(text) => text,
        Err(error) => {
            return LaunchReply::Failed {
                message: format!("Package manifest read failed: {error}"),
            }
        }
    };
    if !matches_package_entry(request, &family, &root, &manifest)
        || !Path::new(&request.executable).is_file()
    {
        return LaunchReply::EntryStale;
    }
    let validation_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
    #[link(name = "kernel32")]
    extern "system" {
        fn GetConsoleWindow() -> *mut std::ffi::c_void;
    }
    let helper_has_console = unsafe { !GetConsoleWindow().is_null() };
    if request.verify_only {
        return LaunchReply::Validated {
            validation_us,
            helper_has_console,
        };
    }
    match request.child_command().spawn() {
        Ok(child) => LaunchReply::Spawned {
            pid: child.id(),
            validation_us,
            helper_has_console,
        },
        Err(error) if matches!(error.raw_os_error(), Some(2 | 3)) => LaunchReply::EntryStale,
        Err(error) => LaunchReply::Failed {
            message: format!("Client process creation failed: {error}"),
        },
    }
}

/// Called before normal app initialization. Payload parsing failures never fall
/// through to opening a second manager window; no scripts or secrets go to disk.
#[cfg(target_os = "windows")]
pub(crate) fn try_run() -> Option<i32> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    if !args.iter().any(|arg| arg == HELPER_ARG) {
        return None;
    }
    if args.len() != 2 || args[0] != HELPER_ARG {
        return Some(2);
    }
    let result = (|| {
        use base64::{engine::general_purpose, Engine};
        let encoded = args[1].to_str().ok_or(())?;
        if encoded.len() > 30000 {
            return Err(());
        }
        let bytes = general_purpose::STANDARD.decode(encoded).map_err(|_| ())?;
        let request: LaunchRequest = serde_json::from_slice(&bytes).map_err(|_| ())?;
        if request.nonce.is_empty() || !request.result_path.is_absolute() {
            return Err(());
        }
        request.reply(launch(&request)).map_err(|_| ())
    })();
    Some(if result.is_ok() { 0 } else { 2 })
}
