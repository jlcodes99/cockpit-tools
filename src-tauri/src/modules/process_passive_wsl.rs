// Passive scans must not dereference a stopped WSL distribution.
#[cfg(any(test, target_os = "windows"))]
fn wsl_unc_distro(path: &str) -> Option<String> {
    let normalized = path.replace('/', "\\").to_ascii_lowercase();
    let unc = normalized
        .strip_prefix(r"\\?\unc\")
        .map(|rest| format!(r"\\{rest}"))
        .unwrap_or(normalized);
    [r"\\wsl.localhost", r"\\wsl$"].iter().find_map(|host| {
        if unc == *host {
            Some(String::new())
        } else {
            unc.strip_prefix(&format!(r"{host}\"))
                .map(|rest| rest.split('\\').next().unwrap_or("").to_string())
        }
    })
}

#[cfg(target_os = "windows")]
pub(crate) fn can_probe_passive_windows_path(path: &str) -> bool {
    use std::os::windows::process::CommandExt;

    let Some(distro) = wsl_unc_distro(path) else {
        return true;
    };
    if distro.is_empty() {
        return false;
    }
    // A failed/stopped probe must not repeat for every candidate. Never cache a
    // positive answer: a distro may stop between two independent scans.
    static NEGATIVE_PROBES: std::sync::LazyLock<
        std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>>,
    > = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    {
        let Ok(mut negative) = NEGATIVE_PROBES.try_lock() else {
            return false;
        };
        negative.retain(|_, at| at.elapsed() < std::time::Duration::from_secs(5));
        if negative.contains_key(&distro) || negative.contains_key("") {
            return false;
        }
    }
    static PROBE_RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    use std::sync::atomic::Ordering;
    if PROBE_RUNNING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return false;
    }
    struct ProbeGuard;
    impl Drop for ProbeGuard {
        fn drop(&mut self) {
            PROBE_RUNNING.store(false, Ordering::Release);
        }
    }
    let _guard = ProbeGuard;
    let mut command = Command::new("wsl.exe");
    command.args(["--list", "--running", "--quiet"]);
    command.creation_flags(CREATE_NO_WINDOW);
    // No state/cache lock spans the child process wait.
    let output = crate::modules::process_timeout::output_with_timeout(
        &mut command,
        WINDOWS_PROCESS_PROBE_TIMEOUT,
    )
    .ok()
    .filter(|output| output.status.success());
    let running = output
        .as_ref()
        .is_some_and(|output| running_wsl_output_contains_distro(&output.stdout, &distro));
    if !running {
        if let Ok(mut negative) = NEGATIVE_PROBES.lock() {
            negative.insert(
                if output.is_some() {
                    distro
                } else {
                    String::new()
                },
                std::time::Instant::now(),
            );
        }
    }
    running
}

#[cfg(any(test, target_os = "windows"))]
fn running_wsl_output_contains_distro(stdout: &[u8], distro: &str) -> bool {
    if distro.is_empty() || stdout.len() % 2 != 0 {
        return false;
    }
    let names = stdout
        .chunks_exact(2)
        .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
        .collect::<Vec<_>>();
    String::from_utf16(&names)
        .map(|text| {
            text.trim_start_matches('\u{feff}').lines().any(|name| {
                name.trim_matches(|c: char| c.is_whitespace() || c == '\0')
                    .eq_ignore_ascii_case(distro)
            })
        })
        .unwrap_or(false)
}
