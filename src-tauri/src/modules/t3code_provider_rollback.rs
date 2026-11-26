//! Snapshot the targeted provider's credential files and secure store before a live switch.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::super::{
    antigravity_profile_dir, extensions, provider_cache_path, DRIVER_ANTIGRAVITY, DRIVER_CLAUDE,
    DRIVER_CODEX,
};
use crate::modules::{
    atomic_write, claude_account, codex_account, minimax_code_account, minimax_code_instance,
};

enum FileContents {
    Absent,
    Regular(Vec<u8>, fs::Permissions),
    Symlink(PathBuf),
}

impl FileContents {
    fn capture(path: &Path) -> Result<Self, String> {
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.is_symlink() => fs::read_link(path)
                .map(Self::Symlink)
                .map_err(|error| error.to_string()),
            Ok(metadata) => fs::read(path)
                .map(|bytes| Self::Regular(bytes, metadata.permissions()))
                .map_err(|error| error.to_string()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(Self::Absent),
            Err(error) => Err(error.to_string()),
        }
    }

    fn restore(self, path: &Path) -> Result<(), String> {
        let remove_file = || match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        };
        match self {
            Self::Absent => remove_file(),
            Self::Regular(bytes, permissions) => {
                atomic_write::write_bytes_atomic(path, &bytes)?;
                fs::set_permissions(path, permissions).map_err(|error| error.to_string())
            }
            Self::Symlink(target) => {
                if fs::read_link(path).ok().as_deref() == Some(target.as_path()) {
                    return Ok(());
                }
                remove_file()?;
                #[cfg(unix)]
                let result = std::os::unix::fs::symlink(target, path);
                #[cfg(windows)]
                let result = std::os::windows::fs::symlink_file(target, path);
                result.map_err(|error| error.to_string())
            }
        }
    }
}

enum ProfileStore {
    Files,
    Codex(PathBuf, Option<Value>),
    Claude(PathBuf, claude_account::ClaudeCodeProfileAuthSnapshot),
}

pub(in crate::modules::t3code_provider_bindings) struct ProviderProfileSnapshot {
    files: Vec<(PathBuf, FileContents)>,
    store: ProfileStore,
}

impl ProviderProfileSnapshot {
    pub(super) fn capture(
        state_dir: &Path,
        instance: &str,
        driver: &str,
        oauth: bool,
    ) -> Result<Self, String> {
        Self::capture_for_account(state_dir, instance, driver, oauth, "")
    }

