"""Compile the actual Cockpit recovery module; no reimplementation of the recovery path."""
import argparse
from pathlib import Path
import subprocess
import tempfile

parser = argparse.ArgumentParser()
parser.add_argument("--deps", type=Path, required=True)
parser.add_argument("--rustc", type=Path, required=True)
args = parser.parse_args()
root = Path(__file__).resolve().parents[2]
directory = Path(tempfile.mkdtemp(prefix="cockpit-health-fixture-"))
source = directory / "main.rs"
source.write_text('''
#![allow(dead_code)]
#[path = "''' + str(root / "src-tauri/src/modules/codex_history_health.rs") + '''"]
mod health;
fn main() {
    let a: Vec<_> = std::env::args().collect();
    let home = std::path::Path::new(&a[1]);
    assert_eq!(std::fs::read_to_string(home.join(".synthetic-history-fixture")).unwrap(), "synthetic-only-v1\\n");
    let report = health::inspect_thread(home, &a[2]).unwrap();
    if a.len() == 3 { println!("{}", serde_json::to_string(&report).unwrap()); return; }
    let result = health::create_recovery_copy(home, &a[2], &report.source_sha256, std::path::Path::new(&a[3])).unwrap();
    println!("{}", serde_json::to_string(&result).unwrap());
}
''')
cmd = [str(args.rustc), "--edition=2021", str(source), "-L", "dependency=" + str(args.deps.resolve())]
for name in ("serde", "serde_json", "rusqlite", "sha2", "uuid"):
    libraries = sorted(args.deps.glob("lib" + name + "-*.rlib"), key=lambda p: p.stat().st_mtime, reverse=True)
    cmd += ["--extern", name + "=" + str(libraries[0].resolve())]
binary = directory / "health-fixture"
subprocess.run(cmd + ["-o", str(binary)], check=True)
print(binary)
