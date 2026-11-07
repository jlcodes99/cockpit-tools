use super::*;
use std::{
    io::Write,
    net::{Ipv4Addr, SocketAddrV4, TcpListener},
    process::Stdio,
    time::Duration,
};
use tokio::io::{AsyncBufReadExt, BufReader};

const FIXTURE_MODE: &str = "COCKPIT_MIHOMO_PROCESS_TEST_MODE";
const FIXTURE_PREFIX: &str = "COCKPIT_MIHOMO_TEST_LISTENER=";
const DEADLINE: Duration = Duration::from_secs(10);
const FIXTURE_MAX_LIFETIME: Duration = Duration::from_secs(30);

fn fixture_command(mode: &str) -> Command {
    // libtest names omit the crate component of module_path!().
    let (_, module) = module_path!().split_once("::").unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            &format!("{module}::lifetime_process_fixture"),
            "--nocapture",
        ])
        .env(FIXTURE_MODE, mode)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .creation_flags(0x08000000);
    command
}

async fn listener_port(child: &mut Child) -> u16 {
    let stdout = child.stdout.take().expect("fixture stdout");
    let mut lines = BufReader::new(stdout).lines();
    tokio::time::timeout(DEADLINE, async {
        while let Some(line) = lines.next_line().await.expect("read fixture stdout") {
            if let Some((_, port)) = line.split_once(FIXTURE_PREFIX) {
                return port.trim().parse().expect("fixture port");
            }
        }
        panic!("fixture exited before binding its listener");
    })
    .await
    .expect("fixture startup timed out")
}

fn listener_is_bound(port: u16) -> bool {
    TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)).is_err()
}

async fn assert_listener_released(port: u16) {
    tokio::time::timeout(DEADLINE, async {
        while listener_is_bound(port) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("owned child retained its listening port");
}

// Runs only when explicitly invoked in a subprocess. The listener deliberately
// ignores stdin and has no cleanup handler, so success depends on Job Objects.
#[test]
fn lifetime_process_fixture() {
    let Ok(mode) = std::env::var(FIXTURE_MODE) else {
        return;
    };
    if mode == "listener" {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        println!("{FIXTURE_PREFIX}{}", listener.local_addr().unwrap().port());
        std::io::stdout().flush().unwrap();
        // Bound cleanup even when the lifetime regression being tested fails.
        std::thread::sleep(FIXTURE_MAX_LIFETIME);
        return;
    }
    assert!(matches!(mode.as_str(), "owner-exit" | "owner-wait"));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let (mut child, _lifetime) = spawn(&mut fixture_command("listener")).unwrap();
        let port = listener_port(&mut child).await;
        println!("{FIXTURE_PREFIX}{port}");
        std::io::stdout().flush().unwrap();
        if mode == "owner-exit" {
            // process::exit skips every Rust Drop, as the Windows updater does.
            std::process::exit(0);
        }
        tokio::time::sleep(FIXTURE_MAX_LIFETIME).await;
    });
}

#[tokio::test]
async fn dropping_one_job_releases_only_its_child_port() {
    let (mut owned, lifetime) = spawn(&mut fixture_command("listener")).unwrap();
    let owned_port = listener_port(&mut owned).await;
    let (mut other_tunnel, other_lifetime) = spawn(&mut fixture_command("listener")).unwrap();
    let other_port = listener_port(&mut other_tunnel).await;
    let mut independent = fixture_command("listener").spawn().unwrap();
    let independent_port = listener_port(&mut independent).await;
    assert!(listener_is_bound(owned_port));

    drop(lifetime);

    assert_listener_released(owned_port).await;
    tokio::time::timeout(DEADLINE, owned.wait())
        .await
        .expect("owned child did not terminate")
        .unwrap();
    assert!(other_tunnel.try_wait().unwrap().is_none());
    assert!(listener_is_bound(other_port));
    assert!(independent.try_wait().unwrap().is_none());
    assert!(listener_is_bound(independent_port));
    drop(other_lifetime);
    tokio::time::timeout(DEADLINE, other_tunnel.wait())
        .await
        .expect("second tunnel did not terminate")
        .unwrap();
    independent.kill().await.unwrap();
}

#[tokio::test]
async fn direct_host_exit_releases_child_port_without_rust_drop() {
    let mut owner = fixture_command("owner-exit").spawn().unwrap();
    let port = listener_port(&mut owner).await;
    let status = tokio::time::timeout(DEADLINE, owner.wait())
        .await
        .expect("owner process did not exit")
        .unwrap();
    assert!(status.success());
    assert_listener_released(port).await;
}

#[tokio::test]
async fn forcibly_terminated_host_releases_child_port() {
    let mut owner = fixture_command("owner-wait").spawn().unwrap();
    let port = listener_port(&mut owner).await;
    assert!(listener_is_bound(port));

    owner.kill().await.unwrap();

    assert_listener_released(port).await;
}

#[tokio::test]
async fn cancelled_probe_releases_child_port_without_child_kill_on_drop() {
    let (started, port) = tokio::sync::oneshot::channel();
    let probe = tokio::spawn(async move {
        let mut command = fixture_command("listener");
        command.kill_on_drop(false);
        let (mut child, _lifetime) = spawn(&mut command).unwrap();
        started.send(listener_port(&mut child).await).unwrap();
        child.wait().await.unwrap();
    });
    let port = tokio::time::timeout(DEADLINE, port)
        .await
        .expect("probe startup timed out")
        .unwrap();
    assert!(listener_is_bound(port));

    probe.abort();
    assert!(probe.await.unwrap_err().is_cancelled());

    assert_listener_released(port).await;
}

#[tokio::test]
async fn short_lived_probe_keeps_output_and_exit_status() {
    let mut command = Command::new("cmd.exe");
    command
        .args(["/D", "/C", "echo probe-output"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let (child, _lifetime) = spawn(&mut command).unwrap();
    let output = tokio::time::timeout(DEADLINE, child.wait_with_output())
        .await
        .expect("version probe timed out")
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "probe-output"
    );
}
