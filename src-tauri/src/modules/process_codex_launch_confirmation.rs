// A receipt belongs to this launch request and carries the PID created with its
// exact profile/environment. Confirm that process, not every unrelated instance.
#[cfg(any(test, target_os = "windows"))]
fn codex_receipt_candidate(
    receipt_pid: Option<u32>,
    before: &HashSet<u32>,
    mut verify: impl FnMut(u32) -> bool,
) -> Option<u32> {
    receipt_pid.filter(|pid| !before.contains(pid) && verify(*pid))
}

#[cfg(target_os = "windows")]
fn verify_codex_launch_receipt_pid(pid: u32, expected: &str, family: Option<&str>) -> bool {
    if family.is_some_and(|family| verify_codex_process_package(pid, family).is_err()) {
        return false;
    }
    let mut system = System::new();
    let pids = [Pid::from_u32(pid)];
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::Some(&pids),
        true,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet),
    );
    system.process(pids[0]).is_some_and(|process| {
        let (exe, _) = resolve_windows_process_exe_for_match(process);
        exe.is_some_and(|exe| is_matching_codex_windows_exe(&exe, expected))
    })
}

#[cfg(any(test, target_os = "windows"))]
fn resolve_codex_launch_maintenance_snapshot(
    entries: Vec<(u32, Option<String>)>,
    complete: bool,
    probe: impl FnOnce() -> Result<Vec<(u32, Option<String>)>, String>,
) -> Result<Vec<(u32, Option<String>)>, String> {
    fresh_codex_entries_or_probe(entries, complete, probe)
        .map_err(|_| "CODEX_SESSION_VISIBILITY_PROCESS_UNAVAILABLE".into())
}

/// Account state preparation can verify an incomplete snapshot using a fresh probe.
/// Optional startup maintenance can report an incomplete snapshot and defer.
pub fn collect_codex_launch_maintenance_process_entries(
    #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] probe_when_incomplete: bool,
) -> Result<Vec<(u32, Option<String>)>, String> {
    #[cfg(target_os = "windows")]
    {
        let path = match resolve_codex_launch_path() {
            Ok(path) => path,
            Err(error) => {
                // CLI-only installations need no desktop path. Prove that no
                // supported GUI process exists before treating this as empty.
                let mut system = System::new();
                system.refresh_processes_specifics(
                    sysinfo::ProcessesToUpdate::All, true, ProcessRefreshKind::nothing(),
                );
                if system.processes().values().any(|process| {
                    process.name().to_string_lossy().eq_ignore_ascii_case("chatgpt.exe")
                }) {
                    return Err(format!("CODEX_SESSION_VISIBILITY_PROCESS_UNAVAILABLE: {error}"));
                }
                return Ok(Vec::new());
            }
        };
        let expected = path.to_string_lossy();
        let (entries, complete) = collect_codex_process_snapshot_from_sysinfo(&expected);
        return resolve_codex_launch_maintenance_snapshot(entries, complete, || {
            if probe_when_incomplete {
                try_collect_codex_process_entries_from_powershell(&expected)
            } else {
                Err("首次进程快照不完整".into())
            }
        });
    }
    #[cfg(not(target_os = "windows"))]
    Ok(collect_codex_process_entries())
}

/// Reconciliation must not turn a retained status cache into launch success.
pub fn collect_codex_launch_confirmation_process_entries() -> Result<Vec<(u32, Option<String>)>, String> {
    #[cfg(target_os = "windows")]
    { return try_collect_codex_process_entries_complete(); }
    #[cfg(not(target_os = "windows"))]
    Ok(collect_codex_process_entries())
}

#[cfg(test)]
mod codex_launch_confirmation_tests {
    use super::*;

    #[test]
    fn launch_maintenance_can_confirm_exit_when_the_native_snapshot_is_incomplete() {
        let entries = resolve_codex_launch_maintenance_snapshot(vec![], false, || Ok(vec![]))
            .expect("complete probe confirmed that Codex stopped");
        assert!(entries.is_empty());
    }

    #[test]
    fn launch_maintenance_preserves_running_instances_and_unknown_process_state() {
        let default_instance = (42, None);
        let native_entries = vec![(65, Some("managed-profile".into()))];
        let entries = resolve_codex_launch_maintenance_snapshot(native_entries.clone(), false, || {
            Ok(vec![default_instance.clone()])
        })
        .unwrap();
        assert_eq!(entries, vec![default_instance, native_entries[0].clone()]);
        assert_eq!(
            resolve_codex_launch_maintenance_snapshot(native_entries, false, || Err("probe failed".into())).unwrap_err(),
            "CODEX_SESSION_VISIBILITY_PROCESS_UNAVAILABLE",
        );
    }

    #[test]
    fn launch_maintenance_uses_a_complete_native_snapshot_immediately() {
        let entries = resolve_codex_launch_maintenance_snapshot(vec![], true, || {
            panic!("complete native snapshot does not need PowerShell")
        })
        .unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn receipt_never_accepts_old_exited_or_mismatched_processes() {
        let before = HashSet::from([10]);
        assert_eq!(codex_receipt_candidate(Some(10), &before, |_| panic!("old PID")), None);
        assert_eq!(codex_receipt_candidate(Some(20), &before, |_| false), None);
        assert_eq!(codex_receipt_candidate(None, &before, |_| panic!("no receipt")), None);
        assert_eq!(codex_receipt_candidate(Some(20), &before, |pid| pid == 20), Some(20));
    }
}
