use super::*;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

#[test]
#[ignore = "requires COCKPIT_TIER_TEST_CLI; sends only to a disposable loopback server"]
fn real_cli_sends_selected_tier_with_unified_managed_catalog() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::process::Stdio;
    let binary = PathBuf::from(std::env::var_os("COCKPIT_TIER_TEST_CLI").expect("test CLI path"));
    let root = std::env::temp_dir().join(format!("cockpit-tier-wire-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let export = root.join("export");
    std::fs::create_dir_all(&export).unwrap();
    let output = crate::modules::process_timeout::output_with_timeout(
        Command::new(&binary)
            .args(["debug", "models", "--bundled"])
            .env("CODEX_HOME", &export)
            .current_dir(&export),
        Duration::from_secs(4),
    )
    .unwrap();
    assert!(output.status.success());
    let mut catalog: Value = serde_json::from_slice(&output.stdout).unwrap();
    supplement(&mut catalog).unwrap();
    for tier in ["default", "priority", "ultrafast"] {
        let profile = root.join(tier);
        std::fs::create_dir_all(&profile).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let receive = std::thread::spawn(move || {
            let started = std::time::Instant::now();
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            started.elapsed() < Duration::from_secs(10),
                            "no native request"
                        );
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("accept: {error}"),
                }
            };
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut headers = Vec::new();
            while !headers.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                socket.read_exact(&mut byte).unwrap();
                headers.push(byte[0]);
                assert!(headers.len() < 65536);
            }
            let headers = String::from_utf8(headers).unwrap();
            assert!(headers.starts_with("POST /v1/responses "));
            let size: usize = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse().unwrap())
                })
                .unwrap();
            let mut body = vec![0; size];
            socket.read_exact(&mut body).unwrap();
            socket.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}").unwrap();
            serde_json::from_slice::<Value>(&body).unwrap()
        });
        std::fs::write(profile.join("config.toml"), format!(
                "model = 'gpt-6.1-sol'\nmodel_provider = 'probe'\nservice_tier = '{tier}'\nmodel_reasoning_effort = 'low'\n[features]\nfast_mode = true\n[model_providers.probe]\nname = 'Loopback test'\nbase_url = 'http://127.0.0.1:{port}/v1'\nwire_api = 'responses'\nrequires_openai_auth = false\n"
            )).unwrap();
        std::fs::write(
            profile.join(MANAGED_FILE),
            serde_json::to_string(&catalog).unwrap(),
        )
        .unwrap();
        let mut config = std::fs::read_to_string(profile.join("config.toml")).unwrap();
        config.insert_str(0, &format!("model_catalog_json = '{MANAGED_FILE}'\n"));
        std::fs::write(profile.join("config.toml"), config).unwrap();
        let mut command = Command::new(&binary);
        command
            .args([
                "exec",
                "--skip-git-repo-check",
                "--ephemeral",
                "--sandbox",
                "read-only",
                "--json",
                "Reply OK",
            ])
            .env("CODEX_HOME", &profile)
            .env("NO_PROXY", "127.0.0.1,localhost")
            .current_dir(&profile)
            .stdin(Stdio::null());
        crate::modules::process_timeout::output_with_timeout(&mut command, Duration::from_secs(12))
            .unwrap();
        let body = receive.join().unwrap();
        assert_eq!(body["model"], "gpt-6.1-sol");
        if tier == "default" {
            assert!(body.get("service_tier").is_none());
        } else {
            assert_eq!(body["service_tier"], tier);
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}
