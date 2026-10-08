//! Read-only discovery of desktop processes still using a managed proxy port.
//! Reuse the official desktop matcher so the default instance keeps its `None`
//! profile context on macOS and Windows; never infer a profile from a proxy port.
use crate::modules::process;
use std::ffi::OsString;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RunningEntry {
    pub pid: u32,
    pub profile_dir: Option<String>,
    pub proxy_port: u16,
}

/// Call from spawn_blocking, never from a runtime/UI thread. Command arguments
/// come from a bounded OS snapshot, without logging command lines (which may
/// contain private paths or credentials).
pub(super) fn running_entries() -> Result<Vec<RunningEntry>, String> {
    Ok(process::collect_codex_proxy_process_snapshot()?
        .into_iter()
        .filter_map(|(pid, profile_dir, args)| entry_from_args(pid, profile_dir, &args))
        .collect())
}

fn entry_from_args(
    pid: u32,
    profile_dir: Option<String>,
    args: &[OsString],
) -> Option<RunningEntry> {
    Some(RunningEntry {
        pid,
        profile_dir,
        proxy_port: proxy_port_from_args(args)?,
    })
}

fn proxy_port_from_args(args: &[OsString]) -> Option<u16> {
    let mut port = None;
    let mut args = args.iter().skip(1);
    while let Some(arg) = args.next() {
        let arg = arg.to_str()?;
        // Ambiguous or overriding proxy flags cannot safely be recovered.
        if arg == "--no-proxy-server"
            || arg == "--proxy-auto-detect"
            || arg == "--proxy-pac-url"
            || arg.starts_with("--proxy-pac-url=")
        {
            return None;
        }
        let value = if arg == "--proxy-server" {
            Some(args.next()?.to_str()?)
        } else {
            arg.strip_prefix("--proxy-server=")
        };
        if let Some(value) = value {
            if port.is_some() {
                return None;
            }
            port = Some(managed_proxy_port(value)?);
        }
    }
    port
}

fn managed_proxy_port(value: &str) -> Option<u16> {
    // Cockpit injects exactly 127.0.0.1 and restores an IPv4-only listener.
    // Do not claim success for IPv6/localhost, remote hosts, credentials,
    // Chromium per-scheme mappings, fallback lists, or URL paths/queries.
    let authority = value
        .strip_prefix("socks5://")
        .or_else(|| value.strip_prefix("http://"))?;
    let (host, port) = authority.rsplit_once(':')?;
    if host != "127.0.0.1" || port.is_empty() || !port.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let port = port.parse::<u16>().ok()?;
    (40_000..60_000).contains(&port).then_some(port)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn reads_only_explicit_managed_loopback_endpoints() {
        for value in ["socks5://127.0.0.1:40000", "http://127.0.0.1:40000"] {
            assert_eq!(managed_proxy_port(value), Some(40000));
        }
        assert_eq!(managed_proxy_port("http://127.0.0.1:59999"), Some(59999));
        for value in [
            "socks5://127.0.0.1:39999",
            "socks5://127.0.0.1:60000",
            "socks5://127.0.0.1:65536",
            "http://localhost:41000",
            "socks5://[::1]:41000",
            "http://example.com:41000",
            "http://127.0.0.1.example.com:41000",
            "http://user:pass@127.0.0.1:41000",
            "http://127.0.0.1:41000/path",
            "http://127.0.0.1:41000?x=1",
            "http://127.0.0.1:41000#fragment",
            "http=127.0.0.1:41000;https=127.0.0.1:41001",
            "socks5://127.0.0.1:41000,direct://",
            "https://127.0.0.1:41000",
            "http://127.0.0.1:+41000",
            "http://127.0.0.1:",
        ] {
            assert_eq!(managed_proxy_port(value), None, "{value}");
        }
    }

    #[test]
    fn accepts_equals_and_separate_arguments() {
        assert_eq!(
            proxy_port_from_args(&args(&[
                "/Applications/Codex.app/Contents/MacOS/Codex",
                "--proxy-server=socks5://127.0.0.1:41000",
            ])),
            Some(41000)
        );
        assert_eq!(
            proxy_port_from_args(&args(&[
                r"C:\Program Files\Codex\ChatGPT.exe",
                "--proxy-server",
                "http://127.0.0.1:41000",
            ])),
            Some(41000)
        );
    }

    #[test]
    fn rejects_missing_duplicate_and_overriding_flags() {
        for tail in [
            vec![],
            vec!["--proxy-server"],
            vec!["--proxy-server-extra=socks5://127.0.0.1:41000"],
            vec!["--title=--proxy-server=socks5://127.0.0.1:41000"],
            vec![
                "--proxy-server=socks5://127.0.0.1:41000",
                "--no-proxy-server",
            ],
            vec![
                "--proxy-auto-detect",
                "--proxy-server=socks5://127.0.0.1:41000",
            ],
            vec![
                "--proxy-pac-url=http://localhost/pac",
                "--proxy-server=http://127.0.0.1:41000",
            ],
            vec![
                "--proxy-server=http://127.0.0.1:41000",
                "--proxy-server=http://127.0.0.1:41001",
            ],
        ] {
            let mut command = vec!["Codex"];
            command.extend(tail);
            assert_eq!(proxy_port_from_args(&args(&command)), None);
        }
    }

    #[test]
    fn keeps_default_and_managed_process_contexts_unchanged() {
        let command = args(&["Codex", "--proxy-server=socks5://127.0.0.1:41000"]);
        let default = entry_from_args(123, None, &command).unwrap();
        assert_eq!(default.profile_dir, None);
        assert_eq!(default.pid, 123);
        let profile = Some("/profiles/managed/.codex".to_string());
        let managed = entry_from_args(456, profile.clone(), &command).unwrap();
        assert_eq!(managed.profile_dir, profile);
        assert_eq!(managed.proxy_port, default.proxy_port);
    }
}
