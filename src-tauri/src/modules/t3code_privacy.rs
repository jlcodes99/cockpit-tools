//! Desktop launch overrides and the native T3 Connect activity preference.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::{process::Stdio, time::Duration};

use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use super::{atomic_write, data_paths, t3code_account, t3code_instance};

const TELEMETRY: &str = "T3CODE_TELEMETRY_ENABLED";
const DO_NOT_TRACK: &str = "DO_NOT_TRACK";
const OTEL: &str = "T3CODE_OTEL_SDK_DISABLED";
const STANDARD_OTEL: &str = "OTEL_SDK_DISABLED";
const LAUNCH_SETTINGS: &str = "COCKPIT_T3CODE_PRIVACY";
const ENV_KEYS: &[&str] = &[TELEMETRY, DO_NOT_TRACK, OTEL, STANDARD_OTEL];
static SETTINGS_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
struct LaunchSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    analytics: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tracing: Option<bool>,
    #[serde(rename = "doNotTrack", skip_serializing_if = "Option::is_none")]
    do_not_track: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredLaunchSettings {
    #[serde(flatten)]
    current: LaunchSettings,
    #[serde(default)]
    analytics_disabled: bool,
    #[serde(default)]
    tracing_disabled: bool,
}

impl LaunchSettings {
    fn marker(self) -> String {
        [self.analytics, self.tracing, self.do_not_track]
            .into_iter()
            .map(|value| match value {
                Some(true) => '1',
                Some(false) => '0',
                None => '-',
            })
            .collect()
    }