    pub(in crate::modules::t3code_provider_bindings) fn capture_for_account(
        state_dir: &Path,
        instance: &str,
        driver: &str,
        oauth: bool,
        account_id: &str,
    ) -> Result<Self, String> {
        let mut extra_files = Vec::new();
        let (root, names, store) = match driver {
            DRIVER_CODEX => {
                let root = state_dir
                    .join("providers/codex")
                    .join(instance)
                    .join("cockpit-home");
                let store = codex_account::snapshot_profile_keychain(&root)?
                    .map_or(ProfileStore::Files, |auth| {
                        ProfileStore::Codex(root.clone(), auth)
                    });
                (
                    root,
                    &[
                        "auth.json",
                        "config.toml",
                        ".cockpit_codex_auth.json",
                        ".cockpit_api_takeover.json",
                        "cockpit-model-catalog.json",
                        "cockpit-provider-model-catalog.json",
                        "cockpit-local-access-model-catalog.json",
                        ".cockpit-provider-model-backup.json",
                    ][..],
                    store,
                )
            }
            DRIVER_ANTIGRAVITY => (
                antigravity_profile_dir(state_dir, instance).join("antigravity-acp"),
                &["acp_token.json", "antigravity-oauth-token", "settings.json"][..],
                ProfileStore::Files,
            ),
            DRIVER_CLAUDE => {
                let root = state_dir.join("providers/claudeAgent").join(instance);
                let store = claude_account::ClaudeCodeProfileAuthSnapshot::capture(&root, oauth)?;
                (
                    root.clone(),
                    &[
                        ".credentials.json",
                        ".claude.json",
                        ".config.json",
                        "settings.json",
                    ][..],
                    ProfileStore::Claude(root, store),
                )
            }
            driver if extensions::is_extended_platform(driver) => {
                let root = extensions::cockpit_provider_dir(state_dir, driver, instance);
                let names = match driver {
                    extensions::PLATFORM_WORKBUDDY | extensions::PLATFORM_WORKBUDDY_AI => {
                        &["auth-secret.json", "launch.cjs"][..]
                    }
                    extensions::PLATFORM_CODEBUDDY | extensions::PLATFORM_CODEBUDDY_CN => &[
                        "secret.token",
                        "run-codebuddy-acp",
                        "auth-secret.json",
                        "launch-codebuddy.cjs",
                    ][..],
                    extensions::PLATFORM_GROK => &[
                        "home/auth.json",
                        "home/config.toml",
                        "secret.key",
                        "run-grok",
                        "run-grok.cmd",
                    ][..],
                    extensions::PLATFORM_MINIMAXCODE => {
                        let account = minimax_code_account::load_account(account_id)
                            .ok_or_else(|| super::message("switchTargetChanged"))?;
                        let data = root.join("data");
                        let scope = minimax_code_instance::resolve_minimax_code_auth_scope(
                            &data,
                            account.region.as_deref(),
                        );
                        let home =
                            minimax_code_instance::minimax_code_namespace_home(&data, &scope);
                        extra_files.extend([home.join("auth.json"), home.join("auth-state.json")]);
                        &[]
                    }
                    _ => return Err(super::message("switchTargetChanged")),
                };
                extra_files.push(root.join("binding-meta.json"));
                (root, names, ProfileStore::Files)
            }
            _ => return Err(super::message("switchTargetChanged")),
        };
        let paths = names
            .iter()
            .map(|name| root.join(name))
            .chain(extra_files)
            .chain(std::iter::once(provider_cache_path(state_dir, instance)));
        let files = paths
            .map(|path| FileContents::capture(&path).map(|file| (path, file)))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { files, store })
    }

    pub(in crate::modules::t3code_provider_bindings) fn restore(self) -> Result<(), String> {
        let mut errors = Vec::new();
        for (path, contents) in self.files {
            if let Err(error) = contents.restore(&path) {
                errors.push(error);
            }
        }
        let store = match self.store {
            ProfileStore::Files => Ok(()),
            ProfileStore::Codex(root, auth) => codex_account::restore_profile_keychain(&root, auth),
            ProfileStore::Claude(root, auth) => auth.restore(&root),
        };
        if let Err(error) = store {
            errors.push(error);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("\n"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restores_only_the_selected_profiles_old_credentials() {
        let directory = tempfile::tempdir().unwrap();
        let state = directory.path().join("userdata");
        let root = antigravity_profile_dir(&state, "work").join("antigravity-acp");
        let other = antigravity_profile_dir(&state, "other").join("antigravity-acp");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&other).unwrap();
        let old_token = root.join("acp_token.json");
        let new_file = root.join("antigravity-oauth-token");
        let other_token = other.join("acp_token.json");
        fs::write(&old_token, "previous token").unwrap();
        fs::write(&other_token, "other provider").unwrap();
        let snapshot =
            ProviderProfileSnapshot::capture(&state, "work", DRIVER_ANTIGRAVITY, true).unwrap();
        fs::write(&old_token, "new token").unwrap();
        fs::write(&new_file, "new file").unwrap();
        snapshot.restore().unwrap();
        assert_eq!(fs::read_to_string(old_token).unwrap(), "previous token");
        assert!(!new_file.exists());
        assert_eq!(fs::read_to_string(other_token).unwrap(), "other provider");
    }
}
