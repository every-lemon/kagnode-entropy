#!/usr/bin/env python3
"""Isolated package lifecycle checks; never starts application/native services."""

import argparse
from pathlib import Path
import subprocess
import json
import tempfile
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from release import system_packages, config


def smoke(folder, engine):
    folder = Path(folder).resolve()
    with tempfile.TemporaryDirectory(prefix="kagnode-package-upgrade-") as temporary:
        return smoke_with_previous(folder, engine, Path(temporary))


def smoke_with_previous(folder, engine, temporary):
    manifest = json.loads((folder / "release-manifest.json").read_text())
    old_version = (
        config().get("upgrade_fixture_version")
        or manifest["version"].rsplit(".", 1)[0] + ".0"
    )
    previous = temporary / "previous"
    previous.mkdir()
    for package in sorted(folder.glob("*.deb")):
        if not package.name.endswith(("_all.deb", "_amd64.deb")):
            continue
        name = subprocess.check_output(
            ["dpkg-deb", "-f", str(package), "Package"], text=True
        ).strip()
        root = temporary / name
        subprocess.run(["dpkg-deb", "--extract", str(package), str(root)], check=True)
        binary = (root / "usr/bin/kagnode").is_file()
        if binary:
            docs = root / "usr/share/doc" / name
            files = [("kagnode", root / "usr/bin/kagnode")] + [
                (str(p.relative_to(docs)), p) for p in docs.rglob("*") if p.is_file()
            ]
        else:
            base = root / "usr/share/kagnode" / name
            files = [
                (str(p.relative_to(base)), p) for p in base.rglob("*") if p.is_file()
            ]
        system_packages(
            files,
            name,
            old_version,
            previous,
            manifest["epoch"],
            arch="amd64" if binary else "all",
            binary=binary,
        )
    for suffix, image, installer in [
        (".deb", "docker.io/library/ubuntu:24.04", "deb"),
        (".rpm", "docker.io/rockylinux/rockylinux:10", "rpm"),
    ]:
        packages = sorted(
            p.name
            for p in folder.glob("*" + suffix)
            if p.name.endswith(("_all" + suffix, "_amd64" + suffix))
        )
        if not packages:
            raise ValueError("No host-compatible packages")
        # Fixture is deliberately outside package-owned source paths.
        script = r"""set -eu
mkdir -p /var/lib/kagnode-fixture
printf 'retained-user-data\n' > /var/lib/kagnode-fixture/keep
for file do
  old=$(printf "%s" "$file" | sed "s/$release_version/$previous_version/")
  if [ "$kind" = deb ]; then
    name=$(dpkg-deb -f "/packages/$file" Package)
    dpkg -i "/previous/$old"
    dpkg -i "/packages/$file"
    dpkg -i "/packages/$file"
    dpkg --remove "$name"
    dpkg --purge "$name"
  else
    name=$(rpm -qp --qf '%{NAME}' "/packages/$file")
    rpm -Uvh "/previous/$old"
    rpm -Uvh "/packages/$file"
    rpm -Uvh --replacepkgs "/packages/$file"
    rpm -e "$name"
  fi
  test "$(cat /var/lib/kagnode-fixture/keep)" = retained-user-data
done
"""
        subprocess.run(
            [
                engine,
                "run",
                "--rm",
                "--network=none",
                "--memory=512m",
                "--pids-limit=128",
                "-v",
                str(folder) + ":/packages:ro",
                "-v",
                str(previous) + ":/previous:ro",
                "-e",
                "kind=" + installer,
                "-e",
                "release_version=" + manifest["version"],
                "-e",
                "previous_version=" + old_version,
                image,
                "sh",
                "-c",
                script,
                "smoke",
                *packages,
            ],
            check=True,
        )
    print(
        "Ubuntu 24.04 and Rocky Linux 10 isolated lifecycle checks passed; no native service operation claimed."
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("folder", type=Path)
    parser.add_argument("--engine", choices=["docker", "podman"], default="podman")
    args = parser.parse_args()
    smoke(args.folder, args.engine)
