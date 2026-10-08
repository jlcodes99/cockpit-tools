//! User-triggered installation of a pinned official engine. Never runs at app startup.
//! Downloads carry no account credentials; prepared releases are immutable and switched atomically.
use super::{account, atomic_write, codex_proxy_engine, codex_proxy_engine_archive as archive};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc, LazyLock, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const MAX_ARCHIVE: u64 = 128 * 1024 * 1024;
const INSTALL_TIMEOUT: Duration = Duration::from_secs(300);
// First launch may wait for macOS security assessment or antivirus scanning.
// This runs only in the cancellable install job, never in page/status reads.
const FIRST_LAUNCH_TIMEOUT: Duration = Duration::from_secs(30);
// Publication is one atomic directory rename. Windows keeps a handle on the freshly
// executed binary while security software scans it, which denies that rename for a
// moment, so the same rename is retried briefly instead of failing the install.
const PUBLISH_RENAME_ATTEMPTS: u32 = 12;
const PUBLISH_RENAME_INITIAL_DELAY: Duration = Duration::from_millis(50);
const PUBLISH_RENAME_MAX_DELAY: Duration = Duration::from_millis(500);
const MANIFEST: &str = include_str!("../../../sidecars/mihomo/upstream-assets.json");
const DOWNLOAD_BASE: &str = "https://github.com/MetaCubeX/mihomo/releases/download/";
static JOB: LazyLock<Mutex<Option<Job>>> = LazyLock::new(|| Mutex::new(None));

