//! Native Qoder client adapters for the global and public CN apps. Credential
//! schemas and CLI encryption are adapted from workbuddy-switch-next (MIT);
//! the realm is always explicit and selects both the copy and the store.
use crate::models::qoder::QoderAccount;
use crate::modules::{
    atomic_write, logger, qoder_account,
    qoder_channel::{Channel, CredentialSource},
    qoder_cn_account::QoderCnAccountStore,
    qoder_native_profile,
};
use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
#[cfg(windows)]
use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit, Nonce};
use base64::{engine::general_purpose::STANDARD, Engine};
use fs2::FileExt;
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

/// Serializes import and switch: a switch must never read the credentials an
/// import is in the middle of writing, and vice versa.
static SWITCH_LOCK: Mutex<()> = Mutex::new(());

/// How many previous credential sets to keep under `qoder-switch-backups`.
const SWITCH_BACKUP_KEEP: usize = 5;

/// Per-account overlays the desktop client writes next to its credentials.
const DESKTOP_ACCOUNT_OVERLAYS: &[&str] =
    &["auth-profile-overlays.v1.dat", "channel-activation.v1.json"];

fn io_error(error: std::io::Error) -> String {
    format!("QODER_NATIVE_STORAGE: {error}")
}

/// The realm a native adapter writes into. The two clients keep separate
/// credential stores and separate data roots, so a realm also selects the store
/// an imported account belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeRegion {
    Global,
    Cn,
}

impl NativeRegion {
    pub fn parse(value: Option<&str>) -> Result<Self, String> {
        match value.unwrap_or("global") {
            "global" => Ok(Self::Global),
            "cn" => Ok(Self::Cn),
            _ => Err("QODER_REGION_INVALID".into()),
        }
    }

    fn channel(self) -> Result<Channel, String> {
        let source = match self {
            Self::Global => CredentialSource::GlobalIde,
            Self::Cn => CredentialSource::PublicCn,
        };
        Channel::for_source(source).ok_or_else(|| "QODER_REGION_INVALID".into())
    }

    /// Platform id of the pointer that records what the client is signed into.
    fn platform_id(self) -> &'static str {
        match self {
            Self::Global => "qoder",
            Self::Cn => "qoder_cn",
        }
    }

    fn desktop_process_names(self) -> &'static [&'static str] {
        match self {
            Self::Global => GLOBAL_DESKTOP_PROCESS_NAMES,
            Self::Cn => CN_DESKTOP_PROCESS_NAMES,
        }
    }

    fn cli_process_names(self) -> &'static [&'static str] {
        match self {
            Self::Global => GLOBAL_CLI_PROCESS_NAMES,
            Self::Cn => CN_CLI_PROCESS_NAMES,
        }
    }

    fn load_account(self, account_id: &str) -> Result<QoderAccount, String> {
        let account = match self {
            Self::Global => qoder_account::load_account(account_id),
            Self::Cn => QoderCnAccountStore::default_store()?.load_account(account_id)?,
        };
        account.ok_or_else(|| "QODER_SESSION_CHANGED".into())
    }

    fn upsert(self, info: Value) -> Result<QoderAccount, String> {
        match self {
            Self::Global => qoder_account::upsert_account_from_snapshot(info, None, None),
            Self::Cn => {
                QoderCnAccountStore::default_store()?.upsert_from_snapshot(info, None, None)
            }
        }
    }

    /// Records that the credentials were just written to the client.
    fn mark_switched(self, account_id: &str) -> Result<(), String> {
        match self {
            Self::Global => qoder_account::mark_switched(account_id),
            Self::Cn => QoderCnAccountStore::default_store()?.mark_switched(account_id),
        }
    }
}

pub(crate) fn write_t3_cli_profile(
    region: NativeRegion,
    account_id: &str,
    config_dir: &Path,
) -> Result<String, String> {
    let account = region.load_account(account_id)?;
    if super::qoder_channel::credential_source(&account)? != region.channel()?.source() {
        return Err("QODER_PLATFORM_MISMATCH".into());
    }
    let info = account.auth_user_info_raw.as_ref().ok_or("QODER_NATIVE_TOKEN_MISSING")?;
    let profile = qoder_native_profile::for_client(info, false)?;
    write_scoped_cli_profile(config_dir, &profile)?;
    Ok(account.email)
}

