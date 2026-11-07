// Windows 默认实例：配置路径优先，只有确认客户端未创建时才自动发现其他入口。
#[cfg(any(test, target_os = "windows"))]
#[derive(Debug)]
enum CodexDefaultLaunchFailure {
    NotStarted(String),
    // Includes successful dispatch followed by a process-confirmation timeout.
    Dispatched(String),
}

#[cfg(any(test, target_os = "windows"))]
fn activate_codex_default_package_with_refresh(
    package: CodexRegisteredLaunch,
    mut activate: impl FnMut(&CodexRegisteredLaunch) -> Result<(), String>,
    refresh: impl FnOnce() -> Result<CodexRegisteredLaunch, String>,
) -> Result<CodexRegisteredLaunch, CodexDefaultLaunchFailure> {
    let mut may_have_started = false;
    let result = activate_codex_package_with_refresh(
        package,
        |package| {
            may_have_started = true;
            let result = activate(package);
            if result
                .as_ref()
                .is_err_and(|error| error.starts_with("CODEX_PACKAGE_REGISTRATION_CHANGED"))
            {
                // The native helper/activation API explicitly rejected the
                // registration before creating the client.
                may_have_started = false;
            }
            result
        },
        refresh,
    );
    result.map_err(|error| {
        if may_have_started {
            CodexDefaultLaunchFailure::Dispatched(error)
        } else {
            CodexDefaultLaunchFailure::NotStarted(error)
        }
    })
}

/// Discovery is lazy and runs once. Never try another entry after dispatch:
/// an absent receipt/PID is not proof that the client was not created.
#[cfg(any(test, target_os = "windows"))]
fn launch_codex_default_with_path_fallback(
    configured: Option<PathBuf>,
    discover: impl FnOnce() -> Vec<PathBuf>,
    mut attempt: impl FnMut(&Path) -> Result<(u32, PathBuf), CodexDefaultLaunchFailure>,
) -> Result<(u32, PathBuf), String> {
    let mut seen = HashSet::new();
    let mut errors = Vec::new();
    let mut missing_only = true;
    let mut try_path = |path: PathBuf| {
        if !seen.insert(normalized_windows_path_text(&path)) {
            return None;
        }
        match attempt(&path) {
            Ok(launched) => Some(Ok(launched)),
            Err(CodexDefaultLaunchFailure::Dispatched(error)) => Some(Err(error)),
            Err(CodexDefaultLaunchFailure::NotStarted(error)) => {
                missing_only &= error == app_path_missing_error("codex");
                crate::modules::logger::log_warn(&format!(
                    "[Codex Start] entry failed before client creation: path={} error={error}",
                    path.display()
                ));
                errors.push(format!("{}: {error}", path.display()));
                None
            }
        }
    };
    if let Some(path) = configured {
        if let Some(result) = try_path(path) {
            return result;
        }
    }
    for path in discover() {
        if let Some(result) = try_path(path) {
            return result;
        }
    }
    if errors.is_empty() || missing_only {
        Err(app_path_missing_error("codex"))
    } else {
        Err(errors.join("\n"))
    }
}

#[cfg(target_os = "windows")]
fn start_windows_codex_default(
    extra_args: &[String],
    fast_after_close: bool,
    launch_proxy: &CodexLaunchProxy,
    prepared_home: Option<&Path>,
) -> Result<u32, String> {
    let configured = config::get_user_config().codex_app_path;
    let initial = normalize_custom_path(Some(&configured)).map(PathBuf::from);
    let default_home_path = resolve_codex_default_launch_home(prepared_home)?;
    let standard_home = dirs::home_dir().map(|home| home.join(".codex"));
    let codex_home_override =
        codex_default_home_env_override(&default_home_path, standard_home.as_deref());
    let default_home = default_home_path.to_string_lossy();
    let default_app_dirs = get_default_codex_windows_app_user_data_dirs(&default_home);
    crate::modules::logger::log_info(&format!(
        "[Codex Start] default data directory={}, explicit_home={}",
        default_home,
        codex_home_override.is_some()
    ));
    let (pid, launched_path) = launch_codex_default_with_path_fallback(
        initial,
        || {
            // Same registered-package discovery as path resolution. Do not use
            // StartApps aliases or guess an Application Id such as '!App'.
            codex_store_packages()
                .into_iter()
                .filter_map(|package| codex_store_gui_exe_in(&package.install_location))
                .filter(|path| is_usable_codex_windows_gui_path(path))
                .collect()
        },
        |path| {
            try_windows_codex_default_path(
                path,
                codex_home_override.as_deref(),
                &default_app_dirs,
                extra_args,
                fast_after_close,
                launch_proxy,
            )
        },
    )?;
    // Preserve concurrent settings changes; only save a confirmed launch target.
    if configured != launched_path.to_string_lossy() {
        update_app_path_in_config("codex", &launched_path, &configured);
    }
    Ok(pid)
}

