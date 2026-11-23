use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::models::codebuddy::CodebuddyAccount;
use crate::modules::{account, atomic_write, secure_account_storage};

pub(crate) const RUN_ARG: &str = "--cockpit-codebuddy-cli";
pub(super) const INSTANCES_FILE: &str = "codebuddy_cli_instances.json";
const WORKING_DIR_FILE: &str = "working-dir.json";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Region {
    International,
    China,
}

impl Region {
    fn account_dir(self) -> &'static str {
        match self {
            Self::International => "codebuddy_accounts",
            Self::China => "codebuddy_cn_accounts",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountRef {
    pub region: Region,
    pub account_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Instance {
    pub id: uuid::Uuid,
    pub name: String,
    pub account: AccountRef,
    pub working_dir: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InstanceDraft {
    pub id: Option<uuid::Uuid>,
    pub name: String,
    pub account: AccountRef,
    pub working_dir: PathBuf,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct LaunchRequest {
    data_dir: PathBuf,
    account: AccountRef,
    working_dir: PathBuf,
    instance_id: Option<uuid::Uuid>,
    language: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    custom_command: Option<String>,
}

fn read_account(data_dir: &Path, reference: &AccountRef) -> Result<CodebuddyAccount, String> {
    let id = &reference.account_id;
    if id.is_empty()
        || id.contains("..")
        || !id
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || b"._-".contains(&ch))
    {
        return Err("accountMissing".into());
    }

    let path = data_dir
        .join(reference.region.account_dir())
        .join(format!("{id}.json"));
    if !path.is_file() {
        return Err("accountMissing".into());
    }
    let account: CodebuddyAccount = secure_account_storage::read_account_file_readonly(
        &path,
        &data_dir.join(super::account_file_crypto::KEY_FILE),
    )
    .map_err(|_| "accountUnreadable")?;
    if account.id != *id {
        return Err("accountMissing".into());
    }
    Ok(account)
}

fn valid_working_dir(path: &Path) -> bool {
    path.is_absolute() && path.is_dir()
}

fn parse_instances(content: &str) -> Result<Vec<Instance>, String> {
    serde_json::from_str(content).map_err(|_| "storageError".into())
}

pub(crate) fn list_instances() -> Result<Vec<Instance>, String> {
    let path = account::get_data_dir()
        .map_err(|_| "storageError")?
        .join(INSTANCES_FILE);
    match fs::read_to_string(path) {
        Ok(content) => parse_instances(&content),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(_) => Err("storageError".into()),
    }
}

fn update_instances<F>(data_dir: &Path, edit: F) -> Result<Vec<Instance>, String>
where
    F: FnOnce(&mut Vec<Instance>) -> Result<(), &'static str>,
{
    let mut updated = Vec::new();
    atomic_write::update_string_atomic(&data_dir.join(INSTANCES_FILE), |current| {
        let mut instances = current
            .map(parse_instances)
            .transpose()?
            .unwrap_or_default();
        edit(&mut instances)?;
        let content = serde_json::to_string_pretty(&instances).map_err(|_| "storageError")?;
        updated = instances;
        Ok(content)
    })
    .map_err(|error| {
        if error == "instanceMissing" {
            error
        } else {
            super::logger::log_warn(&format!("[CodeBuddy CLI] Instance write failed: {error}"));
            "storageError".into()
        }
    })?;
    Ok(updated)
}

pub(crate) fn save_instance(draft: InstanceDraft) -> Result<Vec<Instance>, String> {
    let data_dir = account::get_data_dir().map_err(|_| "storageError")?;
    let name = super::instance_store::normalize_name(&draft.name).map_err(|_| "nameRequired")?;
    if !valid_working_dir(&draft.working_dir) {
        return Err("workingDirInvalid".into());
    }
    read_account(&data_dir, &draft.account)?;
    let id = draft.id.unwrap_or_else(uuid::Uuid::new_v4);
    let instance = Instance {
        id,
        name,
        account: draft.account,
        working_dir: draft.working_dir,
    };
    update_instances(&data_dir, |instances| {
        if let Some(id) = draft.id {
            let existing = instances
                .iter_mut()
                .find(|instance| instance.id == id)
                .ok_or("instanceMissing")?;
            *existing = instance;
        } else {
            instances.push(instance);
        }
        Ok(())
    })
}

pub(crate) fn remove_instance(id: uuid::Uuid) -> Result<Vec<Instance>, String> {
    let data_dir = account::get_data_dir().map_err(|_| "storageError")?;
    update_instances(&data_dir, |instances| {
        let index = instances
            .iter()
            .position(|instance| instance.id == id)
            .ok_or("instanceMissing")?;
        instances.remove(index);
        Ok(())
    })
}

fn launch_profile_dir(request: &LaunchRequest) -> PathBuf {
    if let Some(id) = request.instance_id {
        return request
            .data_dir
            .join("codebuddy_cli_profiles/instances")
            .join(id.to_string());
    }
    account_profile_dir(&request.data_dir, &request.account)
}

fn account_profile_dir(data_dir: &Path, account: &AccountRef) -> PathBuf {
    let digest = Sha256::digest(account.account_id.as_bytes());
    data_dir
        .join("codebuddy_cli_profiles/accounts")
        .join(account.region.account_dir())
        .join(format!("{digest:x}"))
}

pub(crate) fn working_dir(account: &AccountRef) -> Result<Option<PathBuf>, String> {
    let data_dir = account::get_data_dir().map_err(|_| "storageError")?;
    read_working_dir(&data_dir, account)
}

fn read_working_dir(data_dir: &Path, account: &AccountRef) -> Result<Option<PathBuf>, String> {
    let path = account_profile_dir(data_dir, account).join(WORKING_DIR_FILE);
    match fs::read_to_string(path) {
        Ok(content) => serde_json::from_str(&content).map_err(|_| "storageError".into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("storageError".into()),
    }
}

fn credential(data_dir: &Path, reference: &AccountRef) -> Result<String, String> {
    let account = read_account(data_dir, reference)?;
    let token = account.access_token.trim();
    if token.is_empty() || token.contains('\0') {
        return Err("credentialMissing".into());
    }
    Ok(token.to_owned())
}

#[derive(Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum LaunchTarget {
    Instance {
        id: uuid::Uuid,
        working_dir: Option<PathBuf>,
    },
    Account {
        account: AccountRef,
        working_dir: Option<PathBuf>,
        #[serde(default)]
        remember_working_dir: bool,
    },
}

impl LaunchTarget {
    pub(crate) fn remembers_working_dir(&self) -> bool {
        match self {
            Self::Instance { working_dir, .. } => working_dir.is_some(),
            Self::Account {
                remember_working_dir,
                ..
            } => *remember_working_dir,
        }
    }
}

pub(crate) fn prepare_launch(
    target: LaunchTarget,
    custom_command: Option<String>,
) -> Result<LaunchRequest, String> {
    let data_dir = crate::modules::account::get_data_dir().map_err(|_| "storageError")?;
    let (account, working_dir, instance_id) = match target {
        LaunchTarget::Instance { id, working_dir } => {
            let instance = list_instances()?
                .into_iter()
                .find(|instance| instance.id == id)
                .ok_or("instanceMissing")?;
            (
                instance.account,
                working_dir.unwrap_or(instance.working_dir),
                Some(id),
            )
        }
        LaunchTarget::Account {
            account,
            working_dir,
            ..
        } => (
            account,
            working_dir
                .or_else(dirs::home_dir)
                .ok_or("workingDirInvalid")?,
            None,
        ),
    };
    if !valid_working_dir(&working_dir) {
        return Err("workingDirInvalid".into());
    }
    credential(&data_dir, &account)?;
    Ok(LaunchRequest {
        data_dir,
        account,
        working_dir,
        instance_id,
        language: super::config::get_user_config().language,
        custom_command: custom_command.filter(|value| !value.trim().is_empty()),
    })
}

impl LaunchRequest {
    pub(crate) fn remember_working_dir(&self) -> Result<(), String> {
        if let Some(id) = self.instance_id {
            update_instances(&self.data_dir, |instances| {
                let instance = instances
                    .iter_mut()
                    .find(|instance| instance.id == id && instance.account == self.account)
                    .ok_or("instanceMissing")?;
                instance.working_dir.clone_from(&self.working_dir);
                Ok(())
            })?;
            return Ok(());
        }
        let directory = account_profile_dir(&self.data_dir, &self.account);
        fs::create_dir_all(&directory).map_err(|_| "storageError")?;
        let content = serde_json::to_string(&self.working_dir).map_err(|_| "storageError")?;
        atomic_write::write_secret_string_atomic_if_changed(
            &directory.join(WORKING_DIR_FILE),
            &content,
        )
        .map_err(|_| "storageError")?;
        Ok(())
    }

    pub(crate) fn encode(&self) -> Result<String, String> {
        serde_json::to_vec(self)
            .map(|data| URL_SAFE_NO_PAD.encode(data))
            .map_err(|_| "launchFailed".into())
    }

    fn decode(encoded: &str) -> Result<Self, String> {
        let decoded = URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| "launchFailed")?;
        serde_json::from_slice(&decoded).map_err(|_| "launchFailed".into())
    }

    fn cli_command(&self, binary: &Path) -> Result<Command, String> {
        let token = credential(&self.data_dir, &self.account)?;
        if !valid_working_dir(&self.working_dir) {
            return Err("workingDirInvalid".into());
        }
        let profile = launch_profile_dir(self);
        fs::create_dir_all(&profile).map_err(|_| "storageError")?;

        let mut command = if let Some(script) = &self.custom_command {
            #[cfg(unix)]
            let mut shell = Command::new("/bin/sh");
            #[cfg(unix)]
            shell.args(["-c", script.as_str()]);
            #[cfg(target_os = "windows")]
            let mut shell = Command::new("powershell");
            #[cfg(target_os = "windows")]
            {
                let encoded = base64::engine::general_purpose::STANDARD.encode(
                    script
                        .encode_utf16()
                        .flat_map(u16::to_le_bytes)
                        .collect::<Vec<_>>(),
                );
                shell.args(["-NoProfile", "-EncodedCommand", &encoded]);
            }
            shell
        } else {
            #[cfg(not(target_os = "windows"))]
            let cli = Command::new(binary);
            #[cfg(target_os = "windows")]
            let cli = windows_cli_command(binary)?;
            cli
        };
        if let Some(directory) = binary
            .parent()
            .filter(|dir| self.custom_command.is_some() || cfg!(unix) && dir.join("node").is_file())
        {
            // npm scripts can need Node installed next to the CLI.
            let path = std::env::var_os("PATH").unwrap_or_default();
            let dirs = std::iter::once(directory.to_path_buf()).chain(std::env::split_paths(&path));
            if let Ok(path) = std::env::join_paths(dirs) {
                command.env("PATH", path);
            }
        }
        command
            .current_dir(&self.working_dir)
            .env_remove("CODEBUDDY_AUTH_SESSION_FILE_DESCRIPTOR")
            .env_remove("CODEBUDDY_AUTH_TOKEN_FILE_DESCRIPTOR")
            .env_remove("CODEBUDDY_AUTH_SNAPSHOT_PATH")
            .env_remove("CODEBUDDY_API_KEY_FILE_DESCRIPTOR")
            .env_remove("CODEBUDDY_COPILOT_INTERNET_ENVIRONMENT")
            .env_remove("CODEBUDDY_API_KEY")
            .env_remove("CODEBUDDY_BASE_URL")
            .env_remove("CODEBUDDY_CUSTOM_HEADERS")
            .env("CODEBUDDY_CONFIG_DIR", profile)
            .env("CODEBUDDY_AUTH_TOKEN", token);
        if self.account.region == Region::China {
            command.env("CODEBUDDY_INTERNET_ENVIRONMENT", "internal");
        } else {
            // The official CLI defaults to international and resolves enterprise regions from the token.
            command.env_remove("CODEBUDDY_INTERNET_ENVIRONMENT");
        }
        Ok(command)
    }
}

#[cfg(target_os = "windows")]
fn windows_cli_command(binary: &Path) -> Result<Command, String> {
    use std::os::windows::process::CommandExt;

    if binary
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("cmd"))
    {
        let interpreter = std::env::var_os("ComSpec").ok_or("launchFailed")?;
        let mut command = Command::new(interpreter);
        command.args(["/d", "/v:off", "/s", "/c"]);
        // Pass the path through an environment variable. Expansion is one pass,
        // so metacharacters in the path cannot become cmd.exe syntax.
        command.env("COCKPIT_CODEBUDDY_CLI_BINARY", binary);
        command.raw_arg(r#"""%COCKPIT_CODEBUDDY_CLI_BINARY%"""#);
        return Ok(command);
    }
    Ok(Command::new(binary))
}

pub(crate) fn find_cli() -> Option<PathBuf> {
    let configured = super::config::get_user_config().codebuddy_cli_path;
    if configured.is_empty() {
        return detect_cli();
    }
    let path = PathBuf::from(configured);
    crate::commands::cli_launch_path::executable_file(&path).then_some(path)
}

pub(crate) fn detect_cli() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let names = [
        "codebuddy.exe",
        "codebuddy.cmd",
        "codebuddy-code.cmd",
        "cbc.exe",
        "cbc.cmd",
    ];
    #[cfg(not(target_os = "windows"))]
    let names = ["codebuddy", "codebuddy-code", "cbc"];

    let mut dirs = super::codex_wakeup::collect_runtime_search_dirs();
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".local").join("bin"));
        dirs.push(home.join(".codebuddy").join("bin"));
    }
    #[cfg(target_os = "windows")]
    if let Some(local_app_data) = dirs::data_local_dir() {
        dirs.push(local_app_data.join("codebuddy").join("bin"));
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    dirs.push(PathBuf::from("/usr/local/bin"));

    dirs.into_iter()
        .filter(|dir| dir.is_absolute())
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|candidate| {
            let Ok(meta) = candidate.metadata() else {
                return false;
            };
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                meta.is_file() && meta.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            meta.is_file()
        })
}