fn write_scoped_cli_profile(config_dir: &Path, profile: &Value) -> Result<(), String> {
    let root = config_dir.join(".auth");
    initialize_cli_root(&root)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(config_dir, fs::Permissions::from_mode(0o700)).map_err(io_error)?;
    }
    let client = NativeClient::Cli;
    let encrypted = encode(&root, client, profile)?;
    atomic_write::write_bytes_atomic(&root.join(client.filename()), &encrypted)
}

fn initialize_cli_root(root: &Path) -> Result<(), String> {
    fs::create_dir_all(&root).map_err(io_error)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(root, fs::Permissions::from_mode(0o700)).map_err(io_error)?;
    }
    let machine_id = root.join("machine_id");
    // The CLI also accepts a generated UUID when there is no existing machine ID.
    if !machine_id.try_exists().map_err(io_error)? {
        atomic_write::write_bytes_atomic(&machine_id, uuid::Uuid::new_v4().to_string().as_bytes())?;
    }
    Ok(())
}

#[cfg(test)]
#[test]
fn qoder_cli_profiles_have_separate_credentials_and_stable_machine_ids() {
    let state = tempfile::tempdir().unwrap();
    let first = state.path().join("first");
    let second = state.path().join("second");
    let first_user = serde_json::json!({"userId": "first", "accessToken": "fixture-1"});
    let second_user = serde_json::json!({"userId": "second", "accessToken": "fixture-2"});
    write_scoped_cli_profile(&first, &first_user).unwrap();
    write_scoped_cli_profile(&second, &second_user).unwrap();
    let first_root = first.join(".auth");
    let second_root = second.join(".auth");
    let first_machine = fs::read(first_root.join("machine_id")).unwrap();
    let second_machine = fs::read(second_root.join("machine_id")).unwrap();
    assert_ne!(first_machine, second_machine);
    for (root, user) in [(&first_root, &first_user), (&second_root, &second_user)] {
        let client = NativeClient::Cli;
        let bytes = fs::read(root.join(client.filename())).unwrap();
        assert_eq!(&decode(root, client, &bytes).unwrap(), user);
    }
    write_scoped_cli_profile(&first, &first_user).unwrap();
    assert_eq!(fs::read(first_root.join("machine_id")).unwrap(), first_machine);
    assert_eq!(fs::read(second_root.join("machine_id")).unwrap(), second_machine);
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NativeClient {
    Desktop,
    Cli,
}

impl NativeClient {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "desktop" => Ok(Self::Desktop),
            "cli" => Ok(Self::Cli),
            _ => Err("QODER_CLIENT_INVALID".into()),
        }
    }

    fn root(self, channel: Channel) -> Result<PathBuf, String> {
        match self {
            Self::Desktop => Ok(dirs::config_dir()
                .ok_or("QODER_DATA_ROOT_UNAVAILABLE")?
                .join(channel.desktop_dir())),
            Self::Cli => Ok(dirs::home_dir()
                .ok_or("QODER_DATA_ROOT_UNAVAILABLE")?
                .join(channel.cli_dir())
                .join(".auth")),
        }
    }

    fn filename(self) -> &'static str {
        match self {
            Self::Desktop => "auth.v1.dat",
            Self::Cli => "user",
        }
    }
}

#[cfg(windows)]
fn unprotect(bytes: &[u8]) -> Result<Vec<u8>, String> {
    use windows::Win32::{
        Foundation::{LocalFree, HLOCAL},
        Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB},
    };
    let source = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(bytes.len()).map_err(|_| "QODER_NATIVE_KEY_INVALID")?,
        pbData: bytes.as_ptr() as *mut u8,
    };
    let mut result = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptUnprotectData(&source, None, None, None, None, 1, &mut result)
            .map_err(|_| "QODER_NATIVE_DPAPI_FAILED")?;
        let plain = std::slice::from_raw_parts(result.pbData, result.cbData as usize).to_vec();
        let _ = LocalFree(HLOCAL(result.pbData as _));
        Ok(plain)
    }
}

#[cfg(windows)]
fn desktop_key(root: &Path) -> Result<Vec<u8>, String> {
    #[cfg(windows)]
    {
        let state: Value =
            serde_json::from_slice(&fs::read(root.join("Local State")).map_err(io_error)?)
                .map_err(|_| "QODER_NATIVE_LOCAL_STATE_INVALID")?;
        let raw = STANDARD
            .decode(
                state
                    .pointer("/os_crypt/encrypted_key")
                    .and_then(Value::as_str)
                    .ok_or("QODER_NATIVE_KEY_MISSING")?,
            )
            .map_err(|_| "QODER_NATIVE_KEY_INVALID")?;
        if !raw.starts_with(b"DPAPI") {
            return Err("QODER_NATIVE_KEY_UNSUPPORTED".into());
        }
        let key = unprotect(&raw[5..])?;
        if key.len() != 32 {
            return Err("QODER_NATIVE_KEY_INVALID".into());
        }
        Ok(key)
    }
}

