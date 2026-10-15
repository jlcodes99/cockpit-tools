// Windows Codex 受管实例：注册包解析、包身份启动及实例确认。
/// 从商店包启动路径反推包的 `InstallLocation`。
///
/// 启动路径形如 `<InstallLocation>\app\ChatGPT.exe`，因此去掉两级即安装根目录；
/// 非商店路径返回 `None`。
///
/// 这里按文本解析而不是 `Path::parent()`：该函数只处理 Windows 路径，而
/// `Path::parent()` 在非 Windows 主机（单元测试）上不会把 `\` 当分隔符。
#[cfg(any(test, target_os = "windows"))]
fn windowsapps_install_location_from_launch_path(launch_path: &Path) -> Option<String> {
    if !is_windowsapps_launch_path(launch_path) {
        return None;
    }
    let normalized = launch_path.to_string_lossy().replace('/', "\\");
    let trimmed = normalized.trim_end_matches('\\');
    let install_location = trimmed.rsplitn(3, '\\').nth(2)?;
    let text = install_location.trim().to_string();
    if text.is_empty() {
        return None;
    }
    Some(text)
}

#[cfg(any(test, target_os = "windows"))]
#[derive(Debug, Clone, serde::Deserialize)]
struct CodexRegisteredLaunch {
    family_name: String,
    app_id: String,
    executable: String,
}

/// Local manifest data is a launch hint, never proof of current registration.
/// The activation script still checks the registered family, application and exe.
#[cfg(any(test, target_os = "windows"))]
fn codex_package_hint_from_manifest(path: &Path, text: &str) -> Option<CodexRegisteredLaunch> {
    let family_name = codex_store_package_family_from_path(path)?;
    let root = windowsapps_install_location_from_launch_path(path)?;
    let app_id = codex_manifest_app_id(Path::new(&root), path, text)?;
    Some(CodexRegisteredLaunch {
        family_name,
        app_id,
        executable: path.to_string_lossy().into_owned(),
    })
}

#[cfg(any(test, target_os = "windows"))]
fn codex_manifest_app_id(root: &Path, path: &Path, text: &str) -> Option<String> {
    use quick_xml::{events::Event, Reader};
    let root = root.to_string_lossy();
    let expected = normalized_windows_path_text(path);
    let mut reader = Reader::from_str(text);
    let mut matched = None;
    loop {
        match reader.read_event().ok()? {
            Event::Start(element) | Event::Empty(element)
                if element.local_name().as_ref() == b"Application" =>
            {
                let mut id = None;
                let mut executable = None;
                for attribute in element.attributes() {
                    let attribute = attribute.ok()?;
                    let value = attribute.decode_and_unescape_value(reader.decoder()).ok()?;
                    match attribute.key.as_ref() {
                        b"Id" => id = Some(value.into_owned()),
                        b"Executable" => executable = Some(value.into_owned()),
                        _ => {}
                    }
                }
                if let (Some(app_id), Some(relative)) = (id, executable) {
                    if relative.contains(':')
                        || relative.starts_with(['/', '\\'])
                        || relative.split(['/', '\\']).any(|part| part == "..")
                    {
                        continue;
                    }
                    let candidate =
                        PathBuf::from(format!(r"{}\{}", root, relative.replace('/', "\\")));
                    if normalized_windows_path_text(&candidate) == expected
                        && is_chatgpt_store_gui_exe(&candidate)
                        && !app_id.trim().is_empty()
                    {
                        if matched.is_some() {
                            return None;
                        }
                        matched = Some(app_id);
                    }
                }
            }
            Event::Eof => return matched,
            _ => {}
        }
    }
}

#[cfg(target_os = "windows")]
fn codex_package_hint_from_path(path: &Path) -> Option<CodexRegisteredLaunch> {
    let root = windowsapps_install_location_from_launch_path(path)?;
    let manifest = Path::new(&root).join("AppxManifest.xml");
    if !path.is_file() || std::fs::metadata(&manifest).ok()?.len() > 1024 * 1024 {
        return None;
    }
    let text = std::fs::read_to_string(manifest).ok()?;
    codex_package_hint_from_manifest(path, &text)
}