#[derive(Clone, Deserialize)]
pub(crate) struct Asset {
    pub file: String,
    pub sha256: String,
    pub size: u64,
}
#[derive(Deserialize)]
struct Manifest {
    version: String,
    assets: BTreeMap<String, Asset>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineInstallStatus {
    supported: bool,
    version: String,
    installed_version: Option<String>,
    asset_name: Option<String>,
    archive_bytes: Option<u64>,
    job_id: Option<String>,
    phase: String,
    received_bytes: u64,
    total_bytes: Option<u64>,
    error: Option<String>,
}
#[derive(Clone)]
struct Job {
    status: EngineInstallStatus,
    control: InstallControl,
}
#[derive(Clone)]
pub(crate) struct InstallControl {
    state: Arc<AtomicU8>,
    deadline: Instant,
    job_id: Option<String>,
}
impl InstallControl {
    pub(crate) fn check(&self) -> Result<(), String> {
        if self.state.load(Ordering::Acquire) == 1 {
            return Err("ENGINE_INSTALL_CANCELLED".into());
        }
        if Instant::now() >= self.deadline {
            return Err("ENGINE_INSTALL_TIMEOUT".into());
        }
        Ok(())
    }
    pub(crate) fn phase(&self, phase: &str) {
        self.update(|s| {
            if is_busy(&s.phase) {
                s.phase = phase.into();
            }
        });
    }
    fn update(&self, f: impl FnOnce(&mut EngineInstallStatus)) {
        if let Ok(mut guard) = JOB.lock() {
            if let Some(job) = guard.as_mut().filter(|j| j.status.job_id == self.job_id) {
                f(&mut job.status);
            }
        }
    }
    async fn interrupted(&self) -> String {
        loop {
            if let Err(e) = self.check() {
                return e;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    #[cfg(test)]
    pub(crate) fn for_test_cancelled() -> Self {
        let value = Self::for_test();
        value.state.store(1, Ordering::Release);
        value
    }
    #[cfg(test)]
    pub(crate) fn for_test_timed_out() -> Self {
        let mut value = Self::for_test();
        value.deadline = Instant::now();
        value
    }
    #[cfg(test)]
    pub(crate) fn for_test() -> Self {
        Self {
            state: Arc::new(AtomicU8::new(0)),
            deadline: Instant::now() + INSTALL_TIMEOUT,
            job_id: None,
        }
    }
}

fn platform_target() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        ("windows", "aarch64") => Some("aarch64-pc-windows-msvc"),
        ("windows", "x86_64") => Some("x86_64-pc-windows-msvc"),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-gnu"),
        ("linux", "x86_64") => Some("x86_64-unknown-linux-gnu"),
        _ => None,
    }
}
fn asset() -> Option<Asset> {
    let manifest: Manifest = serde_json::from_str(MANIFEST).ok()?;
    if manifest.version != codex_proxy_engine::ENGINE_VERSION {
        return None;
    }
    manifest.assets.get(platform_target()?).cloned()
}
fn root() -> Result<PathBuf, String> {
    account::resolve_data_dir()
        .map(|p| p.join("proxy-engine"))
        .map_err(|_| "ENGINE_INSTALL_IO".into())
}
fn binary_name() -> &'static str {
    if cfg!(windows) {
        "mihomo.exe"
    } else {
        "mihomo"
    }
}
fn idle() -> EngineInstallStatus {
    let asset = asset();
    EngineInstallStatus {
        supported: asset.is_some(),
        version: codex_proxy_engine::ENGINE_VERSION.into(),
        installed_version: None,
        asset_name: asset.as_ref().map(|a| a.file.clone()),
        archive_bytes: asset.map(|a| a.size),
        job_id: None,
        phase: "idle".into(),
        received_bytes: 0,
        total_bytes: None,
        error: None,
    }
}
fn is_busy(phase: &str) -> bool {
    matches!(
        phase,
        "downloading" | "importing" | "verifying" | "extracting" | "checking" | "installing"
    )
}

fn check_preflight_phase(phase: Option<&str>) -> Result<(), String> {
    if phase.is_some_and(is_busy) {
        return Err("ENGINE_INSTALL_BUSY".into());
    }
    Ok(())
}

/// Read only the in-memory installer phase; never scan disk or wait for its job.
pub(crate) fn check_not_installing() -> Result<(), String> {
    let job = JOB.lock().map_err(|_| "ENGINE_INSTALL_IO")?;
    check_preflight_phase(job.as_ref().map(|job| job.status.phase.as_str()))
}

#[derive(Serialize, Deserialize)]
struct Installed {
    directory: String,
    version: String,
    target: String,
    archive_sha256: String,
    files: BTreeMap<String, String>,
}
fn regular(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.is_file() && !m.file_type().is_symlink())
}
fn load_installed(root: &Path) -> Result<Option<Installed>, String> {
    load_record(root, &root.join("active.json"))
}
fn load_record(root: &Path, pointer: &Path) -> Result<Option<Installed>, String> {
    if !pointer.try_exists().map_err(|_| "ENGINE_INSTALL_IO")? {
        return Ok(None);
    }
    if !regular(&pointer)
        || fs::metadata(&pointer)
            .map_err(|_| "ENGINE_INSTALL_IO")?
            .len()
            > 32768
    {
        return Err("ENGINE_INSTALL_VERIFY".into());
    }
    let value: Installed =
        serde_json::from_slice(&fs::read(pointer).map_err(|_| "ENGINE_INSTALL_IO")?)
            .map_err(|_| "ENGINE_INSTALL_VERIFY")?;
    let id = value
        .directory
        .strip_prefix("release-")
        .ok_or("ENGINE_INSTALL_VERIFY")?;
    if uuid::Uuid::parse_str(id).is_err() {
        return Err("ENGINE_INSTALL_VERIFY".into());
    }
    let expected = asset().ok_or("ENGINE_INSTALL_UNSUPPORTED")?;
    if value.version != codex_proxy_engine::ENGINE_VERSION
        || Some(value.target.as_str()) != platform_target()
        || value.archive_sha256 != expected.sha256
    {
        return Ok(None);
    }
    let dir = root.join(&value.directory);
    if !fs::symlink_metadata(&dir).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink()) {
        return Err("ENGINE_INSTALL_VERIFY".into());
    }
    let required = [binary_name()];
    if required
        .iter()
        .any(|name| !value.files.contains_key(*name) || !regular(&dir.join(name)))
    {
        return Err("ENGINE_INSTALL_VERIFY".into());
    }
    // Only basename paths from our prepared manifest can be resolved for validation.
    if value.files.iter().any(|(name, hash)| {
        name.contains(['/', '\\', ':'])
            || name == "."
            || name == ".."
            || hash.len() != 64
            || !hash.bytes().all(|b| b.is_ascii_hexdigit())
    }) {
        return Err("ENGINE_INSTALL_VERIFY".into());
    }
    Ok(Some(value))
}
/// Tiny metadata lookup only. Hash verification is asynchronous, before execution.
pub(crate) fn managed_path() -> Result<Option<PathBuf>, String> {
    let root = root()?;
    Ok(load_installed(&root)?.map(|v| root.join(v.directory).join(binary_name())))
}
pub(crate) async fn verify_managed(binary: &Path) -> Result<(), String> {
    let root = root()?;
    verify_managed_in(root, binary).await
}
async fn verify_managed_in(root: PathBuf, binary: &Path) -> Result<(), String> {
    if !binary.starts_with(&root) {
        return Ok(());
    }
    let binary = binary.to_owned();
    tokio::time::timeout(
        Duration::from_secs(12),
        tokio::task::spawn_blocking(move || {
            let value = load_record(
                &root,
                &binary
                    .parent()
                    .ok_or("ENGINE_INSTALL_VERIFY")?
                    .join("installed.json"),
            )?
            .ok_or("ENGINE_INSTALL_VERIFY")?;
            if root.join(&value.directory).join(binary_name()) != binary {
                return Err("ENGINE_INSTALL_VERIFY".into());
            }
            let control = InstallControl {
                state: Arc::new(AtomicU8::new(0)),
                deadline: Instant::now() + Duration::from_secs(10),
                job_id: None,
            };
            for (name, expected) in value.files {
                if archive::file_sha256(&root.join(&value.directory).join(name), &control)
                    .map_err(|error| if error == "ENGINE_INSTALL_TIMEOUT" { error } else { "ENGINE_INSTALL_VERIFY".into() })?
                    != expected
                {
                    return Err("ENGINE_INSTALL_VERIFY".into());
                }
            }
            Ok(())
        }),
    )
    .await
    .map_err(|_| "ENGINE_INSTALL_TIMEOUT")?
    .map_err(|_| "ENGINE_INSTALL_VERIFY".to_string())?
}