fn decode(root: &Path, client: NativeClient, bytes: &[u8]) -> Result<Value, String> {
    let plain = match client {
        NativeClient::Desktop => {
            #[cfg(target_os = "macos")]
            {
                super::vscode_inject::decode_qoder_desktop(root, bytes)?
            }
            #[cfg(windows)]
            {
            if !bytes.starts_with(b"v10") || bytes.len() < 31 {
                return Err("QODER_NATIVE_ENVELOPE_UNSUPPORTED".into());
            }
            Aes256Gcm::new_from_slice(&desktop_key(root)?)
                .map_err(|_| "QODER_NATIVE_KEY_INVALID")?
                .decrypt(Nonce::from_slice(&bytes[3..15]), &bytes[15..])
                .map_err(|_| "QODER_NATIVE_DECRYPT_FAILED")?
            }
            #[cfg(not(any(windows, target_os = "macos")))]
            return Err("QODER_NATIVE_DESKTOP_UNSUPPORTED".into());
        }
        NativeClient::Cli => {
            if bytes.first() == Some(&b'{') {
                bytes.to_vec()
            } else {
                let machine = fs::read_to_string(root.join("machine_id")).map_err(io_error)?;
                let key = machine
                    .trim()
                    .as_bytes()
                    .get(..16)
                    .ok_or("QODER_NATIVE_MACHINE_ID_INVALID")?;
                let mut encrypted = STANDARD
                    .decode(bytes)
                    .map_err(|_| "QODER_NATIVE_ENVELOPE_INVALID")?;
                cbc::Decryptor::<aes::Aes128>::new_from_slices(key, key)
                    .map_err(|_| "QODER_NATIVE_KEY_INVALID")?
                    .decrypt_padded_mut::<Pkcs7>(&mut encrypted)
                    .map_err(|_| "QODER_NATIVE_DECRYPT_FAILED")?
                    .to_vec()
            }
        }
    };
    serde_json::from_slice(&plain).map_err(|_| "QODER_NATIVE_JSON_INVALID".into())
}

fn encode(root: &Path, client: NativeClient, profile: &Value) -> Result<Vec<u8>, String> {
    let plain = serde_json::to_vec(profile).map_err(|_| "QODER_NATIVE_JSON_INVALID")?;
    let bytes = match client {
        NativeClient::Desktop => {
            #[cfg(target_os = "macos")]
            {
                super::vscode_inject::encode_qoder_desktop(root, &plain)?
            }
            #[cfg(windows)]
            {
            use aes_gcm::aead::{AeadCore, OsRng};
            let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
            let cipher = Aes256Gcm::new_from_slice(&desktop_key(root)?)
                .map_err(|_| "QODER_NATIVE_KEY_INVALID")?
                .encrypt(&nonce, plain.as_slice())
                .map_err(|_| "QODER_NATIVE_ENCRYPT_FAILED")?;
            [b"v10".as_slice(), nonce.as_slice(), &cipher].concat()
            }
            #[cfg(not(any(windows, target_os = "macos")))]
            return Err("QODER_NATIVE_DESKTOP_UNSUPPORTED".into());
        }
        NativeClient::Cli => {
            let machine = fs::read_to_string(root.join("machine_id")).map_err(io_error)?;
            let key = machine
                .trim()
                .as_bytes()
                .get(..16)
                .ok_or("QODER_NATIVE_MACHINE_ID_INVALID")?;
            let mut buffer = plain.clone();
            buffer.resize(plain.len() + 16, 0);
            let encrypted = cbc::Encryptor::<aes::Aes128>::new_from_slices(key, key)
                .map_err(|_| "QODER_NATIVE_KEY_INVALID")?
                .encrypt_padded_mut::<Pkcs7>(&mut buffer, plain.len())
                .map_err(|_| "QODER_NATIVE_ENCRYPT_FAILED")?;
            STANDARD.encode(encrypted).into_bytes()
        }
    };
    // Never hand the client a file it would read back as something else.
    if decode(root, client, &bytes)? != *profile {
        return Err("QODER_NATIVE_ROUNDTRIP_FAILED".into());
    }
    Ok(bytes)
}

