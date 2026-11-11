#!/usr/bin/env python3
"""Reproducible, allowlisted KagNode Entropy source packages."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import tarfile
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def config():
    return json.loads((ROOT / "packaging/release.json").read_text())


def identity():
    cfg = config()
    source = (ROOT / cfg["version_file"]).read_text()
    version = re.search(cfg["version_pattern"], source).group(1)
    release = (
        version
        if cfg.get("channel") == "stable"
        else (
            version
            + ("." if "-" in version else "-")
            + "packaging."
            + str(cfg["revision"])
        )
    )
    if cfg.get("channel") == "stable" and not re.fullmatch(
        r"[0-9]+\.[0-9]+\.[0-9]+", version
    ):
        raise ValueError("Stable releases require a final product version")
    return cfg, version, release


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def source_files():
    names = json.loads((ROOT / "packaging/source-files.json").read_text())
    if names != sorted(set(names)):
        raise ValueError("Source allowlist must be sorted and unique")
    files = []
    for name in names:
        rel = PurePosixPath(name)
        if rel.is_absolute() or ".." in rel.parts or str(rel) != name:
            raise ValueError("Unsafe source path: " + name)
        if any(
            part
            in {".git", ".venv", "node_modules", "__pycache__", "private", "runtime"}
            for part in rel.parts
        ):
            raise ValueError("Private/generated path in source allowlist: " + name)
        if rel.name in {
            "credentials.json",
            "admin.key",
            ".service.lock",
        } or rel.name.startswith(".credentials-"):
            raise ValueError(
                "Private administration state in source allowlist: " + name
            )
        path = ROOT / name
        if (
            path.is_symlink()
            or not path.is_file()
            or not path.resolve().is_relative_to(ROOT.resolve())
            or any(parent.is_symlink() for parent in path.parents if parent != ROOT)
        ):
            raise ValueError("Expected a regular source file: " + name)
        raw = path.read_bytes()
        if re.search(rb"^-----BEGIN (?:[A-Z0-9 ]+ )?PRIVATE KEY-----\s*$", raw, re.M):
            raise ValueError("Private key in source: " + name)
        if path.suffix in {".sqlite", ".sqlite3", ".db", ".qcow2"} or name.startswith(
            ".env"
        ):
            raise ValueError("Runtime data in source: " + name)
        files.append((name, path))
    return files


def source_mode(path):
    return 0o755 if path.stat().st_mode & 0o111 else 0o644


def archives(files, prefix, out, epoch):
    """All formats use identical logical contents, with no links or host metadata."""
    tar = io.BytesIO()
    with tarfile.open(fileobj=tar, mode="w", format=tarfile.PAX_FORMAT) as archive:
        for name, path in sorted(files):
            raw = path.read_bytes()
            member = tarfile.TarInfo(prefix + "/" + name)
            member.size, member.mode, member.mtime = len(raw), source_mode(path), epoch
            member.uid = member.gid = 0
            member.uname = member.gname = "root"
            archive.addfile(member, io.BytesIO(raw))
    import lzma

    (out / (prefix + ".tar.xz")).write_bytes(lzma.compress(tar.getvalue(), preset=6))
    with (out / (prefix + ".tar.gz")).open("wb") as dest:
        with gzip.GzipFile(fileobj=dest, mode="wb", filename="", mtime=epoch) as stream:
            stream.write(tar.getvalue())
    with zipfile.ZipFile(
        out / (prefix + ".zip"), "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9
    ) as archive:
        import time

        date = time.gmtime(max(epoch, 315532800))[:6]
        for name, path in sorted(files):
            entry = zipfile.ZipInfo(prefix + "/" + name, date)
            entry.create_system = 3
            entry.external_attr = (0o100000 | source_mode(path)) << 16
            entry.compress_type = zipfile.ZIP_DEFLATED
            archive.writestr(entry, path.read_bytes())


def system_packages(files, package, version, out, epoch, arch="all", binary=False):
    debversion = version.replace("-", "~", 1)
    with tempfile.TemporaryDirectory(prefix="kagnode-package-") as temporary:
        temp = Path(temporary)
        root = temp / "root"
        root.mkdir()
        if binary:
            target = root / "usr/bin"
            target.mkdir(parents=True)
            shutil.copyfile(dict(files)["kagnode"], target / "kagnode")
            (target / "kagnode").chmod(0o755)
            docs = root / "usr/share/doc" / package
            docs.mkdir(parents=True)
            for name, path in files:
                if name != "kagnode":
                    dest = docs / name
                    dest.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(path, dest)
        else:
            target = root / "usr/share/kagnode" / package
            for name, path in files:
                dest = target / name
                dest.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(path, dest)
                dest.chmod(source_mode(path))
        # No maintainer script, daemon activation, networking or user-data removal.
        debian = root / "DEBIAN"
        debian.mkdir()
        (debian / "control").write_text(
            f"Package: {package}\nVersion: {debversion}\nArchitecture: {arch}\n"
            "Maintainer: KagNode contributors <every-lemon@users.noreply.github.com>\n"
            "Section: misc\nPriority: optional\n"
            f"Description: KagNode {package} source distribution\n"
            " Source/data packages do not provision native services.\n"
        )
        for path in [root, *root.rglob("*")]:
            if path.is_dir():
                path.chmod(0o755)
            os.utime(path, (epoch, epoch))
        env = {**os.environ, "SOURCE_DATE_EPOCH": str(epoch)}
        deb = out / f"{package}_{version}_{arch}.deb"
        subprocess.run(
            [
                "dpkg-deb",
                "--threads-max=1",
                "--root-owner-group",
                "-Zxz",
                "--build",
                str(root),
                str(deb),
            ],
            check=True,
            env=env,
        )
        shutil.rmtree(debian)
        specroot = temp / "rpm"
        for directory in ["SPECS", "BUILD", "BUILDROOT", "RPMS", "SOURCES", "SRPMS"]:
            (specroot / directory).mkdir(parents=True)
        rpmarch = {"all": "noarch", "amd64": "x86_64", "arm64": "aarch64"}[arch]
        base, separator, prerelease = version.partition("-")
        rpmversion = base + "~" + prerelease if separator else base
        license_id = config()["license"]
        rcfile = temp / "rpmrc"
        rcfile.write_text(
            "buildarch_compat: " + os.uname().machine + ": x86_64 aarch64 noarch\n"
        )
        rpmconfig = subprocess.check_output(
            ["rpm", "--eval", "%{_rpmconfigdir}"], text=True
        ).strip()
        spec = specroot / "SPECS/package.spec"
        owned = (
            f"/usr/bin/kagnode\n/usr/share/doc/{package}"
            if binary
            else f"/usr/share/kagnode/{package}"
        )
        spec.write_text(f"""Name: {package}
