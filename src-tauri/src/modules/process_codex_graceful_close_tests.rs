use super::request_codex_graceful_close;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct FixtureProcess(Child);

impl Drop for FixtureProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn native_graceful_close_rejects_pid_overflow() {
    assert!(!request_codex_graceful_close(i32::MAX as u32 + 1));
    assert!(!request_codex_graceful_close(u32::MAX));
    assert!(request_codex_graceful_close(0));
}

#[test]
fn native_graceful_close_leaves_non_application_process_running() {
    let mut process = FixtureProcess(
        Command::new("/bin/sleep")
            .arg("60")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start independent non-application fixture"),
    );
    assert!(!request_codex_graceful_close(process.0.id()));
    assert!(process.0.try_wait().unwrap().is_none());
    process.0.kill().unwrap();
    process.0.wait().unwrap();
    assert!(request_codex_graceful_close(process.0.id()));
}

struct FixtureDirectory(PathBuf);

impl Drop for FixtureDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn wait_for_fixture_ready(child: &mut Child, ready: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready.exists() {
        assert!(child.try_wait().unwrap().is_none(), "fixture exited early");
        assert!(Instant::now() < deadline, "fixture launch timed out");
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
#[ignore = "explicitly launches and terminates two test-only windowless AppKit processes"]
fn native_graceful_close_targets_only_windowless_fixture_pid() {
    let directory = FixtureDirectory(std::env::temp_dir().join(format!(
        "cockpit-native-close-fixture-{}",
        uuid::Uuid::new_v4()
    )));
    let contents = directory.0.join("CloseFixture.app/Contents");
    let executable = contents.join("MacOS/CloseFixture");
    std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
    std::fs::write(
        contents.join("Info.plist"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>CloseFixture</string>
<key>CFBundleIdentifier</key><string>tools.cockpit.test.native-close-fixture</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>LSBackgroundOnly</key><true/>
</dict></plist>"#,
    )
    .unwrap();
    let source = directory.0.join("fixture.m");
    std::fs::write(
        &source,
        r#"#import <AppKit/AppKit.h>
@interface CloseFixtureDelegate : NSObject <NSApplicationDelegate>
@property(copy) NSString *readyPath;
@property(copy) NSString *terminatedPath;
@end
@implementation CloseFixtureDelegate
- (void)applicationDidFinishLaunching:(NSNotification *)notification {
    [@"ready" writeToFile:self.readyPath atomically:YES encoding:NSUTF8StringEncoding error:nil];
}
- (NSApplicationTerminateReply)applicationShouldTerminate:(NSApplication *)sender {
    [@"graceful" writeToFile:self.terminatedPath atomically:YES encoding:NSUTF8StringEncoding error:nil];
    return NSTerminateNow;
}
@end
int main(int argc, const char *argv[]) {
    @autoreleasepool {
        NSApplication *application = [NSApplication sharedApplication];
        [application setActivationPolicy:NSApplicationActivationPolicyProhibited];
        CloseFixtureDelegate *delegate = [CloseFixtureDelegate new];
        delegate.readyPath = [NSString stringWithUTF8String:argv[1]];
        delegate.terminatedPath = [NSString stringWithUTF8String:argv[2]];
        application.delegate = delegate;
        [application run];
    }
    return 0;
}"#,
    )
    .unwrap();
    let compilation = Command::new("/usr/bin/clang")
        .args(["-fobjc-arc", "-framework", "AppKit"])
        .arg(&source)
        .arg("-o")
        .arg(&executable)
        .output()
        .expect("compile test-only AppKit fixture");
    assert!(
        compilation.status.success(),
        "fixture compilation failed: {}",
        String::from_utf8_lossy(&compilation.stderr)
    );
    let launch = |name: &str| {
        let ready = directory.0.join(format!("{name}.ready"));
        let terminated = directory.0.join(format!("{name}.terminated"));
        let mut process = FixtureProcess(
            Command::new(&executable)
                .arg(&ready)
                .arg(&terminated)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("start windowless test-only fixture"),
        );
        wait_for_fixture_ready(&mut process.0, &ready);
        (process, terminated)
    };
    let (mut target, target_marker) = launch("target");
    let (mut control, control_marker) = launch("control");
    let target_pid = target.0.id();
    let requested = std::thread::spawn(move || request_codex_graceful_close(target_pid))
        .join()
        .unwrap();
    assert!(requested, "native terminate did not accept fixture PID");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = target.0.try_wait().unwrap() {
            assert!(status.success(), "target did not quit normally");
            break;
        }
        assert!(Instant::now() < deadline, "target did not exit");
        std::thread::sleep(Duration::from_millis(25));
    }
    assert_eq!(std::fs::read_to_string(target_marker).unwrap(), "graceful");
    assert!(control.0.try_wait().unwrap().is_none());
    assert!(!control_marker.exists(), "quit leaked to the control app");
}