/// 全局客户端独占的名字（CN 版没有 `Qoder.exe`）。
const GLOBAL_DESKTOP_PROCESS_NAMES: &[&str] =
    &["qoder.exe", "qoder", "qoder launcher.exe", "qoderlauncher.exe", "qoderlauncher"];
const GLOBAL_CLI_PROCESS_NAMES: &[&str] = &["qodercli.exe", "qodercli"];

/// The CN client shares its launcher process names with the global one. Matching
/// on names alone either kills the global client (`taskkill /T /F` takes the
/// child tree) or lets the resident global launcher block CN switching, so a
/// match must also sit inside the CN install directory.
pub(crate) const CN_DESKTOP_PROCESS_NAMES: &[&str] = &[
    "qoder cn.exe",
    "qoder cn",
    "qoder launcher.exe",
    "qoderlauncher.exe",
    "qoderlauncher",
];
const CN_CLI_PROCESS_NAMES: &[&str] = &["qoderclicn.exe", "qoderclicn"];
/// Path fragments that only the CN installer produces.
const CN_INSTALL_MARKERS: &[&str] = &["\\qoder cn\\", "\\qodercn\\", "/qoder cn/", "/qodercn/"];

fn normalize_path_key(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase()
}

fn path_is_within(path: &Path, dir: &Path) -> bool {
    let target = normalize_path_key(path);
    let mut root = normalize_path_key(dir);
    if root.is_empty() || target.is_empty() {
        return false;
    }
    if !root.ends_with('\\') {
        root.push('\\');
    }
    target.starts_with(&root)
}

/// Names that cannot belong to the global client, usable when no install
/// directory is known.
fn is_unambiguous_cn_name(name_lower: &str) -> bool {
    name_lower == "qoder cn.exe" || name_lower == "qoder cn"
}

/// Whether a process is the CN desktop client. Without a known install
/// directory only unambiguous names count, so the global client is never taken
/// for the CN one.
pub(crate) fn is_qoder_cn_desktop_process(
    name_lower: &str,
    exe: Option<&Path>,
    cn_install_dir: Option<&Path>,
) -> bool {
    if !CN_DESKTOP_PROCESS_NAMES.contains(&name_lower) {
        return false;
    }
    if let (Some(exe), Some(dir)) = (exe, cn_install_dir) {
        if exe.as_os_str().is_empty() {
            return is_unambiguous_cn_name(name_lower);
        }
        return path_is_within(exe, dir);
    }
    is_unambiguous_cn_name(name_lower)
}

/// The CN install directory, learned from a running client. Deliberately not
/// read from config: a stale override must not redirect a kill at the global
/// client.
pub(crate) fn qoder_cn_install_dir(system: &sysinfo::System) -> Option<PathBuf> {
    system
        .processes()
        .values()
        .filter_map(|process| process.exe())
        .map(PathBuf::from)
        .find(|path| {
            let key = normalize_path_key(path);
            CN_INSTALL_MARKERS.iter().any(|marker| key.contains(marker))
        })
        .and_then(|path| path.parent().map(Path::to_path_buf))
}

pub(crate) fn qoder_cn_desktop_pids(system: &sysinfo::System) -> Vec<u32> {
    #[cfg(target_os = "macos")]
    return system.processes().values().filter_map(|process| {
        let name = process.name().to_string_lossy();
        if !is_unambiguous_cn_name(&name.to_ascii_lowercase())
            || !super::qoder_cn_clients::mac_current_user_process(process)
        {
            return None;
        }
        process.exe()
            .filter(|path| super::qoder_cn_clients::mac_main_process(path, super::qoder_cn_clients::Client::Desktop))
            .map(|_| process.pid().as_u32())
    }).collect();
    #[cfg(not(target_os = "macos"))]
    {
        let dir = qoder_cn_install_dir(system);
        let mut pids: Vec<u32> = system
            .processes()
            .values()
            .filter(|process| {
                let name = process.name().to_string_lossy().to_ascii_lowercase();
                is_qoder_cn_desktop_process(&name, process.exe(), dir.as_deref())
            })
            .map(|process| process.pid().as_u32())
            .collect();
        pids.sort_unstable();
        pids.dedup();
        pids
    }
}