#[cfg(target_os = "windows")]
fn prepare_codex_package_launch(path: &Path) -> Result<Option<CodexRegisteredLaunch>, String> {
    let started = Instant::now();
    let hint = codex_package_hint_from_path(path);
    let source = if hint.is_some() {
        "local-manifest"
    } else {
        "registration-query"
    };
    let result = match hint {
        Some(package) => Ok(Some(package)),
        None => query_codex_registered_launch(path),
    };
    crate::modules::logger::log_info(&format!(
        "[Codex Start] entry preparation source={source} elapsed_ms={}",
        started.elapsed().as_millis()
    ));
    result
}

#[cfg(target_os = "windows")]
fn query_codex_registered_launch(path: &Path) -> Result<Option<CodexRegisteredLaunch>, String> {
    let started = Instant::now();
    let result = codex_launch_powershell_output(&build_codex_registered_launch_probe(path))
        .and_then(|output| parse_codex_registered_launch(&output));
    crate::modules::logger::log_info(&format!(
        "[Codex Start] registration query elapsed_ms={} success={}",
        started.elapsed().as_millis(),
        result.is_ok()
    ));
    result
}

/// Retry only registration validation failures emitted BEFORE activation. Never
/// retry an ambiguous activation failure, which could otherwise launch twice.
#[cfg(any(test, target_os = "windows"))]
fn activate_codex_package_with_refresh(
    mut package: CodexRegisteredLaunch,
    mut activate: impl FnMut(&CodexRegisteredLaunch) -> Result<(), String>,
    refresh: impl FnOnce() -> Result<CodexRegisteredLaunch, String>,
) -> Result<CodexRegisteredLaunch, String> {
    let mut result = activate(&package);
    if result
        .as_ref()
        .is_err_and(|error| error.contains("CODEX_PACKAGE_REGISTRATION_CHANGED"))
    {
        package = refresh()?;
        result = activate(&package);
    }
    if let Err(error) = result {
        if !error.starts_with("CODEX_ACTIVATION_UNCERTAIN:") {
            return Err(error);
        }
        #[cfg(target_os = "windows")]
        crate::modules::logger::log_warn(&format!(
            "[Codex Start] {error}; confirming target process without another activation"
        ));
    }
    Ok(package)
}

