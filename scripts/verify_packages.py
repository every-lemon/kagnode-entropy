#!/usr/bin/env python3
"""Inspect package integrity without installing or executing application code."""

import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import subprocess
import tarfile
import zipfile


def validate(folder):
    folder = Path(folder)
    manifest = json.loads((folder / "release-manifest.json").read_text())
    expected = manifest["assets"]
    names = {p.name for p in folder.iterdir() if p.is_file()}
    if names != set(expected) | {"release-manifest.json", "SHA256SUMS"}:
        raise ValueError("Unexpected or missing release files")
    checksum_lines = (folder / "SHA256SUMS").read_text().splitlines()
    checksum_names = [line.split("  ", 1)[1] for line in checksum_lines]
    if len(checksum_names) != len(set(checksum_names)) or set(
        checksum_names
    ) != names - {"SHA256SUMS"}:
        raise ValueError("Incomplete or duplicate checksum entries")
    for line in checksum_lines:
        sha, name = line.split("  ", 1)
        if (
            Path(name).name != name
            or hashlib.sha256((folder / name).read_bytes()).hexdigest() != sha
        ):
            raise ValueError("Checksum mismatch: " + name)
    for name, meta in expected.items():
        path = folder / name
        if (
            path.stat().st_size != meta["size"]
            or hashlib.sha256(path.read_bytes()).hexdigest() != meta["sha256"]
        ):
            raise ValueError("Manifest mismatch: " + name)
        if name.endswith((".tar.xz", ".tar.gz", ".zip")):
            if name.endswith(".zip"):
                with zipfile.ZipFile(path) as archive:
                    members = archive.infolist()
                    entries = [(x.filename, x.file_size) for x in members]
                    if any(
                        (x.external_attr >> 16) & 0o170000 == 0o120000 for x in members
                    ):
                        raise ValueError("Symlink in ZIP")
                    if archive.testzip():
                        raise ValueError("Invalid ZIP content")
            else:
                with tarfile.open(path) as archive:
                    members = archive.getmembers()
                    entries = [(x.name, x.size) for x in members]
                    if any(not x.isfile() for x in members):
                        raise ValueError("Non-regular TAR member")
            roots = set()
            seen = set()
            for entry, size in entries:
                rel = PurePosixPath(entry)
                if rel.is_absolute() or ".." in rel.parts or entry in seen or size < 0:
                    raise ValueError("Unsafe archive member")
                seen.add(entry)
                roots.add(rel.parts[0])
            if len(roots) != 1:
                raise ValueError("Archive must have one top-level directory")
        elif name.endswith(".deb"):
            subprocess.run(
                ["dpkg-deb", "--info", str(path)], check=True, stdout=subprocess.DEVNULL
            )
            subprocess.run(
                ["dpkg-deb", "--contents", str(path)],
                check=True,
                stdout=subprocess.DEVNULL,
            )
        elif name.endswith(".rpm"):
            subprocess.run(
                ["rpm", "-K", "--nosignature", str(path)],
                check=True,
                stdout=subprocess.DEVNULL,
            )
            owned = subprocess.check_output(
                ["rpm", "-qpl", str(path)], text=True, stderr=subprocess.DEVNULL
            ).splitlines()
            if any(
                name
                in {"/usr", "/usr/bin", "/usr/share", "/usr/share/doc", "/etc", "/var"}
                for name in owned
            ):
                raise ValueError("RPM must not own shared system directories")
            subprocess.run(
                ["rpm", "-qpl", str(path)],
                check=True,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
    print(f"{manifest['repository']}: {len(expected)} release payloads verified")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("folder", type=Path)
    validate(parser.parse_args().folder)