/// Executable of the running CN desktop client, so a switch can reopen exactly
/// the binary it closed instead of guessing an install path.
pub(crate) fn qoder_cn_desktop_executable(system: &sysinfo::System) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    return qoder_cn_desktop_pids(system).into_iter().find_map(|pid| {
        system.process(sysinfo::Pid::from_u32(pid))?.exe().map(Path::to_path_buf)
    });
    #[cfg(not(target_os = "macos"))]
    {
        let dir = qoder_cn_install_dir(system);
        system
            .processes()
            .values()
            .filter(|process| {
                let name = process.name().to_string_lossy().to_ascii_lowercase();
                is_qoder_cn_desktop_process(&name, process.exe(), dir.as_deref())
            })
            .filter_map(|process| process.exe())
            .map(PathBuf::from)
            .find(|path| path.is_file())
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn mac_global_desktop_pids(system: &sysinfo::System) -> Vec<u32> {
    use super::qoder_cn_clients::{mac_current_user_process, mac_global_desktop_executable};
    system.processes().values().filter_map(|process| {
        if !process.name().eq_ignore_ascii_case("Qoder") || !mac_current_user_process(process) {
            return None;
        }
        let path = process.exe()?;
        (mac_global_desktop_executable(path).as_deref() == Some(path)).then_some(process.pid().as_u32())
    }).collect()
}

/// Whether the target client is running. macOS Qoder Desktop and IDE can
/// share an executable name, so desktop clients use the bundle identity.
#[cfg(windows)]
pub(crate) fn windows_global_desktop_pids(system: &sysinfo::System, executable: &Path) -> Vec<u32> {
    let expected = normalize_path_key(executable);
    system.processes().values().filter_map(|process| {
        let path = process.exe()?;
        (normalize_path_key(path) == expected).then_some(process.pid().as_u32())
    }).collect()
}

fn ensure_idle(region: NativeRegion, client: NativeClient) -> Result<(), String> {
    let system = sysinfo::System::new_all();
    if system.processes().is_empty() {
        return Err("QODER_NATIVE_PROCESS_PROBE_FAILED".into());
    }
    let running = match (region, client) {
        (NativeRegion::Cn, NativeClient::Desktop) => !qoder_cn_desktop_pids(&system).is_empty(),
        #[cfg(target_os = "macos")]
        (NativeRegion::Global, NativeClient::Desktop) => !mac_global_desktop_pids(&system).is_empty(),
        #[cfg(windows)]
        (NativeRegion::Global, NativeClient::Desktop) => {
            let settings = super::config::get_user_config();
            super::qoder_cn_clients::resolve_global_desktop(Some(&settings.qoder_desktop_app_path))
                .is_some_and(|path| !windows_global_desktop_pids(&system, &path).is_empty())
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        (region, NativeClient::Desktop) => system.processes().values().any(|process| {
            region.desktop_process_names().contains(
                &process
                    .name()
                    .to_string_lossy()
                    .to_ascii_lowercase()
                    .as_str(),
            )
        }),
        (region, NativeClient::Cli) => system.processes().values().any(|process| {
            region.cli_process_names().contains(
                &process
                    .name()
                    .to_string_lossy()
                    .to_ascii_lowercase()
                    .as_str(),
            )
        }),
    };
    if running {
        // Switching under a live client would be undone by its own next write.
        let detail = match client {
            NativeClient::Desktop => "desktop",
            NativeClient::Cli => "cli",
        };
        return Err(format!("QODER_CLIENT_RUNNING: {detail}"));
    }
    Ok(())
}

/// Every switch parks the previous credentials in its own directory; keep only
/// the most recent ones so ciphertext cannot pile up forever.
fn prune_switch_backups(root: &Path, keep: usize) -> usize {
    let Ok(entries) = fs::read_dir(root) else {
        return 0;
    };
    let mut backups: Vec<(std::time::SystemTime, PathBuf)> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_dir() {
                return None;
            }
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, path))
        })
        .collect();
    if backups.len() <= keep {
        return 0;
    }
    backups.sort_by(|left, right| right.0.cmp(&left.0));
    let mut removed = 0usize;
    for (_, path) in backups.into_iter().skip(keep) {
        match fs::remove_dir_all(&path) {
            Ok(()) => removed += 1,
            Err(error) => logger::log_warn(&format!(
                "[Qoder Switch] 清理旧备份失败: path={}, error={error}",
                path.display()
            )),
        }
    }
    removed
}