pub async fn status() -> Result<EngineInstallStatus, String> {
    // Never block the async runtime on disk or start a child/network request for this read.
    let installed = tokio::time::timeout(
        Duration::from_secs(3),
        tokio::task::spawn_blocking(|| load_installed(&root()?)),
    )
    .await
    .map_err(|_| "ENGINE_INSTALL_TIMEOUT")?
    .map_err(|_| "ENGINE_INSTALL_IO")?;
    let mut result = JOB
        .lock()
        .map_err(|_| "ENGINE_INSTALL_IO")?
        .as_ref()
        .map(|j| j.status.clone())
        .unwrap_or_else(idle);
    match installed {
        Ok(installed) => {
            result.installed_version = installed.as_ref().map(|v| v.version.clone());
            // A slow atomic publication can finish after the supervisor's deadline.
            // Reconcile with durable state instead of leaving a successful install as failed.
            if installed.is_some_and(|v| {
                result
                    .job_id
                    .as_ref()
                    .is_some_and(|id| v.directory == format!("release-{id}"))
            }) && result.error.as_deref() == Some("ENGINE_INSTALL_TIMEOUT")
            {
                result.phase = "completed".into();
                result.error = None;
            }
        }
        Err(e) => {
            result.error = Some(e);
            if !is_busy(&result.phase) {
                result.phase = "failed".into();
            }
        }
    }
    Ok(result)
}

