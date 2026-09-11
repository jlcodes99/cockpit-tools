"""Compile the actual rollout rewrite functions without launching the desktop app.

Use existing Cargo dependency artifacts, read-only, to avoid a full Tauri build.
The extracted functions are not reimplemented or patched by this harness.
"""
import argparse
from pathlib import Path
import subprocess
import tempfile

parser = argparse.ArgumentParser()
parser.add_argument("--deps", required=True, type=Path)
parser.add_argument("--rustc", required=True, type=Path)
args = parser.parse_args()
root = Path(__file__).resolve().parents[2]
modules = root / "src-tauri/src/modules"
discovery = (modules / "codex_session_visibility_instance_discovery.rs").read_text()
functions = discovery[discovery.index("#[derive(Debug, Default)]\nstruct RolloutProviderRewrite"):
                      discovery.index("fn collect_rollout_thread_facts(")]
backup = (modules / "codex_session_visibility_backup.rs").read_text()
writers = backup[backup.index("fn rewrite_rollout_first_line("):backup.index("fn sqlite_candidate_paths(")]
directory = Path(tempfile.mkdtemp(prefix="cockpit-rewrite-fixture-"))
source = directory / "fixture.rs"
source.write_text('''
#![allow(dead_code)]
use std::{fs, path::Path, io::{BufRead, BufReader}, collections::HashSet};
use serde_json::Value as JsonValue;
use chrono::Utc;
mod modules { pub mod logger { pub fn log_warn(_: &str) {} } }
#[derive(Debug, Clone)]
enum RolloutProviderUpdate { FullContent(String), FirstLine(String) }
''' + '#[path = ' + '"' + str(modules / "codex_rollout_byte_layout.rs") + '"]\nmod rollout_byte_layout;\n'
    + functions + writers + '''
fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(args.len(), 4);
    let path = Path::new(&args[2]);
    let rewrite = if args[1] == "full" {
        rewrite_rollout_session_meta_providers(&fs::read_to_string(path).unwrap(), &args[3])
    } else {
        rewrite_rollout_first_session_meta_provider(path, &args[3])
    };
    match rewrite {
        Ok(value) => match value.updated_content {
            Some(RolloutProviderUpdate::FullContent(content)) => write_bytes_atomic(path, content.as_bytes()).unwrap(),
            Some(RolloutProviderUpdate::FirstLine(line)) => rewrite_rollout_first_line(path, &line).unwrap(),
            None => (),
        },
        Err(error) => { eprintln!("{error}"); std::process::exit(2); }
    }
}
''')
command = [str(args.rustc), "--edition=2021", str(source), "-L", "dependency=" + str(args.deps.resolve())]
for name in ("serde_json", "chrono"):
    library = sorted(args.deps.glob("lib" + name + "-*.rlib"))[0].resolve()
    command += ["--extern", name + "=" + str(library)]
binary = directory / "rewrite-fixture"
subprocess.run(command + ["-o", str(binary)], check=True)
print(binary)