/// Writes one account's credentials into the installed client. The previous
/// file is backed up and restored if the write cannot be read back.
pub fn switch(
    region: NativeRegion,
    account_id: &str,
    client: NativeClient,
) -> Result<(), String> {
    let _guard = SWITCH_LOCK.lock().map_err(|_| "QODER_ACCOUNT_BUSY")?;
    let channel = region.channel()?;
    let account = region.load_account(account_id)?;
    let info = account
        .auth_user_info_raw
        .as_ref()
        .ok_or("QODER_NATIVE_TOKEN_MISSING")?;
    // A credential only works in the client that issued it; writing a global
    // account into the CN client would leave both sides inconsistent.
    if super::qoder_channel::credential_source(&account)? != channel.source() {
        return Err("QODER_PLATFORM_MISMATCH".into());
    }
    ensure_idle(region, client)?;
    let profile = qoder_native_profile::for_client(info, client == NativeClient::Desktop)?;
    let root = client.root(channel)?;
    if client == NativeClient::Cli && !root.join("machine_id").try_exists().map_err(io_error)? {
        initialize_cli_root(&root)?;
    }
    if !root.is_dir() {
        return Err("QODER_CLIENT_INITIALIZATION_REQUIRED".into());
    }
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join("cockpit-account-switch.lock"))
        .map_err(io_error)?;
    lock.try_lock_exclusive()
        .map_err(|_| "QODER_ACCOUNT_BUSY")?;
    let path = root.join(client.filename());
    let previous = match fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(io_error(error)),
    };
    let next = encode(&root, client, &profile)?;
    let backup_root = crate::modules::account::get_data_dir()?.join("qoder-switch-backups");
    prune_switch_backups(&backup_root, SWITCH_BACKUP_KEEP.saturating_sub(1));
    let backup = backup_root.join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&backup).map_err(io_error)?;
    if let Some(previous) = &previous {
        atomic_write::write_bytes_atomic(&backup.join(client.filename()), previous)?;
    }
    // The desktop client keeps per-account overlays next to its credentials. They
    // describe the account on its way out, so they are cleared with the switch and
    // restored on rollback.
    let mut overlays = Vec::new();
    if client == NativeClient::Desktop {
        for name in DESKTOP_ACCOUNT_OVERLAYS {
            let target = root.join(name);
            match fs::read(&target) {
                Ok(bytes) => {
                    atomic_write::write_bytes_atomic(&backup.join(name), &bytes)?;
                    overlays.push((target, bytes));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(io_error(error)),
            }
        }
    }
    let mut write_started = false;
    let result = (|| -> Result<(), String> {
        ensure_idle(region, client)?;
        if fs::read(&path).ok() != previous {
            return Err("QODER_SESSION_CHANGED".into());
        }
        atomic_write::write_bytes_atomic(&path, &next)?;
        // Only now can a rollback be needed: a failed atomic write leaves the
        // original file intact, and restoring it would be a second mutation.
        write_started = true;
        for (target, _) in &overlays {
            atomic_write::remove_file_locked(target)?;
        }
        if decode(&root, client, &fs::read(&path).map_err(io_error)?)? != profile {
            return Err("QODER_NATIVE_READBACK_FAILED".into());
        }
        Ok(())
    })();
    if let Err(error) = result {
        if !write_started {
            return Err(error);
        }
        let restore = match &previous {
            Some(bytes) => atomic_write::write_bytes_atomic(&path, bytes),
            None => fs::remove_file(&path).map_err(io_error),
        };
        let mut rollback_failed = restore.is_err();
        for (target, bytes) in &overlays {
            rollback_failed |= atomic_write::write_bytes_atomic(target, bytes).is_err();
        }
        if rollback_failed {
            return Err(format!(
                "{error}; 回滚未完成; 备份目录: {}",
                backup.display()
            ));
        }
        return Err(error);
    }
    region.mark_switched(account_id)?;
    // The CN client stores are outside the app's control, so the recorded pointer
    // is the only way to show which account they currently hold. The global
    // platform derives its pointer from the IDE database instead, so writing it
    // here would fight the next reconciliation. A failure must not turn a
    // completed write into an error: the credentials are already in place, and a
    // retry would trip the session-changed guard.
    if region == NativeRegion::Cn {
        if let Err(error) = super::provider_current_state::set_current_account_id(
            region.platform_id(),
            Some(account_id),
        ) {
            logger::log_warn(&format!(
                "[Qoder CN Switch] 记录当前账号失败: account_id={account_id}, error={error}"
            ));
        }
    }
    Ok(())
}

