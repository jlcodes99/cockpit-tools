use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use sysinfo::{
    Pid, Process, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, Signal, System, UpdateKind,
};

pub(super) struct Hub {
    pid: Pid,
    started_at: u64,
    executable: PathBuf,
    args: Vec<OsString>,
    ide_owned: bool,
}

fn is_app_data_hub(args: &[OsString]) -> bool {
    let mut data_dirs = args.iter().enumerate().filter_map(|(index, arg)| {
        let arg = arg.to_str()?;
        if arg == "--app_data_dir" {
            Some(args.get(index + 1).and_then(|value| value.to_str()))
        } else {
            arg.strip_prefix("--app_data_dir=").map(Some)
        }
    });
    args.iter().filter(|arg| *arg == "--hub").count() == 1
        && data_dirs.next() == Some(Some("antigravity"))
        && data_dirs.next().is_none()
}

fn process_details() -> ProcessRefreshKind {
    ProcessRefreshKind::nothing()
        .with_exe(UpdateKind::Always)
        .with_cmd(UpdateKind::Always)
        .with_user(UpdateKind::Always)
}

fn verified_hub(process: &Process, executable: &std::path::Path) -> bool {
    process
        .user_id()
        .is_some_and(|uid| **uid == unsafe { libc::geteuid() })
        && process
            .exe()
            .and_then(|path| path.canonicalize().ok())
            .as_deref()
            == Some(executable)
        && is_app_data_hub(process.cmd())
}

pub(super) fn snapshot() -> Vec<Hub> {
    let Some(executable) =
        dirs::home_dir().and_then(|home| home.join(".gemini/bin/agy").canonicalize().ok())
    else {
        return Vec::new();
    };
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::All, true);
    let pids: Vec<_> = system
        .processes()
        .values()
        .filter(|process| process.name() == "agy")
        .map(Process::pid)
        .collect();
    system.refresh_processes_specifics(ProcessesToUpdate::Some(&pids), true, process_details());
    pids.into_iter()
        .filter_map(|pid| {
            let process = system.process(pid)?;
            if !verified_hub(process, &executable) {
                return None;
            }
            let mut parent = process.parent();
            let mut ide_owned = false;
            for _ in 0..16 {
                let Some(ancestor) = parent.and_then(|pid| system.process(pid)) else {
                    break;
                };
                if ancestor
                    .name()
                    .to_string_lossy()
                    .to_ascii_lowercase()
                    .contains("antigravity ide")
                {
                    ide_owned = true;
                    break;
                }
                parent = ancestor.parent();
            }
            Some(Hub {
                pid,
                started_at: process.start_time(),
                executable: executable.clone(),
                args: process.cmd().iter().skip(1).cloned().collect(),
                ide_owned,
            })
        })
        .collect()
}

pub(super) fn stop(hub: &Hub) -> Result<bool, String> {
    let mut system = System::new();
    let pids = [hub.pid];
    system.refresh_processes_specifics(ProcessesToUpdate::Some(&pids), true, process_details());
    let Some(process) = system
        .process(hub.pid)
        .filter(|p| p.start_time() == hub.started_at)
    else {
        return Ok(false);
    };
    if !verified_hub(process, &hub.executable) || process.kill_with(Signal::Term) != Some(true) {
        return Err(format!("App Hub (PID {}) could not be stopped", hub.pid));
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        system.refresh_processes(ProcessesToUpdate::Some(&pids), true);
        if !system.process(hub.pid).is_some_and(|p| {
            p.start_time() == hub.started_at && p.status() != ProcessStatus::Zombie
        }) {
            return Ok(!hub.ide_owned);
        }
        if Instant::now() >= deadline {
            return Err(format!("App Hub (PID {}) did not exit", hub.pid));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

pub(super) fn restart(hubs: Vec<Hub>) -> Result<(), String> {
    for hub in hubs {
        let mut child = Command::new(hub.executable)
            .args(hub.args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("App Hub: {error}"))?;
        let result = wait_for_listener(&mut child);
        if result.is_err() {
            let _ = child.kill();
            let _ = child.wait();
        } else {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        result?;
    }
    Ok(())
}

fn wait_for_listener(child: &mut std::process::Child) -> Result<(), String> {
    let pid = child.id().to_string();
    let deadline = Instant::now() + Duration::from_secs(5);
    while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("App Hub: {error}"))?
        {
            return Err(format!("App Hub: {status}"));
        }
        let mut probe = Command::new("/usr/sbin/lsof");
        probe.args(["-nP", "-a", "-p", &pid, "-iTCP", "-sTCP:LISTEN", "-Fn"]);
        if crate::modules::process_timeout::output_with_timeout(
            &mut probe,
            remaining.min(Duration::from_millis(500)),
        )
        .is_ok_and(|output| {
            output.status.success()
                && output
                    .stdout
                    .split(|b| *b == b'\n')
                    .any(|line| line.starts_with(b"n"))
        }) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(format!(
        "App Hub: {}",
        std::io::Error::from(std::io::ErrorKind::TimedOut)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_shared_app_data_hub_arguments_match() {
        let matches = |s: &str| {
            is_app_data_hub(&s.split_whitespace().map(OsString::from).collect::<Vec<_>>())
        };
        assert!(matches("agy --hub --app_data_dir antigravity"));
        assert!(matches("agy --app_data_dir=antigravity --hub"));
        assert!(!matches("agy --app_data_dir antigravity"));
        assert!(!matches("agy --hub --app_data_dir antigravity-cli"));
        assert!(!matches(
            "agy --hub --app_data_dir antigravity --app_data_dir=other"
        ));
        assert!(!matches("agy --hub --hub --app_data_dir antigravity"));
    }
}