#[cfg(target_os = "windows")]
fn try_windows_codex_default_path(
    selected: &Path,
    codex_home: Option<&str>,
    default_app_dirs: &HashSet<String>,
    extra_args: &[String],
    fast_after_close: bool,
    launch_proxy: &CodexLaunchProxy,
) -> Result<(u32, PathBuf), CodexDefaultLaunchFailure> {
    use std::os::windows::process::CommandExt;
    use CodexDefaultLaunchFailure::{Dispatched, NotStarted};

    if !is_usable_codex_windows_gui_path(selected) {
        return Err(NotStarted(app_path_missing_error("codex")));
    }
    // Share the managed-instance resolver: manifest first, then registration
    // matched to this executable. Query failure never implies an unpackaged exe.
    let package = prepare_codex_package_launch(selected).map_err(NotStarted)?;
    let route = codex_managed_launch_route(selected, package.as_ref()).map_err(NotStarted)?;
    let mut launch_path = package
        .as_ref()
        .map(|package| PathBuf::from(&package.executable))
        .unwrap_or_else(|| selected.to_path_buf());
    let probe_started = Instant::now();
    let before_pids = collect_codex_windows_default_process_entries(
        &launch_path.to_string_lossy(),
        default_app_dirs,
        fast_after_close,
    )
    .into_iter()
    .map(|(pid, _)| pid)
    .collect::<HashSet<_>>();
    crate::modules::logger::log_info(&format!(
        "[Codex Start] before pid probe mode={}, count={}, elapsed_ms={}",
        if fast_after_close { "fast" } else { "full" },
        before_pids.len(),
        probe_started.elapsed().as_millis()
    ));
    let launch_not_before_epoch_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut receipt_pid = None;
    let mut package_family = None;
    if route == CodexManagedLaunchRoute::PackageIdentity {
        let package = package.expect("registered package route");
        let mut env = managed_proxy_env_pairs()
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect::<Vec<_>>();
        env.extend(
            launch_proxy
                .env_overrides()
                .into_iter()
                .map(|(key, value)| (key.to_string(), value)),
        );
        crate::modules::logger::log_info(&format!(
            "[Codex Start] strategy=package-identity family={} app_id={} launch_path={}",
            package.family_name,
            package.app_id,
            launch_path.display()
        ));
        let confirmed = activate_codex_default_package_with_refresh(
            package,
            |package| run_codex_package_activation(package, codex_home, None, extra_args, &env)
                .map(|pid| { receipt_pid = Some(pid); }),
            || {
                query_codex_registered_launch(selected)?
                    .ok_or_else(|| "No registered Codex GUI application".into())
            },
        )?;
        launch_path = PathBuf::from(confirmed.executable);
        package_family = Some(confirmed.family_name);
    } else {
        crate::modules::logger::log_info(&format!(
            "[Codex Start] strategy=exe-path launch_path={}",
            launch_path.display()
        ));
        let mut command = Command::new(&launch_path);
        apply_effective_proxy_env_to_command(&mut command, launch_proxy);
        if should_detach_child() {
            command.creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
            command
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
        }
        // The GUI client needs to spawn its own CLI; keep the existing GUI flags.
        prepare_codex_windows_default_command(&mut command, extra_args, codex_home);
        let child = spawn_command_with_trace(&mut command)
            .map_err(|error| NotStarted(format!("启动 Codex 失败: {error}")))?;
        crate::modules::logger::log_info(&format!("[Codex Start] exe launcher pid={}", child.id()));
        receipt_pid = Some(child.id());
    }
    let pid = wait_for_codex_default_start_pid(
        &launch_path.to_string_lossy(),
        &before_pids,
        default_app_dirs,
        launch_not_before_epoch_secs,
        Duration::from_secs(15),
        receipt_pid,
        package_family.as_deref(),
    )
    .ok_or_else(|| {
        Dispatched(format!("CODEX_START_CONFIRMATION_UNCERTAIN:{}", serde_json::json!({
            "beforePids": before_pids, "reason": "no stable new default process"
        })))
    })?;
    Ok((pid, launch_path))
}

#[cfg(test)]
#[path = "process_codex_windows_default_launch_tests.rs"]
mod codex_windows_default_launch_tests;
