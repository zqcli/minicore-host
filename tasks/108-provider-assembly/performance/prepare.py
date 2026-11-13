#!/usr/bin/env python3
"""Build the identical Task108 probe against an exact, unchanged source archive.

Run through the chosen Rust 1.90.0 environment. Only a new example is added to
the extracted source; no manifest, decoder, assembler, or lockfile is edited.
Preparation/builds may precede publication. Formal measurement must follow it.
"""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import time


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--target-dir", type=Path)
    args = parser.parse_args()
    rustc = subprocess.check_output(["rustc", "--version", "--verbose"], text=True)
    if not rustc.startswith("rustc 1.90.0 "):
        raise RuntimeError("the comparison requires Rust 1.90.0")
    root = args.root.resolve()
    revision = subprocess.check_output(
        ["git", "-C", str(root), "rev-parse", "--verify", args.revision + "^{commit}"], text=True
    ).strip()
    args.out.mkdir(parents=True, exist_ok=False)
    output = args.out.resolve()
    archive = subprocess.check_output(["git", "-C", str(root), "archive", revision])
    source = output / "source"
    source.mkdir()
    with tarfile.open(fileobj=io.BytesIO(archive)) as bundle:
        bundle.extractall(source, filter="data")
    production_files = sorted(
        path for crate in ("host-provider", "host-runtime")
        for path in (source / "crates" / crate / "src").rglob("*.rs")
    )
    hashes_before = {str(path.relative_to(source)): digest(path) for path in production_files}
    canonical_probe = Path(__file__).with_name("provider_assembly.rs")
    example = source / "crates/host-provider/examples/task108_provider_assembly.rs"
    example.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(canonical_probe, example)
    lock_sha256 = digest(source / "Cargo.lock")
    target = (args.target_dir or output / "target").resolve()
    command = ["cargo", "build", "--release", "--locked", "--offline", "--manifest-path",
               str(source / "Cargo.toml"), "--target-dir", str(target), "-p",
               "minicore-host-provider", "--example", "task108_provider_assembly"]
    started = time.perf_counter_ns()
    result = subprocess.run(command, text=True, capture_output=True)
    elapsed = time.perf_counter_ns() - started
    (output / "build.stdout").write_text(result.stdout)
    (output / "build.stderr").write_text(result.stderr)
    record = {
        "source_revision": revision,
        "source_archive_sha256": hashlib.sha256(archive).hexdigest(),
        "source_root": str(root), "extracted_source": str(source),
        "source_sha256": hashes_before,
        "probe_sha256": digest(canonical_probe), "compiled_probe_sha256": digest(example),
        "prepare_sha256": digest(__file__), "cargo_lock_sha256": lock_sha256,
        "rustc": rustc, "build_command": command, "build_duration_ns": elapsed,
        "build_exit_code": result.returncode,
        "build_environment": {key: os.environ.get(key) for key in (
            "RUSTUP_TOOLCHAIN", "CARGO_BUILD_JOBS", "CARGO_INCREMENTAL", "RUSTFLAGS",
            "CARGO_PROFILE_RELEASE_OPT_LEVEL", "CARGO_PROFILE_RELEASE_LTO")},
        "fixture_scope": "real source SSE/parser/decoder plus real Runtime ModelDriver; memory byte source; no HTTP/live service",
    }
    if result.returncode == 0:
        assert hashes_before == {str(path.relative_to(source)): digest(path) for path in production_files}
        assert digest(source / "Cargo.lock") == lock_sha256
        binary = output / "task108_provider_assembly"
        shutil.copy2(target / "release/examples/task108_provider_assembly", binary)
        record.update(binary=str(binary), binary_sha256=digest(binary), production_source_unchanged=True)
    (output / "build.json").write_text(json.dumps(record, indent=2) + "\n")
    print(json.dumps({"build_record": str(output / "build.json"), "exit_code": result.returncode,
                      "build_duration_ns": elapsed}), flush=True)
    if result.returncode:
        print(result.stderr)
        raise SystemExit(result.returncode)


if __name__ == "__main__":
    main()
