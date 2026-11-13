#!/usr/bin/env python3
"""Run only extracted pure credential parsers. Caller owns the shared lock."""
import hashlib
import os
from pathlib import Path
import subprocess
import tempfile
import time

ROOT = Path("/home/christian/Documents/dev/mountd")
EXPECTED = "460fdeedc8bd07c5cc452f726e1bd6bab23de718dac98d234f45b2e459fe049d"


def sha(data):
    return hashlib.sha256(data).hexdigest()


def extract(source, name):
    start = source.index(f"fn {name}(".encode())
    end = source.index(b"\n}\n", start) + 3
    return source[start:end]


def main():
    start = time.monotonic()
    source = (ROOT / "src/cred.rs").read_bytes()
    assert sha(source) == EXPECTED, "production source changed; stop review"
    baseline = subprocess.check_output(
        ["git", "show", "171eab5:src/cred.rs"], cwd=ROOT, timeout=10
    )
    fixture = Path(__file__).with_suffix(".rs").read_bytes()
    names = ["looks_like_hex", "decode_hex", "parse_password_metadata"]
    parts = [extract(source, name) for name in names]
    for name, part in zip(names, parts):
        print(f"production {name} SHA256 {sha(part)}", flush=True)
    # Restore just the two changed parser decisions, and verify the old body.
    old_parser = parts[2].replace(
        b'let mut records = stderr\n        .lines()\n        .filter_map(|line| line.trim_start().strip_prefix("password:"));\n    let rest = records\n        .next()',
        b'let rest = stderr\n        .lines()\n        .find_map(|line| line.trim_start().strip_prefix("password:"))',
        1,
    ).replace(
        b'    if records.next().is_some() {\n        return Err("ambiguous password metadata");\n    }\n',
        b"", 1,
    ).replace(
        b'''rest
        .strip_prefix('"')
        .and_then(|literal| literal.strip_suffix('"'))
        .is_some()
    {''',
        b'''rest.starts_with('"') && rest.ends_with('"') {''', 1,
    )
    assert old_parser == extract(baseline, "parse_password_metadata")
    assert parts[0] == extract(baseline, names[0])
    assert parts[1] == extract(baseline, names[1])
    unit = b"\n".join(parts)
    unit += b"\nmod old {\nuse super::decode_hex;\n" + old_parser
    unit += b"\npub(super) fn parse(s: &str) -> Result<Option<String>, &'static str> { parse_password_metadata(s) }\n}\n"
    unit += fixture
    scratch = Path(tempfile.mkdtemp(prefix="mountd-round2-credential-review-", dir=os.environ["TMPDIR"]))
    generated = scratch / "parser.rs"
    binary = scratch / "parser-tests"
    generated.write_bytes(unit)
    print(f"source SHA256 {sha(source)}", flush=True)
    print(f"fixture SHA256 {sha(fixture)}", flush=True)
    print(f"runner SHA256 {sha(Path(__file__).read_bytes())}", flush=True)
    print(f"old parser SHA256 {sha(old_parser)}", flush=True)
    print(f"generated SHA256 {sha(unit)}", flush=True)
    print(f"retained scratch {scratch}", flush=True)
    for command in [
        ["rustc", "--edition=2021", "--test", str(generated), "-o", str(binary)],
        [str(binary), "--test-threads=1", "--nocapture"],
    ]:
        print(f"argv {command!r}", flush=True)
        result = subprocess.run(command, cwd=ROOT, timeout=60, check=False)
        print(f"exit {result.returncode}", flush=True)
        assert result.returncode == 0
    assert (ROOT / "src/cred.rs").read_bytes() == source
    print(f"unchanged source confirmed; elapsed {time.monotonic() - start:.3f}s", flush=True)


if __name__ == "__main__":
    main()
