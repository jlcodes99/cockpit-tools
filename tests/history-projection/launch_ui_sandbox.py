"""Launch the lab with a fresh HOME and an OS-enforced runtime safety boundary."""
import argparse
import json
import os
import plistlib
import shutil
from pathlib import Path
import socket
import sqlite3
import subprocess
import tempfile
import time


def launch(codex_home):
    root = Path(__file__).resolve().parents[2]
    binary = root / "target/debug/cockpit-tools"
    codex_home = codex_home.resolve()
    if (codex_home / ".synthetic-history-fixture").read_text() != "synthetic-only-v1\n":
        raise ValueError("A synthetic fixture is required")
    with sqlite3.connect((codex_home / "state_5.sqlite").as_uri() + "?mode=ro", uri=True) as db:
        for (path,) in db.execute("SELECT rollout_path FROM threads"):
            if not Path(path).resolve().is_relative_to(codex_home):
                raise ValueError("Rollout path escaped the fixture")
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 1420))
    lab = Path(tempfile.mkdtemp(prefix="cockpit-ui-sandbox-", dir="/private/tmp"))
    home, data = lab / "home", lab / "data"
    home.mkdir()
    data.mkdir()
    bundle = lab / "Cockpit History Lab.app" / "Contents"
    (bundle / "MacOS").mkdir(parents=True)
    (bundle / "Info.plist").write_bytes(plistlib.dumps({
        "CFBundleName": "Cockpit History Lab", "CFBundleDisplayName": "Cockpit History Lab",
        "CFBundleIdentifier": "com.jlcodes.cockpit-tools.history-lab",
        "CFBundleExecutable": "cockpit-tools", "CFBundlePackageType": "APPL",
        "CFBundleVersion": "1", "CFBundleShortVersionString": "1.3.43",
        "NSHighResolutionCapable": True,
    }))
    bundled_binary = bundle / "MacOS/cockpit-tools"
    shutil.copy2(binary, bundled_binary)
    subprocess.run(["/usr/bin/codesign", "--force", "--sign", "-", str(bundle.parent)], check=True, capture_output=True)
    (data / "config.json").write_text(json.dumps({
        "ws_enabled": False, "report_enabled": False,
        "codex_auto_refresh_minutes": 0, "codex_launch_on_switch": False,
        "codex_app_ui_injection_enabled": False,
        "codex_auto_restore_takeover_on_launch": False,
        "codex_startup_wakeup_enabled": False,
    }))
    real_home = str(Path.home())
    protected = [str(Path(real_home) / suffix) for suffix in (
        ".codex", ".antigravity_cockpit", ".antigravity_cockpit_dev", ".cc-switch", "Library/Keychains",
    )]
    quote = json.dumps
    policy = "\n".join([
        "(version 1)", "(allow default)",
        f"(deny file-write* (subpath {quote(real_home)}))",
        "(deny file-read* " + " ".join(f"(subpath {quote(p)})" for p in protected) + ")",
        "(deny network*)",
        '(allow network-outbound (remote tcp "localhost:1420"))',
        "(deny signal)",
        "(deny process-exec)",
        f"(allow process-exec (literal {quote(str(bundled_binary))}))",
    ])
    (lab / "runtime.sb").write_text(policy)
    env = {"PATH": "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin",
           "HOME": str(home), "CODEX_HOME": str(codex_home),
           "COCKPIT_TOOLS_PROFILE": "dev", "VITE_COCKPIT_TOOLS_PROFILE": "dev",
           "COCKPIT_TOOLS_TEST_DATA_DIR": str(data), "COCKPIT_TOOLS_DATA_DIR": str(data),
           "TMPDIR": str(lab), "XDG_CACHE_HOME": str(home / ".cache")}
    with (lab / "vite.log").open("w") as log:
        vite = subprocess.Popen(["/usr/local/bin/node", str(root / "node_modules/vite/bin/vite.js"),
                                 "--host", "127.0.0.1", "--port", "1420", "--strictPort"],
                                cwd=root, env=env, stdout=log, stderr=log, start_new_session=True)
    try:
        for _ in range(50):
            if vite.poll() is not None:
                raise RuntimeError("Vite failed: " + (lab / "vite.log").read_text())
            try:
                with socket.create_connection(("127.0.0.1", 1420), timeout=0.1):
                    break
            except OSError:
                time.sleep(0.1)
        else:
            raise RuntimeError("Vite did not become ready")
        with (lab / "app.log").open("w") as log:
            app = subprocess.Popen(["/usr/bin/sandbox-exec", "-f", str(lab / "runtime.sb"), str(bundled_binary)],
                                   cwd=lab, env=env, stdout=log, stderr=log, start_new_session=True)
        time.sleep(3)
        if app.poll() is not None:
            raise RuntimeError("Sandbox app failed: " + (lab / "app.log").read_text()[-3000:])
    except Exception:
        vite.terminate()
        vite.wait(timeout=10)
        raise
    result = {"lab": str(lab), "bundle": str(bundle.parent), "app_pid": app.pid, "vite_pid": vite.pid,
              "codex_home": str(codex_home), "data_dir": str(data), "url": "http://127.0.0.1:1420"}
    (lab / "launch.json").write_text(json.dumps(result, indent=2))
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--codex-home", required=True, type=Path)
    print(json.dumps(launch(parser.parse_args().codex_home), indent=2))