/// Include the publisher ID when resolving an updated package, not just its name.
#[cfg(any(test, target_os = "windows"))]
fn codex_store_package_family_from_path(path: &Path) -> Option<String> {
    let normalized = normalized_windows_path_text(path);
    let directory = normalized
        .split(r"\windowsapps\")
        .nth(1)?
        .split('\\')
        .next()?;
    let (name_version, publisher) = directory.rsplit_once("__")?;
    let name = name_version.split('_').next()?;
    if name.is_empty() || publisher.is_empty() {
        return None;
    }
    Some(format!("{name}_{publisher}"))
}

/// WindowsApps is only a routing hint. Activation itself requires a registered
/// package AND a manifest Application whose executable matches the selected GUI.
#[cfg(any(test, target_os = "windows"))]
fn build_codex_registered_launch_probe(launch_path: &Path) -> String {
    format!(
        r#"$ErrorActionPreference = 'Stop'
function Normalize-LaunchPath([string]$path) {{
  $path = $path.Replace('/', '\')
  if ($path.StartsWith('\\?\UNC\', [StringComparison]::OrdinalIgnoreCase)) {{ $path = '\\' + $path.Substring(8) }}
  elseif ($path.StartsWith('\\?\')) {{ $path = $path.Substring(4) }}
  return [IO.Path]::GetFullPath($path).TrimEnd('\')
}}
$target = Normalize-LaunchPath '{exe}'
$storeFamily = '{store_family}'
$packages = @({package_query} | Where-Object {{ $_.InstallLocation }} | Sort-Object Version -Descending)
$matches = @()
foreach ($pkg in $packages) {{
  $root = Normalize-LaunchPath $pkg.InstallLocation
  $exactRoot = $target.StartsWith($root + '\', [StringComparison]::OrdinalIgnoreCase)
  if (-not $exactRoot -and (-not $storeFamily -or $pkg.PackageFamilyName -ine $storeFamily)) {{ continue }}
  $manifest = Get-AppxPackageManifest -Package $pkg -ErrorAction Stop
  foreach ($application in $manifest.Package.Applications.Application) {{
    $relative = [string]$application.Executable
    if (-not $relative -or [IO.Path]::IsPathRooted($relative)) {{ continue }}
    $candidate = [IO.Path]::GetFullPath((Join-Path $root $relative))
    if (-not $candidate.StartsWith($root + '\', [StringComparison]::OrdinalIgnoreCase)) {{ continue }}
    $isTarget = $candidate -ieq $target
    $isUpdatedGui = $storeFamily -and $pkg.PackageFamilyName -ieq $storeFamily -and [IO.Path]::GetFileName($candidate) -ieq 'ChatGPT.exe'
    if (($isTarget -or $isUpdatedGui) -and (Test-Path -LiteralPath $candidate -PathType Leaf) -and $application.Id) {{
      $matches += [PSCustomObject]@{{ family_name = [string]$pkg.PackageFamilyName; app_id = [string]$application.Id; executable = $candidate }}
    }}
  }}
  if ($matches.Count -gt 0) {{ break }}
}}
if ($matches.Count -gt 1) {{ throw 'Ambiguous registered Codex GUI applications' }}
if ($matches.Count -eq 1) {{ $matches[0] | ConvertTo-Json -Compress }} else {{ Write-Output 'null' }}"#,
        exe = escape_powershell_single_quoted(&launch_path.to_string_lossy()),
        package_query = codex_store_package_family_from_path(launch_path)
            .map(|family| format!(
                "Get-AppxPackage -Name '{}' -ErrorAction Stop",
                escape_powershell_single_quoted(family.rsplit_once('_').unwrap().0)
            ))
            .unwrap_or_else(|| "Get-AppxPackage -ErrorAction Stop".into()),
        store_family = escape_powershell_single_quoted(
            &codex_store_package_family_from_path(launch_path).unwrap_or_default()
        ),
    )
}

#[cfg(target_os = "windows")]
fn codex_launch_powershell_output(script: &str) -> Result<String, String> {
    // This helper drains pipes concurrently and bounds both process and pipe waits.
    // Do not log the script: it can contain per-account proxy credentials.
    for executable in windows_powershell_executable_candidates_for_host() {
        let mut command = build_powershell_command(&executable, &["-Command", script]);
        match crate::modules::process_timeout::output_with_timeout(
            &mut command,
            Duration::from_secs(15),
        ) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {
                return Err(format!(
                    "CODEX_ACTIVATION_UNCERTAIN: PowerShell timed out: {error}"
                ))
            }
            Err(error) => return Err(format!("PowerShell launch/probe failed: {error}")),
            Ok(output) => {
                if !output.status.success() {
                    if String::from_utf8_lossy(&output.stdout)
                        .lines()
                        .any(|line| line.trim() == "CODEX_PACKAGE_REGISTRATION_CHANGED")
                    {
                        return Err("CODEX_PACKAGE_REGISTRATION_CHANGED: registration validation failed before activation".into());
                    }
                    return Err(format!(
                        "PowerShell launch/probe failed: status={}, stderr={}",
                        output.status,
                        String::from_utf8_lossy(&output.stderr)
                            .trim()
                            .chars()
                            .take(600)
                            .collect::<String>()
                    ));
                }
                return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
            }
        }
    }
    Err("PowerShell executable not found".to_string())
}

#[cfg(any(test, target_os = "windows"))]
fn parse_codex_registered_launch(output: &str) -> Result<Option<CodexRegisteredLaunch>, String> {
    let registered: Option<CodexRegisteredLaunch> =
        serde_json::from_str(output.trim_start_matches('\u{feff}'))
            .map_err(|error| format!("Invalid package registration response: {error}"))?;
    if let Some(package) = &registered {
        if package.family_name.trim().is_empty()
            || package.app_id.trim().is_empty()
            || !is_chatgpt_store_gui_exe(Path::new(&package.executable))
        {
            return Err("Invalid registered Codex GUI application".to_string());
        }
    }
    Ok(registered)
}

/// Resolve the selected StartApps entry to its registered GUI executable before
/// activation. A shell alias cannot carry a per-launch environment block.
#[cfg(any(test, target_os = "windows"))]
fn build_codex_default_registered_launch_probe(app_user_model_id: &str) -> Result<String, String> {
    let (family, app_id) = app_user_model_id
        .trim()
        .split_once('!')
        .filter(|(family, app_id)| {
            !family.is_empty() && !app_id.is_empty() && !app_id.contains('!')
        })
        .ok_or_else(|| "Invalid Codex AppUserModelId".to_string())?;
    Ok(format!(
        r#"$ErrorActionPreference = 'Stop'
$family = '{family}'
$appId = '{app_id}'
$pkg = Get-AppxPackage -Name '{package_name}' -ErrorAction Stop | Where-Object {{ $_.PackageFamilyName -ieq $family }} | Sort-Object Version -Descending | Select-Object -First 1
if (-not $pkg) {{ throw 'Codex package is not registered' }}
$root = [IO.Path]::GetFullPath($pkg.InstallLocation).TrimEnd('\')
$matches = @()
foreach ($application in (Get-AppxPackageManifest -Package $pkg -ErrorAction Stop).Package.Applications.Application) {{
  if ($application.Id -ne $appId) {{ continue }}
  $relative = [string]$application.Executable
  if (-not $relative -or [IO.Path]::IsPathRooted($relative)) {{ continue }}
  $exe = [IO.Path]::GetFullPath((Join-Path $root $relative))
  if ($exe.StartsWith($root + '\', [StringComparison]::OrdinalIgnoreCase) -and [IO.Path]::GetFileName($exe) -ieq 'ChatGPT.exe' -and (Test-Path -LiteralPath $exe -PathType Leaf)) {{
    $matches += [PSCustomObject]@{{ family_name = [string]$pkg.PackageFamilyName; app_id = [string]$application.Id; executable = $exe }}
  }}
}}
if ($matches.Count -ne 1) {{ throw 'No unique registered Codex GUI application' }}
$matches[0] | ConvertTo-Json -Compress"#,
        family = escape_powershell_single_quoted(family),
        package_name = escape_powershell_single_quoted(
            family
                .rsplit_once('_')
                .map(|(name, _)| name)
                .unwrap_or(family)
        ),
        app_id = escape_powershell_single_quoted(app_id),
    ))
}

#[cfg(any(test, target_os = "windows"))]
#[derive(Debug, PartialEq)]
enum CodexManagedLaunchRoute {
    PackageIdentity,
    DirectExe,
}

#[cfg(any(test, target_os = "windows"))]
fn codex_managed_launch_route(
    configured: &Path,
    registered: Option<&CodexRegisteredLaunch>,
) -> Result<CodexManagedLaunchRoute, String> {
    if registered.is_some() {
        Ok(CodexManagedLaunchRoute::PackageIdentity)
    } else if is_windowsapps_launch_path(configured) {
        Err("No matching registered Codex GUI package".to_string())
    } else {
        Ok(CodexManagedLaunchRoute::DirectExe)
    }
}

/// Use the selected profile exactly once, even if saved extra arguments contain
/// an old --user-data-dir. Both direct and packaged launches share this builder.
#[cfg(any(test, target_os = "windows"))]
fn build_codex_windows_profile_args(
    extra_args: &[String],
    app_user_data_dir: Option<&Path>,
) -> Vec<String> {
    let mut args = Vec::new();
    let mut input = build_codex_app_launch_args(extra_args).into_iter();
    while let Some(arg) = input.next() {
        if arg == "--user-data-dir" {
            let _ = input.next();
        } else if !arg.starts_with("--user-data-dir=") {
            args.push(arg);
        }
    }
    if let Some(app_user_data_dir) = app_user_data_dir {
        args.push(format!(
            "--user-data-dir={}",
            app_user_data_dir.to_string_lossy()
        ));
    }
    args
}

#[cfg(any(test, target_os = "windows"))]
fn codex_package_launch_request(
    package: &CodexRegisteredLaunch,
    codex_home: Option<&str>,
    app_user_data_dir: Option<&Path>,
    extra_args: &[String],
    env: &[(String, String)],
    result_path: PathBuf,
    nonce: String,
) -> codex_package_launcher::LaunchRequest {
    codex_package_launcher::LaunchRequest {
        family_name: package.family_name.clone(),
        app_id: package.app_id.clone(),
        executable: package.executable.clone(),
        args: build_codex_windows_profile_args(extra_args, app_user_data_dir),
        env: env.to_vec(),
        codex_home: codex_home.map(str::to_string),
        app_user_data_dir: app_user_data_dir.map(|path| path.to_string_lossy().into_owned()),
        result_path,
        nonce,
        verify_only: false,
    }
}

/// Invoke only the windowless host helper. Package activation rebuilds the
/// environment, so carry the payload explicitly; never persist it to a file.
#[cfg(any(test, target_os = "windows"))]
fn build_codex_package_activation_script(
    request: &codex_package_launcher::LaunchRequest,
    helper: &Path,
) -> Result<String, String> {
    use base64::{engine::general_purpose, Engine};
    let payload = general_purpose::STANDARD
        .encode(serde_json::to_vec(request).map_err(|error| error.to_string())?);
    if payload.len() > 30000 {
        return Err("Codex launch environment exceeds the helper limit".into());
    }
    Ok(format!(
        r#"$ErrorActionPreference = 'Stop'
$activationClock = [Diagnostics.Stopwatch]::StartNew()
try {{
  Invoke-CommandInDesktopPackage -PackageFamilyName '{family}' -AppId '{app_id}' -PreventBreakaway -Command '{helper}' -Args '{helper_arg} {payload}'
  Write-Output ('CODEX_LAUNCH_TIMING activation_ms=' + $activationClock.ElapsedMilliseconds)
}} catch {{
  $exception = $_.Exception
  while ($exception) {{
    if ($exception.HResult -in @(-2147023728, -2147024894, -2147024893)) {{
      Write-Output 'CODEX_PACKAGE_REGISTRATION_CHANGED'
      exit 1
    }}
    $exception = $exception.InnerException
  }}
  [Console]::Error.WriteLine('Codex package activation failed')
  exit 1
}}"#,
        helper_arg = codex_package_launcher::HELPER_ARG,
        payload = payload,
        family = escape_powershell_single_quoted(&request.family_name),
        app_id = escape_powershell_single_quoted(&request.app_id),
        helper = escape_powershell_single_quoted(&helper.to_string_lossy()),
    ))
}

#[cfg(target_os = "windows")]
fn launch_codex_via_package_identity(
    package: &CodexRegisteredLaunch,
    codex_home: &str,
    app_user_data_dir: &Path,
    extra_args: &[String],
    extra_env: &[(String, String)],
) -> Result<(), String> {
    let mut env: Vec<(String, String)> = managed_proxy_env_pairs()
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect();
    env.extend_from_slice(extra_env);
    run_codex_package_activation(
        package,
        Some(codex_home),
        Some(app_user_data_dir),
        extra_args,
        &env,
    )
}

#[cfg(target_os = "windows")]
fn run_codex_package_activation(
    package: &CodexRegisteredLaunch,
    codex_home: Option<&str>,
    app_user_data_dir: Option<&Path>,
    extra_args: &[String],
    env: &[(String, String)],
) -> Result<(), String> {
    use codex_package_launcher::{LaunchReply, PendingReceipt};
    let receipt = PendingReceipt::new().map_err(|error| error.to_string())?;
    let request = codex_package_launch_request(
        package,
        codex_home,
        app_user_data_dir,
        extra_args,
        env,
        receipt.path.clone(),
        receipt.nonce.clone(),
    );
    let helper = std::env::current_exe().map_err(|error| error.to_string())?;
    let script = build_codex_package_activation_script(&request, &helper)?;
    let started = Instant::now();
    let activation = codex_launch_powershell_output(&script);
    if let Ok(output) = &activation {
        for line in output
            .lines()
            .filter_map(|line| line.strip_prefix("CODEX_LAUNCH_TIMING "))
        {
            if let Some(value) = line.strip_prefix("activation_ms=") {
                if value.parse::<u64>().is_ok() {
                    crate::modules::logger::log_info(&format!(
                        "[Codex Start] activation_ms={value}"
                    ));
                }
            }
        }
    }
    let wait = if activation.is_ok()
        || activation
            .as_ref()
            .is_err_and(|error| error.starts_with("CODEX_ACTIVATION_UNCERTAIN:"))
    {
        Duration::from_secs(5)
    } else {
        Duration::from_millis(100)
    };
    let result = match receipt.wait(wait) {
        Some(LaunchReply::Spawned { pid, validation_us, helper_has_console }) => {
            crate::modules::logger::log_info(&format!("[Codex Start] native package validation_us={validation_us} helper_has_console={helper_has_console} spawned_pid={pid}"));
            Ok(())
        }
        Some(LaunchReply::EntryStale) => Err("CODEX_PACKAGE_REGISTRATION_CHANGED: native package entry validation failed before client creation".into()),
        Some(LaunchReply::Failed { message }) => Err(message),
        Some(LaunchReply::Validated { .. }) => Err("Unexpected validation-only helper reply".into()),
        None => match activation {
            Err(error) => Err(error),
            Ok(_) => Err("CODEX_ACTIVATION_UNCERTAIN: no helper receipt; client creation may already have occurred".into()),
        }
    };
    crate::modules::logger::log_info(&format!(
        "[Codex Start] activation command elapsed_ms={} success={}",
        started.elapsed().as_millis(),
        result.is_ok()
    ));
    result
}

#[cfg(target_os = "windows")]
fn verify_codex_process_package(pid: u32, expected_family: &str) -> Result<(), String> {
    use windows::core::PWSTR;
    use windows::Win32::Foundation::{CloseHandle, ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS};
    use windows::Win32::Storage::Packaging::Appx::GetPackageFamilyName;
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
            .map_err(|error| format!("OpenProcess pid={pid}: {error}"))?;
        let result = (|| {
            let mut size = 0;
            let status = GetPackageFamilyName(handle, &mut size, PWSTR::null());
            if status != ERROR_INSUFFICIENT_BUFFER || size == 0 || size > 32768 {
                return Err(format!(
                    "GetPackageFamilyName pid={pid}: win32={}",
                    status.0
                ));
            }
            let mut buffer = vec![0u16; size as usize];
            let status = GetPackageFamilyName(handle, &mut size, PWSTR(buffer.as_mut_ptr()));
            if status != ERROR_SUCCESS {
                return Err(format!(
                    "GetPackageFamilyName pid={pid}: win32={}",
                    status.0
                ));
            }
            let actual = String::from_utf16_lossy(
                &buffer[..buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len())],
            );
            if !actual.eq_ignore_ascii_case(expected_family) {
                return Err(format!(
                    "Package mismatch pid={pid}: expected={expected_family}, actual={actual}"
                ));
            }
            let mut exit_code = 0;
            GetExitCodeProcess(handle, &mut exit_code).map_err(|error| error.to_string())?;
            if exit_code != 259 {
                return Err(format!("Codex exited pid={pid}: exit_code={exit_code}"));
            }
            Ok(())
        })();
        let _ = CloseHandle(handle);
        result
    }
}

#[cfg(any(test, target_os = "windows"))]
fn codex_managed_launch_candidate(
    entries: &[(u32, Option<String>)],
    target: &str,
    before: &HashSet<u32>,
) -> Option<u32> {
    // Never accept missing/default user-data directories or a pre-existing PID.
    collect_matching_pids_by_user_data_dir(entries, target, false)
        .into_iter()
        .filter(|pid| !before.contains(pid))
        .min()
}

#[cfg(target_os = "windows")]
fn launch_windows_codex_managed_instance(
    configured: &Path,
    codex_home: &str,
    app_user_data_dir: &Path,
    extra_args: &[String],
    extra_env: &[(String, String)],
) -> Result<u32, String> {
    let fail = |path: &Path, stage: &str, error: &str| {
        codex_managed_store_launch_unsafe_error(
            "not attempted for registered Store packages",
            error,
            &format!(
                "launch_path={}; launch_stage={stage}; codex_home={codex_home}",
                path.display()
            ),
        )
    };
    // Query registration even for packages installed outside WindowsApps. Failure
    // to query is not evidence of an unpackaged executable: fail closed.
    if configured
        .to_string_lossy()
        .to_ascii_lowercase()
        .starts_with("shell:")
    {
        return Err(fail(
            configured,
            "registration",
            "Re-detect the executable path; shell aliases cannot isolate managed profiles",
        ));
    }
    let mut package = prepare_codex_package_launch(configured)
        .map_err(|error| fail(configured, "registration", &error))?;
    let route = codex_managed_launch_route(configured, package.as_ref())
        .map_err(|error| fail(configured, "registration", &error))?;
    let mut launch_path = package
        .as_ref()
        .map(|p| PathBuf::from(&p.executable))
        .unwrap_or_else(|| configured.to_path_buf());
    if normalized_windows_path_text(&launch_path) != normalized_windows_path_text(configured) {
        update_app_path_in_config("codex", &launch_path, &configured.to_string_lossy());
    }
    let before: HashSet<u32> =
        collect_codex_process_entries_from_sysinfo_fallback(&launch_path.to_string_lossy())
            .into_iter()
            .map(|(pid, _)| pid)
            .collect();
    let mut child = if route == CodexManagedLaunchRoute::PackageIdentity {
        let registered = package.as_ref().expect("registered package route");
        crate::modules::logger::log_info(&format!(
            "[Codex Start] strategy=package-identity family={} app_id={} launch_path={}",
            registered.family_name,
            registered.app_id,
            launch_path.display()
        ));
        let confirmed = activate_codex_package_with_refresh(
            registered.clone(),
            |package| {
                launch_codex_via_package_identity(
                    package,
                    codex_home,
                    app_user_data_dir,
                    extra_args,
                    extra_env,
                )
            },
            || {
                query_codex_registered_launch(configured)?
                    .ok_or_else(|| "No registered Codex GUI application".into())
            },
        )
        .map_err(|error| fail(&launch_path, "activation", &error))?;
        launch_path = PathBuf::from(&confirmed.executable);
        if normalized_windows_path_text(&launch_path) != normalized_windows_path_text(configured) {
            update_app_path_in_config("codex", &launch_path, &configured.to_string_lossy());
        }
        package = Some(confirmed);
        None
    } else {
        Some(
            spawn_command_with_trace(&mut build_windows_codex_instance_command(
                &launch_path,
                codex_home,
                app_user_data_dir,
                extra_args,
                extra_env,
            ))
            .map_err(|error| format!("Codex launch failed: {error}"))?,
        )
    };
    let expected = launch_path.to_string_lossy();
    let target = normalize_path_for_compare(&app_user_data_dir.to_string_lossy());
    let started = Instant::now();
    let mut stable_pid = None;
    let mut stable_since = Instant::now();
    let mut last_error = "No matching managed Codex process".to_string();
    let mut first_candidate_ms = None;
    while started.elapsed() < Duration::from_secs(15) {
        if let Some(child) = child.as_mut() {
            if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
                return Err(format!(
                    "Codex exited before startup confirmation: {status}"
                ));
            }
        }
        let entries = collect_codex_process_entries_from_sysinfo_fallback(&expected);
        let candidate = codex_managed_launch_candidate(&entries, &target, &before);
        let candidate = candidate.filter(|pid| is_pid_running(*pid));
        if candidate.is_some() && first_candidate_ms.is_none() {
            first_candidate_ms = Some(started.elapsed().as_millis());
            crate::modules::logger::log_info(&format!(
                "[Codex Start] first managed process elapsed_ms={}",
                first_candidate_ms.unwrap()
            ));
        }
        if candidate != stable_pid {
            stable_pid = candidate;
            stable_since = Instant::now();
        }
        if let Some(pid) = candidate {
            let identity = package
                .as_ref()
                .map(|p| verify_codex_process_package(pid, &p.family_name))
                .unwrap_or(Ok(()));
            match identity {
                Ok(()) if stable_since.elapsed() >= Duration::from_millis(750) => {
                    crate::modules::logger::log_info(&format!("[Codex Start] managed instance confirmed pid={pid} package={} elapsed_ms={}", package.as_ref().map(|p| p.family_name.as_str()).unwrap_or("unpackaged"), started.elapsed().as_millis()));
                    return Ok(pid);
                }
                Err(error) => {
                    last_error = error;
                    stable_since = Instant::now();
                }
                _ => {}
            }
        }
        thread::sleep(Duration::from_millis(250));
    }
    crate::modules::logger::log_warn(&format!(
        "[Codex Start] confirmation failed launch_path={} error={last_error}",
        launch_path.display()
    ));
    if package.is_some() {
        Err(fail(&launch_path, "confirmation", &last_error))
    } else {
        Err(format!("Codex startup confirmation failed: {last_error}"))
    }
}

#[cfg(test)]
#[path = "process_codex_windows_launch_tests.rs"]
mod codex_windows_managed_launch_tests;