Version: {rpmversion}
Release: 1
Summary: KagNode Entropy source and documentation
License: {license_id}
URL: https://github.com/every-lemon/{config()["repository"]}
BuildArch: {rpmarch}
AutoReqProv: no
%global debug_package %{{nil}}
%global __os_install_post %{{nil}}
%global _build_id_links none
%global clamp_mtime_to_source_date_epoch 1
%global use_source_date_epoch_as_buildtime 1
%description
KagNode Entropy source and documentation. No services are installed.
Native service provisioning is not performed or qualified by this RPM.
%prep
%build
%install
mkdir -p %{{buildroot}}
cp -a {root}/. %{{buildroot}}/
%files
{owned}
""")
        subprocess.run(
            [
                "rpmbuild",
                "--rcfile",
                rpmconfig + "/rpmrc:" + str(rcfile),
                "-bb",
                "--target",
                rpmarch,
                "--define",
                "_topdir " + str(specroot),
                "--define",
                "_buildhost build.kagnode.invalid",
                str(spec),
            ],
            check=True,
            env=env,
            stdout=subprocess.DEVNULL,
        )
        rpms = list((specroot / "RPMS").rglob("*.rpm"))
        if len(rpms) != 1:
            raise ValueError("Expected exactly one RPM")
        shutil.copyfile(rpms[0], out / f"{package}_{version}_{arch}.rpm")


def build(out, native_cli=True):
    cfg, product, version = identity()
    out = out.resolve()
    if out.exists() and any(out.iterdir()):
        raise ValueError(
            "Use an empty output directory; old release artifacts are retained"
        )
    out.mkdir(parents=True, exist_ok=True)
    epoch = int(
        os.environ.get(
            "SOURCE_DATE_EPOCH",
            subprocess.check_output(
                ["git", "log", "-1", "--format=%ct"], cwd=ROOT, text=True
            ).strip(),
        )
    )
    files = source_files()
    prefix = f"{cfg['repository']}_{version}_source"
    archives(files, prefix, out, epoch)
    package = (
        cfg["repository"] if cfg["kind"] == "artwork" else cfg["repository"] + "-source"
    )
    system_packages(files, package, version, out, epoch)
    if cfg["kind"] == "cli" and native_cli:
        with tempfile.TemporaryDirectory(prefix="kagnode-cli-") as temporary:
            stage = Path(temporary)
            for arch in ["amd64", "arm64"]:
                binary = stage / "kagnode"
                subprocess.run(
                    [
                        "go",
                        "build",
                        "-trimpath",
                        "-buildvcs=false",
                        "-ldflags=-s -w",
                        "-o",
                        str(binary),
                        ".",
                    ],
                    cwd=ROOT,
                    env={
                        **os.environ,
                        "CGO_ENABLED": "0",
                        "GOOS": "linux",
                        "GOARCH": arch,
                    },
                    check=True,
                )
                binary.chmod(0o755)
                payload = [("kagnode", binary)] + [
                    (name, ROOT / name)
                    for name in [
                        "README.md",
                        "LICENSE",
                        "BRAND-POLICY.md",
                        "ARTWORK-LICENSE",
                        "assets/kagnode-wordmark.svg",
                    ]
                ]
                archives(
                    payload, f"{cfg['repository']}_{version}_linux_{arch}", out, epoch
                )
                system_packages(
                    payload,
                    cfg["repository"],
                    version,
                    out,
                    epoch,
                    arch=arch,
                    binary=True,
                )
    assets = {
        p.name: {"sha256": digest(p), "size": p.stat().st_size}
        for p in sorted(out.iterdir())
        if p.is_file()
    }
    head = subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True
    ).strip()
    manifest = {
        "schema": 1,
        "repository": "every-lemon/" + cfg["repository"],
        "version": version,
        "product_version": product,
        "source_commit": head,
        "epoch": epoch,
        "channel": cfg.get("channel", "preview"),
        "rpm_status": "Rocky Linux 10 source-only package; no service installation or native provisioning",
        "assets": assets,
    }
    (out / "release-manifest.json").write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n"
    )
    (out / "SHA256SUMS").write_text(
        "".join(
            f"{digest(p)}  {p.name}\n" for p in sorted(out.iterdir()) if p.is_file()
        )
    )
    print(json.dumps({"version": version, "output": str(out), "files": len(assets)}))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "dist/packages")
    parser.add_argument("--identity", action="store_true")
    parser.add_argument("--no-cli-binaries", action="store_true")
    args = parser.parse_args()
    if args.identity:
        cfg, product, version = identity()
        print(
            json.dumps(
                {
                    "repository": cfg["repository"],
                    "product": product,
                    "version": version,
                }
            )
        )
    else:
        build(args.output, not args.no_cli_binaries)