pub async fn begin(archive_path: Option<String>) -> Result<EngineInstallStatus, String> {
    let spec = asset().ok_or("ENGINE_INSTALL_UNSUPPORTED")?;
    let mut snapshot = idle();
    let id = uuid::Uuid::new_v4().to_string();
    snapshot.job_id = Some(id.clone());
    snapshot.phase = if archive_path.is_some() {
        "importing"
    } else {
        "downloading"
    }
    .into();
    snapshot.total_bytes = Some(spec.size);
    let control = InstallControl {
        state: Arc::new(AtomicU8::new(0)),
        deadline: Instant::now() + INSTALL_TIMEOUT,
        job_id: Some(id.clone()),
    };
    {
        let mut job = JOB.lock().map_err(|_| "ENGINE_INSTALL_IO")?;
        if job.as_ref().is_some_and(|j| is_busy(&j.status.phase)) {
            return Err("ENGINE_INSTALL_BUSY".into());
        }
        *job = Some(Job {
            status: snapshot.clone(),
            control: control.clone(),
        });
    }
    // The installer owns its lifetime; leaving a page does not abandon a half-written install.
    tokio::spawn(async move {
        let task_control = control.clone();
        let task = tokio::spawn(async move {
            install(spec, archive_path.map(PathBuf::from), task_control).await
        });
        let result = supervise(&control, task).await;
        match &result {
            Ok(()) => super::logger::log_info("[ProxyEngine] 用户请求的内核安装完成"),
            Err(code) => super::logger::log_warn(&format!("[ProxyEngine] 内核安装结束: {code}")),
        }
        control.update(|s| match result {
            Ok(()) => {
                s.phase = "completed".into();
                s.installed_version = Some(s.version.clone());
                s.error = None;
            }
            Err(e) => {
                s.phase = if e == "ENGINE_INSTALL_CANCELLED" {
                    "cancelled"
                } else {
                    "failed"
                }
                .into();
                s.error = Some(e);
            }
        });
    });
    Ok(snapshot)
}
async fn supervise(
    control: &InstallControl,
    mut task: tokio::task::JoinHandle<Result<(), String>>,
) -> Result<(), String> {
    tokio::select! {
        result = &mut task => result.unwrap_or_else(|_| Err("ENGINE_INSTALL_IO".into())),
        error = control.interrupted() => {
            // Detached worker retains ownership of all I/O and cleanup. No pre-commit
            // worker may publish after the UI has received cancellation / timeout.
            let _ = control.state.compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire);
            Err(error)
        }
    }
}
pub fn cancel(id: &str) -> Result<(), String> {
    let guard = JOB.lock().map_err(|_| "ENGINE_INSTALL_IO")?;
    if let Some(job) = guard
        .as_ref()
        .filter(|j| j.status.job_id.as_deref() == Some(id) && is_busy(&j.status.phase))
    {
        // Once the atomic publication starts, it finishes instead of reporting a false cancel.
        let _ = job
            .control
            .state
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire);
    }
    Ok(())
}

struct Staging {
    root: PathBuf,
    stage: PathBuf,
    release: PathBuf,
    lock: Option<File>,
    committed: bool,
}
impl Staging {
    fn cleanup(&mut self) {
        let _ = fs::remove_dir_all(&self.stage);
        if !self.committed {
            let _ = fs::remove_dir_all(&self.release);
        }
        if let Some(lock) = self.lock.take() {
            let _ = FileExt::unlock(&lock);
        }
    }
}
impl Drop for Staging {
    fn drop(&mut self) {
        if self.lock.is_none() {
            return;
        }
        let mut owned = Staging {
            root: self.root.clone(),
            stage: self.stage.clone(),
            release: self.release.clone(),
            lock: self.lock.take(),
            committed: self.committed,
        };
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn_blocking(move || owned.cleanup());
        } else {
            owned.cleanup();
        }
    }
}

