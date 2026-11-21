use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use super::*;

fn effective_env(profile: &Path) -> HashMap<String, String> {
    // Codex applies each successfully parsed entry in order. Windows treats
    // environment variable names as case-insensitive.
    dotenvy::from_path_iter(profile.join(".env"))
        .unwrap()
        .flatten()
        .map(|(key, value)| (key.to_ascii_uppercase(), value))
        .collect()
}

fn mock_connect_proxy() -> (u16, Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    listener.set_nonblocking(true).unwrap();
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        let until = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < until {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut request = String::new();
                    BufReader::new(&stream).read_line(&mut request).unwrap();
                    let _ = send.send(request);
                    let _ =
                        stream.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n");
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("{error}"),
            }
        }
    });
    (port, receive)
}

fn request_through_profile(profile: &Path) {
    let env = effective_env(profile);
    let proxy = reqwest::Proxy::all(&env["HTTPS_PROXY"])
        .unwrap()
        .no_proxy(reqwest::NoProxy::from_string(&env["NO_PROXY"]));
    let result = reqwest::blocking::Client::builder()
        .proxy(proxy)
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap()
        .get("https://codex-proxy-test.invalid/")
        .send();
    assert!(result.is_err()); // The fake proxy records CONNECT and returns 502.
}

#[test]
fn managed_launch_uses_account_proxy_and_inherit_restores_original_proxy() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join(".env");
    let (original_port, original_proxy) = mock_connect_proxy();
    let (account_port, account_proxy) = mock_connect_proxy();
    let original = format!(
        "export HtTpS_pRoXy=http://127.0.0.1:{original_port}\r\nNO_PROXY=original.test\r\nKEEP='setting'"
    );
    std::fs::write(&path, &original).unwrap();

    launch(
        home.path(),
        &CodexLaunchProxy::Managed(account_port),
        || {
            request_through_profile(home.path());
            assert_eq!(effective_env(home.path())["KEEP"], "setting");
            Ok(42)
        },
    )
    .unwrap();
    assert_eq!(
        account_proxy.recv_timeout(Duration::from_secs(1)).unwrap(),
        "CONNECT codex-proxy-test.invalid:443 HTTP/1.1\r\n"
    );
    assert!(original_proxy.try_recv().is_err());

    launch(home.path(), &CodexLaunchProxy::Inherit, || {
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        request_through_profile(home.path());
        Ok(42)
    })
    .unwrap();
    assert!(original_proxy
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .starts_with("CONNECT codex-proxy-test.invalid:443 "));
}

#[test]
fn direct_launch_overrides_every_proxy_variable_and_removes_its_new_file_on_inherit() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join(".env");
    launch(home.path(), &CodexLaunchProxy::Direct, || {
        let env = effective_env(home.path());
        for key in ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"] {
            assert_eq!(env[key], "");
        }
        assert_eq!(env["NO_PROXY"], "*");
        Ok(42)
    })
    .unwrap();
    launch(home.path(), &CodexLaunchProxy::Inherit, || {
        assert!(!path.exists());
        Ok(42)
    })
    .unwrap();
}

#[test]
fn managed_relaunch_takes_precedence_over_subsequent_edits_and_retains_them_on_inherit() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join(".env");
    std::fs::write(&path, "KEEP=before\n").unwrap();
    launch(home.path(), &CodexLaunchProxy::Managed(12001), || Ok(42)).unwrap();
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    writeln!(file, "Https_Proxy=http://127.0.0.1:12003\nLOCAL=value").unwrap();
    drop(file);

    launch(home.path(), &CodexLaunchProxy::Managed(12002), || {
        let env = effective_env(home.path());
        assert_eq!(env["HTTPS_PROXY"], "http://127.0.0.1:12002");
        assert_eq!(env["LOCAL"], "value");
        Ok(42)
    })
    .unwrap();
    launch(home.path(), &CodexLaunchProxy::Inherit, || {
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "KEEP=before\nHttps_Proxy=http://127.0.0.1:12003\nLOCAL=value\n"
        );
        Ok(42)
    })
    .unwrap();
}

#[test]
fn unterminated_dotenv_value_does_not_silently_hide_managed_settings() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join(".env");
    let original = "KEEP='unterminated\n";
    std::fs::write(&path, original).unwrap();

    assert!(launch(home.path(), &CodexLaunchProxy::Managed(12001), || {
        panic!("must not start without the selected proxy")
    })
    .is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    assert_eq!(
        launch(home.path(), &CodexLaunchProxy::Inherit, || Ok(42)),
        Ok(42)
    );
}

#[test]
fn concurrent_edit_before_write_can_be_retried_without_losing_client_settings() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join(".env");
    let proxy = CodexLaunchProxy::Managed(12001);
    std::fs::write(&path, "KEEP=before\n").unwrap();
    let projection = Projection::prepare(home.path(), &proxy).unwrap().unwrap();
    std::fs::write(&path, "KEEP=after\n").unwrap();

    assert_eq!(projection.write(), Err(CHANGED.into()));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "KEEP=after\n");
    launch(home.path(), &proxy, || Ok(42)).unwrap();
    launch(home.path(), &CodexLaunchProxy::Inherit, || Ok(42)).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "KEEP=after\n");
}

#[test]
fn edit_during_launch_stops_the_reported_pid_preserves_the_edit_and_allows_retry() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join(".env");
    let proxy = CodexLaunchProxy::Managed(12001);
    let edited = "HTTPS_PROXY=http://127.0.0.1:12002\n";
    let mut stopped = None;
    let result = launch_with_stop(
        home.path(),
        &proxy,
        || {
            std::fs::write(&path, edited).unwrap();
            Ok(42)
        },
        |pid| {
            stopped = Some(pid);
            Ok(())
        },
    );
    assert_eq!(result, Err(CHANGED.into()));
    assert_eq!(stopped, Some(42));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);

    launch(home.path(), &proxy, || Ok(42)).unwrap();
    launch(home.path(), &CodexLaunchProxy::Inherit, || Ok(42)).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);
}