pub(crate) fn run_from_terminal(encoded: &str) -> Result<i32, String> {
    #[cfg(target_os = "windows")]
    unsafe {
        use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
    let request = LaunchRequest::decode(encoded)?;
    let command = find_cli()
        .ok_or_else(|| String::from("cliMissing"))
        .and_then(|binary| request.cli_command(&binary));
    let mut command = match command {
        Ok(command) => command,
        Err(error) => return Ok(report_terminal_error(&request.language, &error)),
    };
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let _ = command.exec();
        Ok(report_terminal_error(&request.language, "launchFailed"))
    }
    #[cfg(target_os = "windows")]
    {
        use std::process::Stdio;
        let input = fs::File::open("CONIN$").map_err(|_| "launchFailed")?;
        let output = || {
            fs::OpenOptions::new()
                .write(true)
                .open("CONOUT$")
                .map_err(|_| "launchFailed")
        };
        command
            .stdin(Stdio::from(input))
            .stdout(Stdio::from(output()?))
            .stderr(Stdio::from(output()?));
        // This CLI is explicitly interactive and must inherit the attached console.
        Ok(match command.status() {
            Ok(status) => status.code().unwrap_or(1),
            Err(_) => report_terminal_error(&request.language, "launchFailed"),
        })
    }
}