    fn from_marker(marker: &str) -> Self {
        // Previous builds only wrote overrides that turned telemetry off.
        let flags = match marker {
            "analytics" => "0--",
            "tracing" => "-0-",
            "both" => "00-",
            flags => flags,
        };
        let bit = |flag| match flag {
            b'1' => Some(true),
            b'0' => Some(false),
            _ => None,
        };
        match flags.as_bytes() {
            [analytics, tracing, do_not_track] => Self {
                analytics: bit(*analytics),
                tracing: bit(*tracing),
                do_not_track: bit(*do_not_track),
            },
            _ => Self::default(),
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrivacyPatch {
    do_not_track: Option<bool>,
    analytics: Option<bool>,
    tracing: Option<bool>,
    agent_activity: Option<bool>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OptionState {
    enabled: bool,
    selected: bool,
    blocked_by: Option<&'static str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivacyState {
    do_not_track: bool,
    analytics: OptionState,
    tracing: OptionState,
    agent_activity: bool,
    restart_required: bool,
    privacy_mode_saved: bool,
    all_enabled_saved: bool,
}

type Environment = std::collections::HashMap<String, String>;

fn read_optional(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

fn settings_path() -> Result<PathBuf, String> {
    data_paths::resolve_data_dir().map(|dir| dir.join("t3code-privacy.json"))
}

fn read_settings(path: &Path) -> Result<LaunchSettings, String> {
    let Some(text) = read_optional(path)? else {
        return Ok(LaunchSettings::default());
    };
    let stored: StoredLaunchSettings =
        serde_json::from_str(&text).map_err(|error| error.to_string())?;
    Ok(LaunchSettings {
        analytics: stored
            .current
            .analytics
            .or(stored.analytics_disabled.then_some(false)),
        tracing: stored
            .current
            .tracing
            .or(stored.tracing_disabled.then_some(false)),
        do_not_track: stored.current.do_not_track,
    })
}

fn activity_path() -> Result<PathBuf, String> {
    Ok(t3code_account::default_state_dir()?
        .join("secrets")
        .join("cloud-publish-agent-activity.bin"))
}

fn enabled_activity(path: &Path) -> Result<bool, String> {
    Ok(read_optional(path)?.as_deref() == Some("true"))
}

fn write_activity(path: &Path, enabled: bool) -> Result<(), String> {
    if enabled_activity(path)? == enabled {
        return Ok(());
    }
    let mut directory = std::fs::DirBuilder::new();
    directory.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        directory.mode(0o700);
    }
    directory
        .create(
            path.parent()
                .expect("T3 Code activity preference has a parent"),
        )
        .map_err(|error| error.to_string())?;
    // T3 Code reads this preference from disk for each publication.
    atomic_write::write_secret_string_atomic(path, if enabled { "true" } else { "false" })
}

fn inherited_environment() -> Environment {
    ENV_KEYS
        .iter()
        .filter_map(|&key| std::env::var(key).ok().map(|value| (key.into(), value)))
        .collect()
}

fn apply_launch_settings(settings: LaunchSettings, environment: &mut Environment) {
    environment.remove(LAUNCH_SETTINGS);
    if let Some(enabled) = settings.do_not_track {
        // T3 Code fills a missing value from the login shell. Pass a
        // nonempty value even for an explicit opt-out of Do Not Track.
        environment.insert(DO_NOT_TRACK.into(), if enabled { "1" } else { "0" }.into());
    }
    if let Some(enabled) = settings.analytics {
        environment.insert(TELEMETRY.into(), enabled.to_string());
    }
    if settings.analytics == Some(false) || settings.do_not_track == Some(true) {
        environment.insert(OTEL.into(), "true".into());
    } else if let Some(enabled) = settings.tracing {
        let sdk_disabled = !enabled
            || yes(&environment_flag(environment, DO_NOT_TRACK))
            || no(&environment_flag(environment, TELEMETRY));
        environment.insert(OTEL.into(), sdk_disabled.to_string());
    }
    if settings != LaunchSettings::default() {
        environment.insert(LAUNCH_SETTINGS.into(), settings.marker());
    }
}

pub(super) fn launch_environment() -> Result<Environment, String> {
    let settings = read_settings(&settings_path()?)?;
    let mut environment = inherited_environment();
    apply_launch_settings(settings, &mut environment);
    Ok(environment)
}

fn running_environment() -> Result<Option<Environment>, String> {
    let settings = t3code_instance::load_default_settings()?;
    let Some(pid) = t3code_instance::resolve_pid(None, settings.last_pid) else {
        return Ok(None);
    };
    let pid = Pid::from_u32(pid);
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing().with_environ(UpdateKind::Always),
    );
    Ok(system.process(pid).map(|process| {
        process
            .environ()
            .iter()
            .filter_map(|entry| {
                let text = entry.to_str()?;
                let (key, value) = text.split_once('=')?;
                (ENV_KEYS.contains(&key) || key == LAUNCH_SETTINGS)
                    .then(|| (key.to_owned(), value.to_owned()))
            })
            .collect()
    }))
}

// These two values are also imported by T3 Code's own login-shell discovery.
#[cfg(unix)]
async fn read_login_shell_flags(environment: &mut Environment) {
    let Ok(shell) = std::env::var("SHELL") else {
        return;
    };
    if !Path::new(&shell).is_absolute() {
        return;
    }
    let output = tokio::time::timeout(
        Duration::from_secs(2),
        tokio::process::Command::new(shell)
            .args([
                "-ilc",
                r#"printf '\n__cockpit_telemetry=%s\n__cockpit_dnt=%s\n' "$T3CODE_TELEMETRY_ENABLED" "$DO_NOT_TRACK""#,
            ])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await;
    if let Ok(Ok(output)) = output {
        if output.status.success() {
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                let match_line = line
                    .strip_prefix("__cockpit_telemetry=")
                    .map(|value| (TELEMETRY, value))
                    .or_else(|| {
                        line.strip_prefix("__cockpit_dnt=")
                            .map(|value| (DO_NOT_TRACK, value))
                    });
                if let Some((key, value)) = match_line {
                    if !value.is_empty() && environment.get(key).is_none_or(String::is_empty) {
                        environment.insert(key.into(), value.into());
                    }
                }
            }
        }
    }
}

fn managed_policy_disables_telemetry() -> bool {
    #[cfg(target_os = "macos")]
    let path = "/Library/Application Support/T3Code/policy.json";
    #[cfg(target_os = "windows")]
    let path = r"C:\Program Files\T3Code\policy.json";
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let path = "/etc/t3code/policy.json";
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|value| {
            value
                .pointer("/limits/telemetry")
                .and_then(serde_json::Value::as_bool)
        })
        == Some(false)
}

fn yes(value: &str) -> bool {
    matches!(value, "true" | "yes" | "on" | "1" | "y")
}

fn no(value: &str) -> bool {
    matches!(value, "false" | "no" | "off" | "0" | "n")
}

fn environment_flag(environment: &Environment, key: &str) -> String {
    environment
        .get(key)
        .map(|value| value.trim().to_ascii_lowercase())
        .unwrap_or_default()
}

fn blocked_sources(
    environment: &Environment,
    managed_policy: bool,
) -> (Option<&'static str>, Option<&'static str>) {
    if managed_policy {
        return (Some("managedPolicy"), Some("managedPolicy"));
    }
    let flag = |key| environment_flag(environment, key);
    let analytics = if yes(&flag(DO_NOT_TRACK)) {
        Some(DO_NOT_TRACK)
    } else if no(&flag(TELEMETRY)) {
        Some(TELEMETRY)
    } else {
        None
    };
    let otel = flag(OTEL);
    let trace = analytics.or_else(|| {
        if yes(&otel) {
            Some(OTEL)
        } else if !no(&otel) && flag(STANDARD_OTEL) == "true" {
            Some(STANDARD_OTEL)
        } else {
            None
        }
    });
    (analytics, trace)
}

fn option_states(
    settings: LaunchSettings,
    mut environment: Environment,
    managed_policy: bool,
) -> (bool, OptionState, OptionState) {
    apply_launch_settings(settings, &mut environment);
    let do_not_track = yes(&environment_flag(&environment, DO_NOT_TRACK));
    let (analytics_source, tracing_source) = blocked_sources(&environment, managed_policy);
    let analytics = OptionState {
        enabled: analytics_source.is_none(),
        selected: settings.analytics.unwrap_or(analytics_source.is_none()),
        blocked_by: analytics_source
            .filter(|_| managed_policy || settings.analytics != Some(false)),
    };
    let tracing = OptionState {
        enabled: tracing_source.is_none(),
        selected: settings.tracing.unwrap_or(tracing_source.is_none()),
        blocked_by: tracing_source
            .filter(|_| managed_policy || (analytics.enabled && settings.tracing != Some(false))),
    };
    (do_not_track, analytics, tracing)
}

async fn state(settings: LaunchSettings, agent_activity: bool) -> Result<PrivacyState, String> {
    let (running, managed_policy) = tokio::task::spawn_blocking(|| {
        Ok::<_, String>((running_environment()?, managed_policy_disables_telemetry()))
    })
    .await
    .map_err(|error| error.to_string())??;

    let mut environment = inherited_environment();
    let applied = LaunchSettings::from_marker(
        running
            .as_ref()
            .and_then(|running| running.get(LAUNCH_SETTINGS))
            .map(String::as_str)
            .unwrap_or_default(),
    );
    if let Some(running) = &running {
        for (key, value) in running {
            // An override inserted by Cockpit should not appear as an external restriction.
            if (key == TELEMETRY && applied.analytics.is_some())
                || (key == DO_NOT_TRACK && applied.do_not_track.is_some())
                || (key == OTEL
                    && (applied.tracing.is_some()
                        || applied.analytics == Some(false)
                        || applied.do_not_track == Some(true)))
            {
                continue;
            }
            environment.insert(key.clone(), value.clone());
        }
    }
    #[cfg(unix)]
    if !environment.contains_key(TELEMETRY) || !environment.contains_key(DO_NOT_TRACK) {
        read_login_shell_flags(&mut environment).await;
    }

    let (do_not_track, analytics, tracing) = option_states(settings, environment, managed_policy);
    let restart_required = running.is_some() && settings != applied;
    Ok(PrivacyState {
        do_not_track,
        analytics,
        tracing,
        agent_activity,
        restart_required,
        privacy_mode_saved: settings.do_not_track == Some(true)
            && settings.analytics == Some(false)
            && settings.tracing == Some(false)
            && !agent_activity,
        all_enabled_saved: settings.do_not_track == Some(false)
            && settings.analytics == Some(true)
            && settings.tracing == Some(true)
            && agent_activity,
    })
}

pub async fn read() -> Result<PrivacyState, String> {
    let _reservation = SETTINGS_LOCK.lock().await;
    let (settings, activity) = tokio::task::spawn_blocking(|| {
        Ok::<_, String>((
            read_settings(&settings_path()?)?,
            enabled_activity(&activity_path()?)?,
        ))
    })
    .await
    .map_err(|error| error.to_string())??;
    state(settings, activity).await
}

fn update_settings(
    path: &Path,
    activity_path: &Path,
    patch: PrivacyPatch,
) -> Result<(LaunchSettings, bool), String> {
    let mut settings = read_settings(path)?;
    let previous = settings;
    if let Some(enabled) = patch.do_not_track {
        settings.do_not_track = Some(enabled);
    }
    if let Some(enabled) = patch.analytics {
        settings.analytics = Some(enabled);
    }
    if let Some(enabled) = patch.tracing {
        settings.tracing = Some(enabled);
    }
    let previous_activity = read_optional(activity_path)?;
    let old_activity = previous_activity.as_deref() == Some("true");
    let activity = patch.agent_activity.unwrap_or(old_activity);
    if activity != old_activity {
        write_activity(activity_path, activity)?;
    }
    if settings != previous {
        let content = serde_json::to_string_pretty(&settings).map_err(|error| error.to_string())?;
        if let Err(error) = atomic_write::write_string_atomic(path, &content) {
            if activity != old_activity {
                let expected = if activity { "true" } else { "false" };
                if let Err(rollback) = atomic_write::replace_secret_string_if_matches(
                    activity_path,
                    Some(expected),
                    previous_activity.as_deref(),
                ) {
                    return Err(format!("{error}\n{rollback}"));
                }
            }
            return Err(error);
        }
    }
    Ok((settings, activity))
}

pub async fn update(patch: PrivacyPatch) -> Result<PrivacyState, String> {
    let _reservation = SETTINGS_LOCK.lock().await;
    let (settings, activity) = tokio::task::spawn_blocking(move || {
        update_settings(&settings_path()?, &activity_path()?, patch)
    })
    .await
    .map_err(|error| error.to_string())??;
    state(settings, activity).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switching_off_analytics_also_disables_tracing_and_preserves_inherited_defaults() {
        let mut environment = Environment::from([(TELEMETRY.into(), "true".into())]);
        apply_launch_settings(
            LaunchSettings {
                analytics: Some(false),
                tracing: Some(true),
                ..Default::default()
            },
            &mut environment,
        );
        assert_eq!(
            environment.get(LAUNCH_SETTINGS).map(String::as_str),
            Some("01-")
        );
        assert_eq!(environment.get(OTEL).map(String::as_str), Some("true"));
        assert_eq!(
            blocked_sources(&environment, false),
            (Some(TELEMETRY), Some(TELEMETRY))
        );

        let mut environment = Environment::from([(DO_NOT_TRACK.into(), "1".into())]);
        apply_launch_settings(LaunchSettings::default(), &mut environment);
        assert_eq!(
            blocked_sources(&environment, false),
            (Some(DO_NOT_TRACK), Some(DO_NOT_TRACK))
        );
    }

    #[test]
    fn tracing_can_be_turned_off_while_analytics_remains_on() {
        let mut environment = Environment::new();
        apply_launch_settings(
            LaunchSettings {
                tracing: Some(false),
                ..Default::default()
            },
            &mut environment,
        );
        assert_eq!(blocked_sources(&environment, false), (None, Some(OTEL)));
        assert_eq!(
            blocked_sources(&Environment::new(), true),
            (Some("managedPolicy"), Some("managedPolicy"))
        );

        let environment = Environment::from([
            (OTEL.into(), "false".into()),
            (STANDARD_OTEL.into(), "true".into()),
        ]);
        assert_eq!(blocked_sources(&environment, false), (None, None));
    }

    #[test]
    fn explicit_opt_in_overrides_child_environment_but_respects_managed_policy() {
        let inherited = Environment::from([
            (DO_NOT_TRACK.into(), "1".into()),
            (TELEMETRY.into(), "false".into()),
            (OTEL.into(), "true".into()),
            (STANDARD_OTEL.into(), "true".into()),
        ]);
        let settings = LaunchSettings {
            analytics: Some(true),
            tracing: Some(true),
            do_not_track: Some(false),
        };
        let mut child = inherited.clone();
        apply_launch_settings(settings, &mut child);
        assert_eq!(child.get(DO_NOT_TRACK).map(String::as_str), Some("0"));
        assert_eq!(child.get(TELEMETRY).map(String::as_str), Some("true"));
        assert_eq!(child.get(OTEL).map(String::as_str), Some("false"));
        assert_eq!(LaunchSettings::from_marker(&settings.marker()), settings);

        let (do_not_track, analytics, tracing) = option_states(settings, inherited.clone(), false);
        assert!(!do_not_track);
        assert!(analytics.enabled && tracing.enabled);
        let (_, analytics, tracing) = option_states(settings, inherited, true);
        assert!(analytics.selected && tracing.selected);
        assert!(!analytics.enabled && !tracing.enabled);
        assert_eq!(analytics.blocked_by, Some("managedPolicy"));
        assert_eq!(tracing.blocked_by, Some("managedPolicy"));
    }

    #[test]
    fn legacy_defaults_do_not_become_an_explicit_opt_in() {
        let temp = tempfile::tempdir().unwrap();
        let privacy_path = temp.path().join("privacy.json");
        std::fs::write(
            &privacy_path,
            r#"{"analyticsDisabled":false,"tracingDisabled":true}"#,
        )
        .unwrap();
        let settings = read_settings(&privacy_path).unwrap();
        assert_eq!(settings.analytics, None);
        assert_eq!(settings.tracing, Some(false));
        assert_eq!(LaunchSettings::from_marker("tracing"), settings);
        let inherited = Environment::from([(DO_NOT_TRACK.into(), "1".into())]);
        let (do_not_track, analytics, tracing) = option_states(settings, inherited, false);
        assert!(do_not_track);
        assert!(!analytics.enabled && !tracing.enabled);
        assert_eq!(analytics.blocked_by, Some(DO_NOT_TRACK));
    }

    #[test]
    fn bulk_toggle_persists_all_choices_and_preserves_phone_pairing() {
        let temp = tempfile::tempdir().unwrap();
        let privacy_path = temp.path().join("privacy.json");
        let secrets = temp.path().join("secrets");
        std::fs::create_dir(&secrets).unwrap();
        let link = secrets.join("cloud-relay-url.bin");
        std::fs::write(&link, "stored link").unwrap();
        let activity_path = secrets.join("cloud-publish-agent-activity.bin");
        write_activity(&activity_path, true).unwrap();
        let (_, activity) = update_settings(
            &privacy_path,
            &activity_path,
            PrivacyPatch {
                do_not_track: Some(true),
                analytics: Some(false),
                tracing: Some(false),
                agent_activity: Some(false),
            },
        )
        .unwrap();

        assert!(!activity);
        assert_eq!(read_settings(&privacy_path).unwrap().marker(), "001");
        assert_eq!(std::fs::read_to_string(&activity_path).unwrap(), "false");
        let (_, activity) = update_settings(
            &privacy_path,
            &activity_path,
            PrivacyPatch {
                do_not_track: Some(false),
                analytics: Some(true),
                tracing: Some(true),
                agent_activity: Some(true),
            },
        )
        .unwrap();

        assert!(activity);
        assert_eq!(read_settings(&privacy_path).unwrap().marker(), "110");
        assert_eq!(std::fs::read_to_string(&activity_path).unwrap(), "true");
        assert_eq!(std::fs::read_to_string(&link).unwrap(), "stored link");

        let (_, activity) = update_settings(
            &privacy_path,
            &activity_path,
            PrivacyPatch {
                do_not_track: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(activity);
        let saved = read_settings(&privacy_path).unwrap();
        assert_eq!(saved.marker(), "111");
        let (do_not_track, analytics, tracing) = option_states(saved, Environment::new(), false);
        assert!(do_not_track && analytics.selected && tracing.selected);
        assert!(!analytics.enabled && !tracing.enabled);
    }
}