#[cfg(any(windows, target_os = "macos"))]
pub(crate) fn desktop_ready(region: NativeRegion) -> Result<bool, String> {
    let root = NativeClient::Desktop.root(region.channel()?)?;
    let platform = match region {
        NativeRegion::Global => "qoder_desktop",
        NativeRegion::Cn => "qoder_cn_desktop",
    };
    Ok(root.try_exists().map_err(io_error)?
        && super::vscode_inject::client_secret_storage_ready(platform, &root)?)
}

#[cfg(target_os = "macos")]
pub(crate) fn switch_cn_ide(account_id: &str) -> Result<(), String> {
    let _guard = SWITCH_LOCK.lock().map_err(|_| "QODER_ACCOUNT_BUSY")?;
    let region = NativeRegion::Cn;
    let account = region.load_account(account_id)?;
    if super::qoder_channel::credential_source(&account)? != CredentialSource::PublicCn {
        return Err("QODER_PLATFORM_MISMATCH".into());
    }
    let db = dirs::config_dir()
        .ok_or("QODER_DATA_ROOT_UNAVAILABLE")?
        .join("QoderCN/User/globalStorage/state.vscdb");
    if !db.is_file() {
        return Err("QODER_CN_IDE_INITIALIZATION_REQUIRED".into());
    }
    super::qoder_account::inject_cn_ide_snapshot(&db, &account)?;
    if let Err(error) = region.mark_switched(account_id) {
        logger::log_warn(&format!("[Qoder CN Switch] 记录 IDE 切换时间失败: {error}"));
    }
    if let Err(error) =
        super::provider_current_state::set_current_account_id(region.platform_id(), Some(account_id))
    {
        logger::log_warn(&format!("[Qoder CN Switch] 记录当前账号失败: {error}"));
    }
    Ok(())
}

/// Stable key for the account the client of `region` is signed into, used by the
/// local auto-import watcher to notice a change without importing first.
pub fn local_identity_key(region: NativeRegion) -> Option<String> {
    let channel = region.channel().ok()?;
    for client in [NativeClient::Desktop, NativeClient::Cli] {
        let Ok(root) = client.root(channel) else {
            continue;
        };
        let path = root.join(client.filename());
        if !path.exists() {
            continue;
        }
        let Ok(bytes) = fs::read(&path) else {
            continue;
        };
        let Ok(profile) = decode(&root, client, &bytes) else {
            continue;
        };
        let Ok(info) = qoder_native_profile::from_client(&profile, channel) else {
            continue;
        };
        let user = info
            .get("id")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.trim().is_empty());
        if let Some(user) = user {
            return Some(serde_json::json!([channel.marker(), user]).to_string());
        }
    }
    None
}

/// CN accounts on disk. Used by the tray and the macOS menu, which both render
/// the platform without going through a command.
pub fn list_accounts() -> Vec<QoderAccount> {
    QoderCnAccountStore::default_store()
        .and_then(|store| store.list_accounts())
        .unwrap_or_default()
}

/// Which stored CN account is currently selected.
pub fn resolve_current_account_id(accounts: &[QoderAccount]) -> Option<String> {
    super::provider_current_state::resolve_existing_current_account_id(
        "qoder_cn",
        accounts.iter().map(|account| account.id.as_str()),
    )
}

/// 刷新后检查 CN 账号的剩余额度。CN 存储在 core 里、看不到当前账号状态，
/// 所以预警入口放在这个适配层。
pub fn run_quota_alert_if_needed(
) -> Result<Option<crate::modules::account::QuotaAlertPayload>, String> {
    super::qoder_quota_alert::run(super::qoder_quota_alert::QoderStore::Cn)
}

/// Reads the credentials the installed client currently holds and stores them as
/// an account in that region's store. Nothing is written back to the client.
pub fn import(region: NativeRegion, client: NativeClient) -> Result<QoderAccount, String> {
    // Shares the switch lock: reading credentials mid-switch would store the
    // account that is on its way out.
    let _guard = SWITCH_LOCK.lock().map_err(|_| "QODER_ACCOUNT_BUSY")?;
    let channel = region.channel()?;
    let root = client.root(channel)?;
    if !root.is_dir() {
        return Err("QODER_CLIENT_INITIALIZATION_REQUIRED".into());
    }
    let profile = decode(
        &root,
        client,
        &fs::read(root.join(client.filename())).map_err(io_error)?,
    )?;
    let info = qoder_native_profile::from_client(&profile, channel)?;
    region.upsert(info)
}

