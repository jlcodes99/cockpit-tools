use std::path::PathBuf;

use crate::models::InstanceProfile;
use crate::modules;

pub(super) fn idle_codex_profile_dirs_for_app_exit(
    default_dir: PathBuf,
    default_last_pid: Option<u32>,
    instances: Vec<InstanceProfile>,
    mut is_running: impl FnMut(Option<u32>, Option<&str>) -> bool,
) -> Vec<PathBuf> {
    let mut profiles = Vec::new();
    if !is_running(default_last_pid, None) {
        profiles.push(default_dir);
    }
    for instance in instances {
        // Use live process state, not saved routing.enabled: a running profile
        // may have a disabled configuration saved for its next launch.
        if !is_running(instance.last_pid, Some(&instance.user_data_dir)) {
            profiles.push(PathBuf::from(instance.user_data_dir));
        }
    }
    profiles
}

fn configured_idle_codex_profile_dirs() -> Result<Vec<PathBuf>, String> {
    let settings = modules::codex_instance::load_default_settings()?;
    Ok(idle_codex_profile_dirs_for_app_exit(
        modules::codex_instance::get_default_codex_home()?,
        settings.last_pid,
        modules::codex_instance::load_instance_store()?.instances,
        |last_pid, profile| modules::process::resolve_codex_pid(last_pid, profile).is_some(),
    ))
}

pub fn restore_mixed_model_profiles_for_app_exit() {
    let profiles = match configured_idle_codex_profile_dirs() {
        Ok(profiles) => profiles,
        Err(error) => {
            modules::logger::log_warn(&format!(
                "[MixedModelRouting] 应用退出前读取实例失败: {}",
                error
            ));
            return;
        }
    };
    for profile_dir in profiles {
        match modules::codex_local_access::restore_mixed_model_gateway_profile(&profile_dir) {
            Ok(true) => {
                if let Err(error) =
                    modules::codex_local_access::cleanup_provider_gateway_profile_model_overrides(
                        &profile_dir,
                    )
                {
                    modules::logger::log_warn(&format!(
                        "[MixedModelRouting] 应用退出恢复模型目录失败: profile={} error={}",
                        profile_dir.display(),
                        error
                    ));
                }
            }
            Ok(false) => {}
            Err(error) => modules::logger::log_warn(&format!(
                "[MixedModelRouting] 应用退出恢复官方配置失败: profile={} error={}",
                profile_dir.display(),
                error
            )),
        }
    }
}