pub(crate) fn report_terminal_error(lang: &str, code: &str) -> i32 {
    use std::io::Write;
    let message = super::i18n::translate(lang, &format!("codebuddyCli.errors.{code}"), &[]);
    #[cfg(target_os = "windows")]
    if let Ok(mut output) = fs::OpenOptions::new().write(true).open("CONOUT$") {
        let _ = writeln!(output, "{message}");
        return 1;
    }
    let _ = writeln!(io::stderr(), "{message}");
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_paths_isolate_regions_accounts_and_instances() {
        let mut request = LaunchRequest {
            data_dir: PathBuf::from("data"),
            account: AccountRef {
                region: Region::China,
                account_id: "shared".into(),
            },
            working_dir: PathBuf::from("working"),
            instance_id: None,
            language: "en".into(),
            custom_command: None,
        };
        let china = launch_profile_dir(&request);
        request.account.region = Region::International;
        assert_ne!(china, launch_profile_dir(&request));
        request.account.account_id = "other".into();
        assert_ne!(china, launch_profile_dir(&request));
        request.instance_id = Some(uuid::Uuid::new_v4());
        let first = launch_profile_dir(&request);
        request.instance_id = Some(uuid::Uuid::new_v4());
        assert_ne!(first, launch_profile_dir(&request));

        let decoded = LaunchRequest::decode(&request.encode().unwrap()).unwrap();
        assert_eq!(request.account, decoded.account);
        assert_eq!(request.instance_id, decoded.instance_id);
        assert_eq!(request.data_dir, decoded.data_dir);
        assert_eq!(request.working_dir, decoded.working_dir);
        assert!(decoded.custom_command.is_none());
    }

    #[test]
    fn launch_reads_only_the_selected_region_and_passes_credentials_in_environment() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path();
        let account_id = "same-id";
        for (region, token) in [
            (Region::China, "test-china"),
            (Region::International, "test-intl"),
        ] {
            let account_dir = data_dir.join(region.account_dir());
            fs::create_dir(&account_dir).unwrap();
            let record = serde_json::json!({
                "id": account_id, "email": "test@example.com", "created_at": 0,
                "last_used": 0, "access_token": token, "status": "other"
            });
            fs::write(
                account_dir.join(format!("{account_id}.json")),
                record.to_string(),
            )
            .unwrap();

            let request = LaunchRequest {
                data_dir: data_dir.to_path_buf(),
                account: AccountRef {
                    region,
                    account_id: account_id.into(),
                },
                working_dir: data_dir.to_path_buf(),
                instance_id: None,
                language: "en".into(),
                custom_command: None,
            };
            let command = request.cli_command(Path::new("fake-codebuddy")).unwrap();
            let env = command
                .get_envs()
                .collect::<std::collections::HashMap<_, _>>();
            assert_eq!(
                env[std::ffi::OsStr::new("CODEBUDDY_AUTH_TOKEN")],
                Some(std::ffi::OsStr::new(token))
            );
            assert_eq!(command.get_args().count(), 0);
            assert!(!String::from_utf8_lossy(
                &URL_SAFE_NO_PAD.decode(request.encode().unwrap()).unwrap()
            )
            .contains(token));

            #[cfg(unix)]
            {
                let mut request = request;
                request.custom_command = Some("test -n \"$CODEBUDDY_AUTH_TOKEN\" && pwd".into());
                let request = LaunchRequest::decode(&request.encode().unwrap()).unwrap();
                let output = request
                    .cli_command(Path::new("/bin/false"))
                    .unwrap()
                    .output()
                    .unwrap();
                assert!(output.status.success());
                let actual = String::from_utf8(output.stdout).unwrap();
                assert_eq!(
                    Path::new(actual.trim_end()).canonicalize().unwrap(),
                    data_dir.canonicalize().unwrap()
                );
            }
        }
        assert_eq!(
            credential(
                data_dir,
                &AccountRef {
                    region: Region::China,
                    account_id: "../same-id".into(),
                }
            )
            .unwrap_err(),
            "accountMissing"
        );
    }

    #[test]
    fn account_working_directories_reuse_existing_profiles_and_are_isolated() {
        let root = tempfile::tempdir().unwrap();
        let data_dir = root.path();
        let mut request = LaunchRequest {
            data_dir: data_dir.to_path_buf(),
            account: AccountRef {
                region: Region::China,
                account_id: "first".into(),
            },
            working_dir: data_dir.join("project a"),
            instance_id: None,
            language: "en".into(),
            custom_command: None,
        };
        let china_first = request.account.clone();
        let project_a = request.working_dir.clone();
        let profile = data_dir
            .join("codebuddy_cli_profiles/accounts/codebuddy_cn_accounts")
            .join(format!("{:x}", Sha256::digest(b"first")));
        fs::create_dir_all(&profile).unwrap();
        fs::write(
            profile.join(WORKING_DIR_FILE),
            serde_json::to_string(&project_a).unwrap(),
        )
        .unwrap();
        assert_eq!(
            read_working_dir(data_dir, &china_first).unwrap(),
            Some(project_a.clone())
        );

        request.account.account_id = "second".into();
        request.working_dir = data_dir.join("project b");
        fs::create_dir(&request.working_dir).unwrap();
        assert_eq!(read_working_dir(data_dir, &request.account).unwrap(), None);
        request.remember_working_dir().unwrap();
        assert_eq!(
            read_working_dir(data_dir, &request.account).unwrap(),
            Some(request.working_dir)
        );
        assert_eq!(
            read_working_dir(data_dir, &china_first).unwrap(),
            Some(project_a)
        );
        let same_id_international = AccountRef {
            region: Region::International,
            ..china_first
        };
        assert_eq!(
            read_working_dir(data_dir, &same_id_international).unwrap(),
            None
        );
    }

    #[test]
    fn update_preserves_existing_data_after_failed_edit() {
        let dir = tempfile::tempdir().unwrap();
        update_instances(dir.path(), |instances| {
            instances.push(Instance {
                id: uuid::Uuid::new_v4(),
                name: "First".into(),
                account: AccountRef {
                    region: Region::China,
                    account_id: "a".into(),
                },
                working_dir: PathBuf::from("project"),
            });
            Ok(())
        })
        .unwrap();
        let path = dir.path().join(INSTANCES_FILE);
        let original = fs::read_to_string(&path).unwrap();
        assert_eq!(
            update_instances(dir.path(), |instances| {
                instances.clear();
                Err("instanceMissing")
            })
            .unwrap_err(),
            "instanceMissing"
        );
        assert_eq!(original, fs::read_to_string(&path).unwrap());
    }
}
