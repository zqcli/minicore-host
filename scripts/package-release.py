#!/usr/bin/env python3
"""Smoke-test and package the native Host release built by CI."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tempfile
from urllib.parse import quote, urlsplit
import zipfile


def packaged_document(source, root, included, source_url):
    def replace(match):
        destination = match.group(1)
        url = urlsplit(destination)
        if url.scheme or url.netloc or not url.path:
            return match.group(0)
        target = (source.parent / url.path).resolve()
        if target in included:
            return match.group(0)
        if not target.exists():
            raise RuntimeError(f"broken documentation link in {source}: {destination}")
        relative = quote(target.relative_to(root).as_posix(), safe="/")
        kind = "tree" if target.is_dir() else "blob"
        link = f"{source_url[0]}/{kind}/{source_url[1]}/{relative}"
        if url.fragment:
            link += f"#{url.fragment}"
        return f"]({link})"

    return re.sub(r"\]\(([^)\s]+)\)", replace, source.read_text(encoding="utf-8"))


def sha256(stream):
    digest = hashlib.sha256()
    for chunk in iter(lambda: stream.read(1024 * 1024), b""):
        digest.update(chunk)
    return digest.hexdigest()


def verify_archive(archive, stage, executable, binary_sha256):
    expected = {
        source.relative_to(stage.parent).as_posix()
        for source in stage.rglob("*") if source.is_file()
    }
    binary_name = f"{stage.name}/bin/{executable}"

    def verify_files(names):
        if len(names) != len(set(names)) or set(names) != expected:
            raise RuntimeError("archive file set differs from the actual package stage")
        executables = {
            name for name in names
            if name.startswith(f"{stage.name}/bin/") or name.lower().endswith(".exe")
        }
        if executables != {binary_name}:
            raise RuntimeError("archive must contain only the Host executable in bin")

    if archive.suffix == ".zip":
        with zipfile.ZipFile(archive, "r") as bundle:
            verify_files([member.filename for member in bundle.infolist() if not member.is_dir()])
            with bundle.open(binary_name) as stream:
                actual_sha256 = sha256(stream)
    else:
        with tarfile.open(archive, "r:gz") as bundle:
            members = bundle.getmembers()
            if any(not member.isdir() and not member.isfile() for member in members):
                raise RuntimeError("archive contains a link or another non-regular entry")
            files = [member for member in members if member.isfile()]
            verify_files([member.name for member in files])
            binary_member = bundle.getmember(binary_name)
            if binary_member.mode != 0o755:
                raise RuntimeError("archived Host executable must have mode 0755")
            if {member.name for member in files if member.mode & 0o111} != {binary_name}:
                raise RuntimeError("archive contains another executable file")
            with bundle.extractfile(binary_member) as stream:
                actual_sha256 = sha256(stream)
    if actual_sha256 != binary_sha256:
        raise RuntimeError("archived Host differs from the smoke-tested release binary")
    print(f"Archive verified: {len(expected)} files; Host SHA256: {binary_sha256}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("target")
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"],
        cwd=root,
        text=True,
    ))
    package = next(value for value in metadata["packages"] if value["name"] == "minicore-host")
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()
    repository = os.environ.get("GITHUB_REPOSITORY", "zqcli/minicore-host")
    source_url = (f"https://github.com/{repository}", revision)
    target_directory = Path(metadata["target_directory"])
    executable = "minicore-host.exe" if args.target.endswith("windows-msvc") else "minicore-host"
    binary = target_directory / args.target / "release" / executable
    smoke = subprocess.run(
        [str(binary), "--help"],
        check=True,
        capture_output=True,
        text=True,
        timeout=30,
    )
    if smoke.stdout or "Usage: minicore-host" not in smoke.stderr:
        raise RuntimeError("release --help must succeed on stderr without protocol stdout")
    with binary.open("rb") as stream:
        binary_sha256 = sha256(stream)

    name = f"minicore-host-{package['version']}-{args.target}"
    output = target_directory / "packages"
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="minicore-host-package-") as temporary:
        stage = Path(temporary) / name
        (stage / "bin").mkdir(parents=True)
        installed = stage / "bin" / executable
        shutil.copy2(binary, installed)
        if not executable.endswith(".exe"):
            installed.chmod(0o755)
        documents = [root / filename for filename in (
            "README.md", "SPEC.md", "PROJECT.md", "example.host.json", "docs/tool-approval.md",
        )]
        documents.extend(sorted((root / "crates").glob("*/README.md")))
        included = set(documents)
        for source in documents:
            destination = stage / source.relative_to(root)
            destination.parent.mkdir(parents=True, exist_ok=True)
            if source.suffix == ".md":
                destination.write_text(
                    packaged_document(source, root, included, source_url), encoding="utf-8",
                )
            else:
                shutil.copy2(source, destination)

        if executable.endswith(".exe"):
            archive = output / f"{name}.zip"
            with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as bundle:
                for source in sorted(stage.rglob("*")):
                    if source.is_file():
                        bundle.write(source, source.relative_to(stage.parent))
        else:
            archive = output / f"{name}.tar.gz"
            with tarfile.open(archive, "w:gz") as bundle:
                bundle.add(stage, arcname=name)
        verify_archive(archive, stage, executable, binary_sha256)

    with archive.open("rb") as stream:
        archive_sha256 = sha256(stream)
    print(archive)
    print(f"Archive SHA256: {archive_sha256}  {archive.name}")
    github_output = os.environ.get("GITHUB_OUTPUT")
    if github_output:
        with open(github_output, "a", encoding="utf-8") as outputs:
            outputs.write(f"name={name}\npath={archive.as_posix()}\n")


if __name__ == "__main__":
    main()
