use crate::models::Account;
use crate::modules::{
    self, antigravity_credential, antigravity_legacy_instance, instance, process,
};

#[cfg(target_os = "macos")]
#[path = "antigravity_shared_switch_hub.rs"]
mod hub;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SwitchTarget {
    Desktop,
    Ide,
    Cli,
}

pub(crate) fn switch(
    account: &Account,
    target: SwitchTarget,
    launch: bool,
    ide_is_shared: bool,
) -> Result<(Account, Option<String>), String> {
    let desktop_is_shared = target == SwitchTarget::Desktop
        || matches!(
            antigravity_legacy_instance::resolve_auth_mode(),
            antigravity_legacy_instance::AntigravityDesktopAuthMode::SystemCredential
        );
    let desktop = desktop_is_shared
        .then(antigravity_legacy_instance::load_default_settings)
        .transpose()?;
    let ide = ide_is_shared
        .then(instance::load_default_settings)
        .transpose()?;
    let (open_desktop, open_ide) = clients_to_open(target, launch);
    let desktop_running = open_desktop
        && desktop.as_ref().is_some_and(|settings| {
            process::resolve_antigravity_legacy_pid(settings.last_pid, None).is_some()
        });
    let ide_running = open_ide
        && ide.as_ref().is_some_and(|settings| {
            process::resolve_antigravity_pid(settings.last_pid, None).is_some()
        });
    if open_desktop {
        process::ensure_antigravity_legacy_launch_path_configured()?;
    }
    if open_ide {
        process::ensure_antigravity_launch_path_configured()?;
    }

    #[cfg(target_os = "macos")]
    let hubs = if open_desktop || open_ide {
        hub::snapshot(target)
    } else {
        Vec::new()
    };

    let mut desktop_closed = false;
    let mut ide_closed = false;
    #[cfg(target_os = "macos")]
    let mut stopped_hubs = Vec::new();
    let written = (|| {
        if desktop_running {
            let dir = modules::antigravity_paths::legacy_default_user_data_dir()?;
            let dir = dir.to_string_lossy().into_owned();
            process::close_antigravity_legacy_instances(
                std::slice::from_ref(&dir),
                &dir,
                20,
                process::AntigravityCloseMode::Restart,
            )?;
            desktop_closed = true;
            let _ = antigravity_legacy_instance::update_default_pid(None);
        }
        if ide_running {
            let dir = instance::get_default_user_data_dir()?;
            process::close_antigravity_instances(
                &[dir.to_string_lossy().into_owned()],
                20,
                process::AntigravityCloseMode::Restart,
            )?;
            ide_closed = true;
            let _ = instance::update_default_pid(None);
        }
        #[cfg(target_os = "macos")]
        for candidate in hubs {
            if hub::stop(&candidate)? {
                stopped_hubs.push(candidate);
            }
        }
        match target {
            SwitchTarget::Cli => {
                antigravity_credential::write_antigravity_cli_switch_credentials(account)
            }
            _ => antigravity_credential::write_antigravity_desktop_switch_credentials(account),
        }
    })();
    #[cfg(target_os = "macos")]
    let hub_error = hub::restart(stopped_hubs).err();
    #[cfg(not(target_os = "macos"))]
    let hub_error: Option<String> = None;
    let switched =
        written.and_then(|()| commit_shared_switch(&account.id, desktop_is_shared, ide_is_shared));

    let desktop_error = if desktop_closed || (open_desktop && switched.is_ok()) {
        let args = desktop
            .as_ref()
            .map(|settings| process::parse_extra_args(&settings.extra_args))
            .unwrap_or_default();
        process::start_antigravity_legacy_with_args_proxy(
            "",
            &args,
            account.egress_proxy_url.as_deref(),
        )
            .and_then(|pid| antigravity_legacy_instance::update_default_pid(Some(pid)).map(|_| ()))
            .err()
    } else {
        None
    };
    let ide_error = if ide_closed || (open_ide && switched.is_ok()) {
        let args = ide
            .as_ref()
            .map(|settings| process::parse_extra_args(&settings.extra_args))
            .unwrap_or_default();
        process::start_antigravity_with_args_proxy(
            "",
            &args,
            account.egress_proxy_url.as_deref(),
        )
            .and_then(|pid| instance::update_default_pid(Some(pid)).map(|_| ()))
            .err()
    } else {
        None
    };
    let errors: Vec<_> = [hub_error, desktop_error, ide_error]
        .into_iter()
        .flatten()
        .collect();
    let launch_error = (!errors.is_empty()).then(|| errors.join("\n"));
    if let (Err(_), Some(error)) = (&switched, &launch_error) {
        modules::logger::log_warn(&format!("[Antigravity] 恢复客户端失败: {error}"));
    }
    Ok((switched?, launch_error))
}

fn commit_shared_switch(
    account_id: &str,
    desktop_is_shared: bool,
    ide_is_shared: bool,
) -> Result<Account, String> {
    let result = (|| {
        let account = modules::account::commit_switched_account(account_id)?;
        let binding = Some(Some(account.id.clone()));
        let desktop = desktop_is_shared
            .then(|| {
                antigravity_legacy_instance::update_default_settings(
                    binding.clone(),
                    None,
                    Some(false),
                )
            })
            .transpose();
        let ide = ide_is_shared
            .then(|| instance::update_default_settings(binding, None, Some(false)))
            .transpose();
        desktop?;
        ide?;
        Ok::<_, String>(account)
    })();
    result.map_err(|error| format!("ANTIGRAVITY_SWITCH_STATE_SAVE_FAILED: {error}"))
}

fn clients_to_open(target: SwitchTarget, launch: bool) -> (bool, bool) {
    (
        launch && target == SwitchTarget::Desktop,
        launch && target == SwitchTarget::Ide,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_switch_opens_only_the_selected_client() {
        assert_eq!(clients_to_open(SwitchTarget::Ide, true), (false, true));
        assert_eq!(clients_to_open(SwitchTarget::Desktop, true), (true, false));
        assert_eq!(clients_to_open(SwitchTarget::Ide, false), (false, false));
        assert_eq!(
            clients_to_open(SwitchTarget::Desktop, false),
            (false, false)
        );
        assert_eq!(clients_to_open(SwitchTarget::Cli, true), (false, false));
    }
}