/// A handle that Windows security scanning still holds (`ERROR_ACCESS_DENIED` /
/// `ERROR_SHARING_VIOLATION`) resolves on its own within a second or two.
fn transient_publish_error(error: &std::io::Error) -> bool {
    if error.kind() == std::io::ErrorKind::PermissionDenied {
        return true;
    }
    #[cfg(windows)]
    {
        return matches!(error.raw_os_error(), Some(5) | Some(32));
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Retry only transient handle/permission races. Every other error fails fast so the
/// caller keeps its staging directory and can still report the real cause.
fn retry_transient_publish<T>(
    mut operation: impl FnMut() -> std::io::Result<T>,
) -> std::io::Result<T> {
    let mut delay = PUBLISH_RENAME_INITIAL_DELAY;
    let mut attempt = 1_u32;
    loop {
        match operation() {
            Ok(value) => return Ok(value),
            Err(error) => {
                if attempt >= PUBLISH_RENAME_ATTEMPTS || !transient_publish_error(&error) {
                    return Err(error);
                }
                super::logger::log_warn(&format!(
                    "[ProxyEngine] 内核发布遇到临时占用，重试: attempt={attempt}/{PUBLISH_RENAME_ATTEMPTS}, kind={:?}, os_code={:?}",
                    error.kind(),
                    error.raw_os_error()
                ));
                std::thread::sleep(delay);
                delay = (delay * 2).min(PUBLISH_RENAME_MAX_DELAY);
                attempt += 1;
            }
        }
    }
}
fn private_dir(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|_| "ENGINE_INSTALL_IO")?;
    if fs::symlink_metadata(path)
        .map_err(|_| "ENGINE_INSTALL_IO")?
        .file_type()
        .is_symlink()
    {
        return Err("ENGINE_INSTALL_IO".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| "ENGINE_INSTALL_IO")?;
    }
    Ok(())
}
fn create_staging(root: PathBuf, id: &str) -> Result<Staging, String> {
    private_dir(&root)?;
    let lock_path = root.join("install.lock");
    if lock_path.is_symlink() {
        return Err("ENGINE_INSTALL_IO".into());
    }
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)
        .map_err(|_| "ENGINE_INSTALL_IO")?;
    lock.try_lock_exclusive()
        .map_err(|_| "ENGINE_INSTALL_BUSY")?;
    for entry in fs::read_dir(&root)
        .map_err(|_| "ENGINE_INSTALL_IO")?
        .flatten()
    {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name
            .strip_prefix("staging-")
            .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
        {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
    let stage = root.join(format!("staging-{id}"));
    let release = root.join(format!("release-{id}"));
    fs::create_dir(&stage).map_err(|_| "ENGINE_INSTALL_IO")?;
    let context = Staging {
        root,
        stage,
        release,
        lock: Some(lock),
        committed: false,
    };
    private_dir(&context.stage)?;
    Ok(context)
}
async fn install(
    spec: Asset,
    local: Option<PathBuf>,
    control: InstallControl,
) -> Result<(), String> {
    install_at(root()?, spec, local, control).await
}
async fn install_at(
    root: PathBuf,
    spec: Asset,
    local: Option<PathBuf>,
    control: InstallControl,
) -> Result<(), String> {
    control.check()?;
    let id = control
        .job_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let mut stage = tokio::task::spawn_blocking(move || create_staging(root, &id))
        .await
        .map_err(|_| "ENGINE_INSTALL_IO")??;
    let archive_path = stage.stage.join("download.archive");
    let output = stage.stage.join("engine");
    let result: Result<(), String> = async {
        transfer(&spec, local, &archive_path, &control).await?;
        control.check()?;
        let path = archive_path.clone();
        let dest = output.clone();
        let prepared_control = control.clone();
        let spec_copy = spec.clone();
        let files = tokio::task::spawn_blocking(move || {
            private_dir(&dest)?;
            archive::prepare(
                &path,
                &dest,
                &spec_copy.file,
                &spec_copy.sha256,
                &prepared_control,
            )
        })
        .await
        .map_err(|_| "ENGINE_INSTALL_ARCHIVE")??;
        control.check()?;
        // Hash-verified official binary only. Checks CPU/OS compatibility before publication.
        let staged_binary = output.join(binary_name());
        check_first_launch(&staged_binary, &control, FIRST_LAUNCH_TIMEOUT).await?;
        control.check()?;
        let installed = Installed {
            directory: stage
                .release
                .file_name()
                .ok_or("ENGINE_INSTALL_IO")?
                .to_string_lossy()
                .into_owned(),
            version: codex_proxy_engine::ENGINE_VERSION.into(),
            target: platform_target()
                .ok_or("ENGINE_INSTALL_UNSUPPORTED")?
                .into(),
            archive_sha256: spec.sha256,
            files,
        };
        let pointer = serde_json::to_string(&installed).map_err(|_| "ENGINE_INSTALL_IO")?;
        let commit_control = control.clone();
        // Move ownership into a blocking worker so cancellation never deletes a directory
        // while an extraction / fsync / atomic replacement still operates on it.
        let publish = tokio::task::spawn_blocking(move || {
            commit_control.check()?;
            commit_control
                .state
                .compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire)
                .map_err(|_| "ENGINE_INSTALL_CANCELLED")?;
            commit_control.phase("installing");
            // Each immutable release has its own record, so launching an old selected
            // path remains safe when another install updates active.json concurrently.
            atomic_write::write_secret_string_atomic(&output.join("installed.json"), &pointer)
                .map_err(|_| "ENGINE_INSTALL_IO")?;
            File::create(output.join(".lease")).map_err(|_| "ENGINE_INSTALL_IO")?;
            // The staged binary was just executed for the version check, so Windows may
            // still hold it open and deny this rename for a moment.
            retry_transient_publish(|| fs::rename(&output, &stage.release)).map_err(|error| {
                super::logger::log_warn(&format!(
                    "[ProxyEngine] 内核发布重命名失败: kind={:?}, os_code={:?}",
                    error.kind(),
                    error.raw_os_error()
                ));
                "ENGINE_INSTALL_IO".to_string()
            })?;
            atomic_write::write_secret_string_atomic(&stage.root.join("active.json"), &pointer)
                .map_err(|_| "ENGINE_INSTALL_IO")?;
            stage.committed = true;
            prune_releases(&stage.root, &installed.directory);
            stage.cleanup();
            Ok::<(), String>(())
        });
        publish.await.map_err(|_| "ENGINE_INSTALL_IO")?
    }
    .await;
    result
}

async fn check_first_launch(
    binary: &Path,
    control: &InstallControl,
    timeout: Duration,
) -> Result<(), String> {
    control.check()?;
    control.phase("checking");
    tokio::select! {
        e = control.interrupted() => Err(e),
        result = codex_proxy_engine::verify_version_with_timeout(binary, timeout) => result.map_err(|error| {
            match error.as_str() {
                "PROXY_ENGINE_TIMEOUT" => "ENGINE_INSTALL_START_TIMEOUT",
                "PROXY_ENGINE_VERSION" => "ENGINE_INSTALL_VERSION",
                _ => "ENGINE_INSTALL_START_FAILED",
            }.to_owned()
        }),
    }
}

fn approved_redirect(url: &url::Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && matches!(
            url.host_str(),
            Some(
                "github.com"
                    | "release-assets.githubusercontent.com"
                    | "objects.githubusercontent.com"
            )
        )
}
async fn transfer(
    spec: &Asset,
    local: Option<PathBuf>,
    target: &Path,
    control: &InstallControl,
) -> Result<(), String> {
    let mut output = tokio::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(target)
        .await
        .map_err(|_| "ENGINE_INSTALL_IO")?;
    let mut received = 0_u64;
    if let Some(path) = local {
        // Reject special files before opening: a FIFO/device must not block import.
        let metadata = tokio::fs::symlink_metadata(&path)
            .await
            .map_err(|_| "ENGINE_INSTALL_IO")?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err("ENGINE_INSTALL_ARCHIVE".into());
        }
        if metadata.len() > MAX_ARCHIVE {
            return Err("ENGINE_INSTALL_TOO_LARGE".into());
        }
        control.update(|s| s.total_bytes = Some(metadata.len()));
        let mut input = tokio::fs::File::open(path)
            .await
            .map_err(|_| "ENGINE_INSTALL_IO")?;
        let mut buf = vec![0_u8; 64 * 1024];
        loop {
            let count = tokio::select! { e = control.interrupted() => return Err(e), n = input.read(&mut buf) => n.map_err(|_| "ENGINE_INSTALL_IO")? };
            if count == 0 {
                break;
            }
            received += count as u64;
            if received > MAX_ARCHIVE {
                return Err("ENGINE_INSTALL_TOO_LARGE".into());
            }
            output
                .write_all(&buf[..count])
                .await
                .map_err(|_| "ENGINE_INSTALL_IO")?;
            control.update(|s| s.received_bytes = received);
        }
    } else {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(20))
            .timeout(INSTALL_TIMEOUT)
            .https_only(true)
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= 5 || !approved_redirect(attempt.url()) {
                    attempt.error("unapproved engine download redirect")
                } else {
                    attempt.follow()
                }
            }))
            .build()
            .map_err(|_| "ENGINE_INSTALL_DOWNLOAD")?;
        let url = format!(
            "{DOWNLOAD_BASE}v{}/{}",
            codex_proxy_engine::ENGINE_VERSION,
            spec.file
        );
        let response = tokio::select! { e = control.interrupted() => return Err(e), r = client.get(url).send() => r.map_err(download_error)? };
        let mut response = response.error_for_status().map_err(download_error)?;
        if response.content_length().is_some_and(|s| s > MAX_ARCHIVE) {
            return Err("ENGINE_INSTALL_TOO_LARGE".into());
        }
        loop {
            let chunk = tokio::select! { e = control.interrupted() => return Err(e), c = response.chunk() => c.map_err(download_error)? };
            let Some(chunk) = chunk else {
                break;
            };
            received += chunk.len() as u64;
            if received > MAX_ARCHIVE {
                return Err("ENGINE_INSTALL_TOO_LARGE".into());
            }
            output
                .write_all(&chunk)
                .await
                .map_err(|_| "ENGINE_INSTALL_IO")?;
            control.update(|s| s.received_bytes = received);
        }
    }
    output.sync_all().await.map_err(|_| "ENGINE_INSTALL_IO")?;
    control.check()
}
fn download_error(error: reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "ENGINE_INSTALL_TIMEOUT"
    } else {
        "ENGINE_INSTALL_DOWNLOAD"
    }
}

