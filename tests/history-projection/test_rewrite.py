"""Exercise extracted production Rust rewrite functions on synthetic files."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile

binary = Path(sys.argv[1]).resolve()
home = Path(tempfile.mkdtemp(prefix="cockpit-layout-regression-"))
count = 0
for mode in ("first", "full"):
    for ending in (b"\n", b"\r\n", b""):
        meta = {"type": "session_meta", "ordinal": 0, "payload": {
            "id": "synthetic", "model_provider": "synthetic-provider", "history_mode": "paginated"}}
        line = json.dumps(meta, separators=(",", ":")).encode()
        tail = b'{"ordinal":1,"type":"synthetic"}' + ending if ending else b""
        original = line + ending + tail
        path = home / (mode + "-" + str(len(ending)) + ".jsonl")
        path.write_bytes(original)
        subprocess.run([str(binary), mode, str(path), "openai"], check=True)
        changed = path.read_bytes()
        assert len(changed) == len(original)
        assert changed[len(line):] == original[len(line):]
        assert json.loads(changed.splitlines()[0])["payload"]["model_provider"] == "openai"
        subprocess.run([str(binary), mode, str(path), "synthetic-provider"], check=True)
        assert len(path.read_bytes()) == len(original)
        before = path.read_bytes()
        refused = subprocess.run([str(binary), mode, str(path), "x" * 190], capture_output=True)
        assert refused.returncode == 2 and path.read_bytes() == before
        count += 1

# Deep repair must not shift a later session_meta even if it has no history marker.
path = home / "multiple-meta.jsonl"
first = json.dumps(meta, separators=(",", ":")).encode()
later = b'{"type":"session_meta","payload":{"model_provider":"synthetic-provider"}}'
original = first + b"\n" + later + b"\n"
path.write_bytes(original)
subprocess.run([str(binary), "full", str(path), "openai"], check=True)
assert [len(x) for x in path.read_bytes().splitlines(True)] == [len(x) for x in original.splitlines(True)]

# Existing unindexed legacy migration remains available, including longer IDs.
path = home / "legacy.jsonl"
path.write_text('{"type":"session_meta","payload":{"model_provider":"a"}}\n')
subprocess.run([str(binary), "first", str(path), "longer-provider"], check=True)
assert json.loads(path.read_text())["payload"]["model_provider"] == "longer-provider"
print(f"PASS: {count + 2} production rewrite scenarios")
