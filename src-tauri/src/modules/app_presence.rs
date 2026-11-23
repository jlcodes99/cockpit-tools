use tauri::{ActivationPolicy, AppHandle, Runtime};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresencePolicy {
    Regular,
    Accessory,
}

pub fn desired_policy(hide_dock_icon: bool) -> PresencePolicy {
    if hide_dock_icon {
        PresencePolicy::Accessory
    } else {
        PresencePolicy::Regular
    }
}

pub fn apply_policy<R: Runtime>(app: &AppHandle<R>, policy: PresencePolicy) -> tauri::Result<()> {
    let tauri_policy = match policy {
        PresencePolicy::Regular => ActivationPolicy::Regular,
        PresencePolicy::Accessory => ActivationPolicy::Accessory,
    };

    app.set_activation_policy(tauri_policy)?;

    let label = match policy {
        PresencePolicy::Regular => "visible",
        PresencePolicy::Accessory => "hidden",
    };
    crate::modules::logger::log_info(&format!(
        "[Window] 已应用 macOS Dock 图标策略: {}",
        label
    ));
    Ok(())
}

pub fn apply_configured_policy<R: Runtime>(app: &AppHandle<R>) {
    let config = crate::modules::config::get_user_config();
    let policy = desired_policy(config.hide_dock_icon);
    if let Err(err) = apply_policy(app, policy) {
        crate::modules::logger::log_warn(&format!("[Window] 应用 macOS Dock 图标策略失败: {}", err));
    }
}

#[cfg(test)]
mod tests {
    use super::{desired_policy, PresencePolicy};
    use crate::modules::config::UserConfig;

    #[test]
    fn dock_icon_preference_alone_decides_presence_policy() {
        assert_eq!(desired_policy(true), PresencePolicy::Accessory);
        assert_eq!(desired_policy(false), PresencePolicy::Regular);
    }

    #[test]
    fn tray_visibility_does_not_affect_presence_policy() {
        let mut config = UserConfig::default();
        config.hide_dock_icon = true;
        config.show_tray_icon = false;

        assert_eq!(desired_policy(config.hide_dock_icon), PresencePolicy::Accessory);

        config.show_tray_icon = true;
        assert_eq!(desired_policy(config.hide_dock_icon), PresencePolicy::Accessory);
    }
}