/// A running child keeps a shared lease. Cleanup can only retire unused releases.
pub(crate) fn lease_managed(binary: &Path) -> Result<Option<File>, String> {
    let root = root()?;
    if !binary.starts_with(&root) {
        return Ok(None);
    }
    let path = binary
        .parent()
        .ok_or("ENGINE_INSTALL_VERIFY")?
        .join(".lease");
    if !regular(&path) {
        return Err("ENGINE_INSTALL_VERIFY".into());
    }
    let file = File::open(path).map_err(|_| "ENGINE_INSTALL_IO")?;
    FileExt::try_lock_shared(&file).map_err(|_| "ENGINE_INSTALL_BUSY")?;
    Ok(Some(file))
}
fn prune_releases(root: &Path, active: &str) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    let mut releases: Vec<_> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name
                .strip_prefix("release-")
                .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
            {
                return None;
            }
            let metadata = fs::symlink_metadata(entry.path()).ok()?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() || name == active {
                return None;
            }
            Some((metadata.modified().ok()?, entry.path()))
        })
        .collect();
    releases.sort_by(|a, b| b.0.cmp(&a.0));
    // Keep one previous release as well as active. Never retire a leased process.
    for (_, path) in releases.into_iter().skip(1) {
        let lease = path.join(".lease");
        if !regular(&lease) {
            continue;
        }
        let Ok(lock) = OpenOptions::new().read(true).write(true).open(lease) else {
            continue;
        };
        if lock.try_lock_exclusive().is_err() {
            continue;
        }
        let garbage = root.join(format!("staging-{}", uuid::Uuid::new_v4()));
        // Rename while holding the exclusive lease: new launches cannot acquire the old path.
        if fs::rename(&path, &garbage).is_ok() {
            drop(lock);
            let _ = fs::remove_dir_all(garbage);
        }
    }
}

#[cfg(test)]
#[path = "codex_proxy_engine_install_tests.rs"]
mod tests;