/// Imports whichever client profiles of `region` exist. Returns `Ok(false)` when
/// neither client has ever signed in on this machine.
pub fn import_from_local(region: NativeRegion) -> Result<bool, String> {
    let channel = region.channel()?;
    let mut found = false;
    let mut last_error = None;
    let mut imported = false;
    for client in [NativeClient::Desktop, NativeClient::Cli] {
        let path = client.root(channel)?.join(client.filename());
        if !path.exists() {
            continue;
        }
        found = true;
        match import(region, client) {
            Ok(_) => imported = true,
            Err(error) => {
                logger::log_warn(&format!(
                    "[Qoder Native] 导入客户端凭据失败: client={}, error={error}",
                    client.filename()
                ));
                last_error = Some(error);
            }
        }
    }
    if !found {
        return Ok(false);
    }
    if imported {
        Ok(true)
    } else {
        Err(last_error.unwrap_or_else(|| "QODER_NATIVE_IMPORT_FAILED".into()))
    }
}

/// Validates an import payload and stamps the CN marker before it reaches the
/// store, so a global account can never land in the CN index.
pub fn prepare_cn_import(value: &mut Value) -> Result<(), String> {
    qoder_native_profile::prepare_import(value, NativeRegion::Cn.channel()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The global client shares the launcher name, so a shared name alone must
    /// never make a process count as the CN client: switching would kill it.
    #[test]
    fn shared_launcher_names_need_the_cn_install_dir() {
        let cn_dir = Path::new(r"C:\Users\u\AppData\Local\Programs\Qoder CN");
        let global = Path::new(r"C:\Users\u\AppData\Local\Programs\Qoder\Qoder Launcher.exe");
        let cn = Path::new(r"C:\Users\u\AppData\Local\Programs\Qoder CN\Qoder Launcher.exe");

        assert!(!is_qoder_cn_desktop_process(
            "qoder launcher.exe",
            Some(global),
            Some(cn_dir)
        ));
        assert!(is_qoder_cn_desktop_process(
            "qoder launcher.exe",
            Some(cn),
            Some(cn_dir)
        ));
    }

    /// 两个 realm 必须落到不同的数据根和不同的 CLI 进程名：混用会把账号写进
    /// 对方客户端的凭据文件里。
    #[test]
    fn regions_select_their_own_channel_and_process_names() {
        assert_eq!(NativeRegion::parse(None).unwrap(), NativeRegion::Global);
        assert_eq!(
            NativeRegion::parse(Some("global")).unwrap(),
            NativeRegion::Global
        );
        assert_eq!(NativeRegion::parse(Some("cn")).unwrap(), NativeRegion::Cn);
        assert!(NativeRegion::parse(Some("qodercn")).is_err());

        let global = NativeRegion::Global.channel().unwrap();
        let cn = NativeRegion::Cn.channel().unwrap();
        assert_ne!(global.desktop_dir(), cn.desktop_dir());
        assert_ne!(global.cli_dir(), cn.cli_dir());
        assert_eq!(global.openapi_base_url(), "https://openapi.qoder.sh");
        assert_eq!(cn.openapi_base_url(), "https://openapi.qoder.com.cn");

        assert!(NativeRegion::Global
            .desktop_process_names()
            .contains(&"qoder.exe"));
        assert!(NativeRegion::Global.cli_process_names().contains(&"qodercli"));
        assert!(NativeRegion::Cn.cli_process_names().contains(&"qoderclicn"));
    }

    #[test]
    fn unambiguous_names_match_without_a_known_dir() {
        assert!(is_qoder_cn_desktop_process("qoder cn.exe", None, None));
        assert!(!is_qoder_cn_desktop_process("qoder launcher.exe", None, None));
        assert!(!is_qoder_cn_desktop_process("qoder.exe", None, None));
    }

    #[test]
    fn unknown_process_names_never_match() {
        let cn = Path::new(r"C:\Users\u\AppData\Local\Programs\Qoder CN\Qoder CN.exe");
        assert!(!is_qoder_cn_desktop_process("code.exe", Some(cn), Some(Path::new(
            r"C:\Users\u\AppData\Local\Programs\Qoder CN"
        ))));
    }
}
