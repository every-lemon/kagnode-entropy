"""Project-owned installer execution. Source packages never invoke this automatically.

No shell command construction, data migration, guessed restore, or native-image work.
Only an exact fresh reviewed plan may mutate a host. Tests inject process/platform I/O;
there is deliberately no production fake-success flag.
"""

from __future__ import annotations
import argparse
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import platform
import pwd
import re
import signal
import shutil
import stat
import subprocess
import sys
import time
from urllib.parse import urlsplit
from urllib.request import Request, build_opener, ProxyHandler, HTTPRedirectHandler
from installer_process import OwnedRunner, interrupted
from installer_protocol import (
    ProtocolError,
    canonical,
    plan_digest,
    read_request,
    reviewed_plan,
    write_result,
)


class LocalOnly(HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        raise ProtocolError("The local health endpoint redirected unexpectedly")


class Installer:
    def __init__(self, source, spec, run=None):
        self.source, self.spec, self.run = (
            Path(source).resolve(),
            spec,
            run or OwnedRunner(),
        )

    def platform(self):
        release = platform.freedesktop_os_release()
        return {
            "id": release.get("ID", "unknown"),
            "version": release.get("VERSION_ID", ""),
            "arch": platform.machine(),
        }

    def source_inventory(self):
        names = json.loads((self.source / "packaging/source-files.json").read_text())
        if isinstance(names, dict):
            names = names.get("files", [])
        if not isinstance(names, list) or not names:
            raise ProtocolError("Source publication inventory is missing")
        result = {}
        for name in names:
            if (
                not isinstance(name, str)
                or Path(name).is_absolute()
                or ".." in Path(name).parts
            ):
                raise ProtocolError("Invalid source inventory")
            path = self.source / name
            self.safe_path(path)
            if not path.is_file():
                raise ProtocolError("Source inventory is incomplete")
            result[name] = hashlib.sha256(path.read_bytes()).hexdigest()
        return result

    @staticmethod
    def safe_path(path):
        path = Path(path)
        if (
            not path.is_absolute()
            or ".." in path.parts
            or any(p.is_symlink() for p in (path, *path.parents))
        ):
            raise ProtocolError("A managed path is unsafe")
        return path

    def configuration(self, request, health=False, partial=False):
        fields = {f["key"]: f for f in self.spec["fields"]}
        incoming = request.get("configuration", {})
        if set(incoming) - fields.keys():
            raise ProtocolError("Unknown configuration field")
        result = {}
        for key, field in fields.items():
            value = incoming.get(key, field.get("default", ""))
            if field["type"] == "integer":
                if type(value) is not int or not 1024 <= value <= 65535:
                    raise ProtocolError("Invalid unprivileged listener port")
            elif (
                not isinstance(value, str)
                or len(value) > 2048
                or any(c in value for c in "\n\r\x00")
            ):
                raise ProtocolError("Invalid configuration value")
            if (
                field.get("required")
                and value == ""
                and request["mode"] != "adopt"
                and not health
                and not partial
            ):
                raise ProtocolError("Required configuration is missing: " + key)
            if (
                key in ("domain", "hostname")
                and value
                and not re.fullmatch(
                    r"(?=.{1,253}$)[a-z0-9](?:[a-z0-9.-]*[a-z0-9])?", value
                )
            ):
                raise ProtocolError("Use a lowercase DNS hostname")
            if key in ("ipv4", "ipv6", "trusted_proxy") and value:
                address = ipaddress.ip_address(value)
                if (
                    key == "ipv4"
                    and address.version != 4
                    or key == "ipv6"
                    and address.version != 6
                ):
                    raise ProtocolError("Wrong IP address family")
            if (
                key in ("admin", "admin_email")
                and value
                and not re.fullmatch(r"[^\s@]{1,64}@[^\s@]{1,253}", value)
            ):
                raise ProtocolError("Invalid administrator email")
            if key == "public_url" and value:
                u = urlsplit(value)
                if (
                    u.scheme != "https"
                    or not u.hostname
                    or u.username
                    or u.password
                    or u.path not in ("", "/")
                    or u.query
                    or u.fragment
                ):
                    raise ProtocolError("Public URL must be an HTTPS origin")
                value = value.rstrip("/")
            if (
                key.endswith("password_file")
                and value
                and not health
                and request["mode"] != "adopt"
            ):
                p = self.safe_path(value)
                fd = os.open(p, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
                try:
                    meta = os.fstat(fd)
                    if (
                        not stat.S_ISREG(meta.st_mode)
                        or meta.st_uid != os.geteuid()
                        or meta.st_mode & 0o077
                        or meta.st_nlink != 1
                        or not 12 <= meta.st_size <= 1024
                    ):
                        raise ProtocolError(
                            "Password file must be private, regular, and owned by the installer user"
                        )
                finally:
                    os.close(fd)
            result[key] = value
        return result

    def paths(self, request):
        paths = dict(self.spec["paths"])
        if self.spec["kind"] in ("application", "static"):
            application = self.safe_path(Path(request["root"]) / self.spec["project"])
            shared = {
                "code": str(application / "runtime"),
                "configuration": str(application / "config/service.env"),
                "data": str(application / "data"),
                "unit": paths.get(
                    "unit", str(application / "runtime/packaging/source-files.json")
                ),
            }
            if self.spec["kind"] == "static":
                shared = {
                    "code": str(application / "runtime"),
                    "data": str(application / "runtime"),
                    "configuration": str(
                        application / "runtime/packaging/source-files.json"
                    ),
                }
            if request["mode"] != "adopt" or not any(
                Path(paths[k]).exists() for k in ("code", "data", "configuration")
            ):
                paths = shared
        return paths

    def installed_binding(self, request):
        """Read only recognized service fields; never import data or return secrets."""
        paths = self.paths(request)
        port = request.get("configuration", {}).get("port")
        if self.spec["kind"] != "application":
            return paths, port
        unit = self.spec["units"][0]
        properties = {}
        for key in (
            "WorkingDirectory",
            "ExecStart",
            "EnvironmentFiles",
            "FragmentPath",
        ):
            try:
                result = self.run(
                    ["systemctl", "show", unit, "--property=" + key, "--value"],
                    capture_output=True,
                    text=True,
                    timeout=5,
                    check=False,
                )
                if result.returncode == 0 and len(result.stdout) <= 65536:
                    properties[key] = result.stdout.strip()
            except (OSError, subprocess.SubprocessError):
                pass

        def location(value):
            return bool(
                isinstance(value, str)
                and re.fullmatch(r"/[A-Za-z0-9_./+-]+", value)
                and ".." not in Path(value).parts
            )

        if location(properties.get("WorkingDirectory")):
            paths["code"] = properties["WorkingDirectory"]
        if location(properties.get("FragmentPath")):
            paths["unit"] = properties["FragmentPath"]
        env = properties.get("EnvironmentFiles", "").split(" (", 1)[0]
        if location(env):
            paths["configuration"] = env
        config = Path(paths["configuration"])
        if (
            config.is_file()
            and not config.is_symlink()
            and config.stat().st_size <= 65536
        ):
            for line in config.read_text().splitlines():
                key, separator, value = line.partition("=")
                value = value.strip().strip('"').strip("'")
                if (
                    separator
                    and key in ("PORT", "AUTHENTACTOR_PORT")
                    and port is None
                    and value.isdecimal()
                ):
                    port = int(value)
                if (
                    separator
                    and key in ("DATA_DIR", "AUTHENTACTOR_DATA")
                    and location(value)
                ):
                    paths["data"] = value
        command = properties.get("ExecStart", "")
        match = re.search(r"--port(?:=|\s+)([0-9]{1,5})(?:\s|[;}]|$)", command)
        if port is None and match:
            port = int(match[1])
        match = re.search(r"--data-dir(?:=|\s+)(/[A-Za-z0-9_./+-]+)", command)
        if match and location(match[1]):
            paths["data"] = match[1]
        match = re.search(r"--credentials-file(?:=|\s+)(/[A-Za-z0-9_./+-]+)", command)
        if self.spec["project"] == "entropy" and match and location(match[1]):
            directory = Path(match[1]).parent
            paths["data"] = str(
                directory.parent if directory.name == "identity" else directory
            )
        if type(port) is not int or not 1024 <= port <= 65535:
            port = None
        return paths, port

    def workspace(self, request):
        root = self.safe_path(request["root"])
        for parent in (root, *root.parents):
            if not parent.exists():
                continue
            meta = parent.stat()
            # Root-owned sticky temporary parents preserve the ownership of
            # the private child, and are used by disposable test/setup roots.
            writable = meta.st_mode & 0o022
            sticky_parent = parent != root and meta.st_mode & stat.S_ISVTX
            if (
                not stat.S_ISDIR(meta.st_mode)
                or meta.st_uid not in (0, os.geteuid())
                or (writable and not sticky_parent)
            ):
                raise ProtocolError(
                    "Installer workspace ancestors must be owned and protected from other users"
                )
        return root

    def journal(self, request):
        return self.safe_path(
            Path(request["root"])
            / self.spec["project"]
            / "installer"
            / (request["operation_id"] + ".json")
        )

    def read_journal(self, request):
        path = self.journal(request)
        if not path.exists():
            return None
        meta = path.stat()
        if (
            not stat.S_ISREG(meta.st_mode)
            or meta.st_mode & 0o077
            or meta.st_uid != os.geteuid()
            or meta.st_nlink != 1
        ):
            raise ProtocolError("Recovery journal permissions are unsafe")
        value = json.loads(path.read_text())
        if (
            value.get("operation_id") != request["operation_id"]
            or value.get("project") != self.spec["project"]
        ):
            raise ProtocolError("Recovery journal identity mismatch")
        return value

    @staticmethod
    def system_tool(binary):
        """Build identities must be able to execute root-controlled prerequisites."""
        if not re.fullmatch(r"/[A-Za-z0-9_./+-]+", binary):
            return False
        target = Path(binary).resolve()
        if any(target.is_relative_to(root) for root in ("/root", "/home")):
            return False
        try:
            meta = target.stat()
            if (
                not stat.S_ISREG(meta.st_mode)
                or meta.st_uid != 0
                or meta.st_mode & 0o022
                or meta.st_mode & 0o005 != 0o005
            ):
                return False
            return all(
                parent.stat().st_uid == 0
                and not parent.stat().st_mode & 0o022
                and parent.stat().st_mode & 0o001
                for parent in target.parents
            )
        except OSError:
            return False

    def plan(self, request):
        self.workspace(request)
        config = self.configuration(request, partial=True)
        missing = (
            [
                f["key"]
                for f in self.spec["fields"]
                if f.get("required") and config[f["key"]] == ""
            ]
            if request["mode"] != "adopt"
            else []
        )
        host = self.platform()
        mode = request["mode"]
        intent = request.get("intent", "setup")
        if intent not in ("setup", "recover"):
            raise ProtocolError("Unknown installation intent")
        paths = (
            self.installed_binding(request)[0]
            if mode == "adopt"
            else self.paths(request)
        )
        for path in paths.values():
            self.safe_path(path)
        native = self.spec["kind"] == "native"
        supported = mode == "adopt" or mode == ("native" if native else "application")
        reason = (
            ""
            if supported
            else "This project does not support the selected installation mode."
        )
        if mode != "adopt" and (host["id"], host["version"]) not in [
            tuple(x) for x in self.spec["platforms"]
        ]:
            supported, reason = (
                False,
                "This setup adapter does not support this operating-system release.",
            )
        if native and host["arch"] not in self.spec.get(
            "architectures", ["x86_64", "aarch64"]
        ):
            supported, reason = (
                False,
                "Native setup does not support this architecture.",
            )
        dependencies = [
            {"name": name, "present": bool(shutil.which(name)), "required": True}
            for name in self.spec["dependencies"]
        ]
        if self.spec["kind"] == "application":
            versions = {
                "passwords": ("python3", "--version", (3, 12)),
                "media": ("node", "--version", (24, 0)),
                "entropy": ("rustc", "--version", (1, 88)),
            }
            executable, flag, minimum = versions[self.spec["project"]]
            item = next(x for x in dependencies if x["name"] == executable)
            item["version"] = "unavailable"
            item["compatible"] = False
            # Do not execute a PATH-resolved program as root before checking
            # that it is a system-wide, root-controlled prerequisite.
            if item["present"] and self.system_tool(shutil.which(executable)):
                try:
                    probe = self.run(
                        [shutil.which(executable), flag],
                        env={
                            "PATH": "/usr/local/bin:/usr/bin:/bin",
                            "HOME": "/nonexistent",
                            "LANG": "C.UTF-8",
                        },
                        capture_output=True,
                        text=True,
                        timeout=5,
                        check=False,
                    )
                    match = re.search(r"(\d+)\.(\d+)(?:\.(\d+))?", probe.stdout)
                    if probe.returncode == 0 and match:
                        version = tuple(int(x or "0") for x in match.groups())
                        item["version"] = ".".join(map(str, version))
                        item["compatible"] = version[:2] >= minimum and (
                            version[0] == (24 if executable == "node" else minimum[0])
                        )
                except (OSError, subprocess.SubprocessError):
                    pass
            if mode != "adopt" and not item["compatible"]:
                supported, reason = (
                    False,
                    "The application runtime version does not meet its published prerequisite.",
                )
            for runtime in ("node", "npm", "cargo", "rustc", "python3"):
                binary = (
                    shutil.which(runtime)
                    if runtime in self.spec["dependencies"]
                    else None
                )
                if binary and not self.system_tool(binary) and mode != "adopt":
                    supported, reason = (
                        False,
                        "Install build/runtime prerequisites system-wide; dedicated service users cannot use another account's private toolchain.",
                    )
        if mode != "adopt" and any(not item["present"] for item in dependencies):
            supported, reason = (
                False,
                "Install the listed missing prerequisites before reviewing setup again.",
            )
        existing = self.read_journal(request)
        if existing and existing.get("configuration") != config:
            supported, reason = (
                False,
                "This operation belongs to different configuration.",
            )
        if intent == "recover":
            if not existing or existing.get("configuration") != config:
                supported, reason = (
                    False,
                    "Recovery needs this operation's matching private journal; existing data is never guessed or restored.",
                )
        elif mode != "adopt" and self.spec["kind"] in ("application", "static"):
            data = Path(paths["data"])
            empty_private_data = (
                data.is_dir()
                and not any(data.iterdir())
                and data.stat().st_uid == os.geteuid()
                and not data.stat().st_mode & 0o077
            )
            occupied = any(
                Path(paths[k]).exists()
                for k in ("code", "unit", "configuration")
                if k in paths
            ) or (data.exists() and not empty_private_data)
            if occupied:
                supported, reason = (
                    False,
                    "Existing application files or data were found. Use adopt or an operation-owned recovery; setup never replaces them.",
                )
        if (
            self.spec.get("installed_entrypoint")
            and not Path(self.spec["installed_entrypoint"]).is_file()
        ):
            supported, reason = (
                False,
                "Install the project's native Server package first; this source adapter does not build images or replace native packages.",
            )
        inventory = self.source_inventory()
        plan = {
            "schema": 1,
            "project": self.spec["project"],
            "operation_id": request["operation_id"],
            "intent": intent,
            "supported": supported,
            "reason": reason,
            "platform": host,
            "mode": mode,
            "paths": paths,
            "managed_paths": list(paths.values()),
            "dependencies": dependencies,
            "configuration_fields": self.spec["fields"],
            "configuration": config,
            "source_sha256": hashlib.sha256(canonical(inventory)).hexdigest(),
            "changes": (
                [
                    "Inspect existing installation and record adoption; do not change its data or services"
                ]
                if mode == "adopt"
                else self.spec["changes"]
            ),
            "warnings": self.spec["warnings"],
            "manual_steps": self.spec.get("manual_steps", [])
            if mode != "adopt"
            else [],
            "interactive": bool(self.spec.get("interactive") and mode != "adopt"),
            "entrypoint": self.spec.get("entrypoint", []),
            "requires_root": True,
            "destructive": False,
            "health": {"supported": True, "scope": self.spec["health_scope"]},
            "recovery": existing.get("stage") if existing else None,
        }
        plan["configuration_errors"] = [
            {"field": key, "message": "A value is required."} for key in missing
        ]
        plan["ready_for_apply"] = supported and not missing
        plan["plan_sha256"] = plan_digest(plan)
        return plan

    def command(
        self,
        argv,
        journal,
        *,
        cwd=None,
        env=None,
        stdin=None,
        interactive=False,
        timeout=1800,
    ):
        # Arguments are a fixed executable plus validated individual values, never a shell string.
        if interactive:
            if not sys.stdin.isatty() or not sys.stdout.isatty():
                raise ProtocolError("This reviewed native setup needs a real terminal")
            result = self.run(
                argv,
                cwd=cwd,
                env=env,
                input=stdin,
                timeout=timeout,
                check=False,
                interactive=True,
            )
        else:
            log = Path(journal["log"])
            self.safe_path(log)
            fd = os.open(
                log, os.O_WRONLY | os.O_CREAT | os.O_APPEND | os.O_NOFOLLOW, 0o600
            )
            with os.fdopen(fd, "ab") as output:
                result = self.run(
                    argv,
                    cwd=cwd,
                    env=env,
                    input=stdin,
                    stdout=output,
                    stderr=subprocess.STDOUT,
                    timeout=timeout,
                    check=False,
                )
        if result.returncode:
            raise ProtocolError(
                "A setup command failed; inspect the private local operation log"
            )

    def save_journal(self, request, journal):
        path = self.journal(request)
        path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        for parent in (path.parent, path.parent.parent):
            meta = parent.stat()
            mask = 0o077 if parent == path.parent else 0o022
            if meta.st_uid != os.geteuid() or meta.st_mode & mask:
                raise ProtocolError(
                    "Installer workspace must be private and owned by the current user"
                )
        temp = path.with_suffix(".tmp")
        fd = os.open(temp, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        try:
            with os.fdopen(fd, "wb") as output:
                output.write(canonical(journal) + b"\n")
                output.flush()
                os.fsync(output.fileno())
            os.replace(temp, path)
        finally:
            if temp.exists():
                temp.unlink()

    def health(self, request):
        config = self.configuration(request, health=True)
        paths, installed_port = (
            self.installed_binding(request)
            if request["mode"] == "adopt"
            else (self.paths(request), config.get("port"))
        )
        checks = []
        observed_version = None
        for unit in self.spec["units"]:
            result = self.run(
                ["systemctl", "is-active", "--quiet", unit],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                timeout=10,
                check=False,
            )
            checks.append({"name": unit, "ok": result.returncode == 0})
        for key in self.spec.get("required_paths", ["data"]):
            checks.append(
                {
                    "name": key + " present",
                    "ok": Path(paths[key]).exists()
                    and not Path(paths[key]).is_symlink(),
                }
            )
        if self.spec["kind"] == "static":
            try:
                valid = all(
                    (Path(paths["code"]) / name).is_file()
                    and not (Path(paths["code"]) / name).is_symlink()
                    and hashlib.sha256(
                        (Path(paths["code"]) / name).read_bytes()
                    ).hexdigest()
                    == digest
                    for name, digest in self.source_inventory().items()
                )
            except OSError:
                valid = False
            checks.append({"name": "reviewed artwork inventory", "ok": valid})
        endpoint = self.spec.get("health_path")
        if endpoint:
            try:
                if self.spec["kind"] == "application" and installed_port is None:
                    raise ValueError(
                        "The installed listener port is not known; provide an explicitly reviewed port"
                    )
                address = (
                    "http://127.0.0.1:"
                    + str(installed_port or self.spec.get("port", 8000))
                    + endpoint
                )
                with build_opener(ProxyHandler({}), LocalOnly()).open(
                    Request(address, headers={"Accept": "application/json"}), timeout=5
                ) as response:
                    raw = response.read(65537)
                    if len(raw) > 65536:
                        raise ValueError("Local health response exceeded its limit")
                    body = json.loads(raw)
                    if (
                        isinstance(body, dict)
                        and isinstance(body.get("version"), str)
                        and re.fullmatch(
                            r"[0-9]+\.[0-9]+\.[0-9]+(?:-[A-Za-z0-9.-]+)?",
                            body["version"],
                        )
                    ):
                        observed_version = body["version"]
                    key = self.spec["health_key"]
                    ok = (
                        response.status == 200
                        and isinstance(body, dict)
                        and key in body
                    )
                    if key == "ok":
                        ok = ok and body[key] is True
                    if key == "initialized":
                        ok = ok and type(body[key]) is bool
                    if key == "trusted_execution":
                        ok = (
                            ok
                            and body.get("service") == "KagNode Entropy"
                            and body[key] == "none"
                        )
                checks.append({"name": "local application response", "ok": ok})
            except (OSError, ValueError):
                checks.append({"name": "local application response", "ok": False})
        return {
            "ok": bool(checks) and all(x["ok"] for x in checks),
            "checks": checks,
            "scope": self.spec["health_scope"],
            "observed_version": observed_version,
        }

    def binding(self, request, plan, health):
        return {
            "code": plan["paths"]["code"],
            "config": plan["paths"]["configuration"],
            "data": plan["paths"]["data"],
            "services": self.spec["units"],
            "managed_by": "project-installer",
            "mode": request["mode"],
            "reviewed_source_version": self.spec.get("version")
            if request["mode"] != "adopt"
            else None,
            "reviewed_source_sha256": plan["source_sha256"]
            if request["mode"] != "adopt"
            else None,
            "observed_runtime_version": health.get("observed_version"),
            "updates": {
                "supported": False,
                "reason": "No project-owned runtime update operation is supplied by this adapter. Source downloads do not update the installed runtime; retain the existing verified native update path where applicable.",
            },
        }

    @staticmethod
    def require_root():
        if os.geteuid() != 0:
            raise ProtocolError(
                "Use a local administrator to apply this reviewed setup"
            )

    def apply(self, request):
        plan = reviewed_plan(request, self.plan(request))
        if not plan["ready_for_apply"]:
            raise ProtocolError(
                "Complete and review the installation configuration first"
            )
        self.require_root()
        if request["mode"] == "adopt":
            health = self.health(request)
            if not health["ok"]:
                return self.result(
                    request,
                    "needs_recovery",
                    False,
                    health,
                    "Existing installation did not pass health; no files changed.",
                )
            journal = {
                "schema": 1,
                "project": self.spec["project"],
                "operation_id": request["operation_id"],
                "configuration": plan["configuration"],
                "source_sha256": plan["source_sha256"],
                "stage": "adopted",
                "log": str(self.journal(request).with_suffix(".log")),
            }
            self.save_journal(request, journal)
            result = self.result(
                request, "completed", False, health, "Read-only adoption recorded."
            )
            result["binding"] = self.binding(request, plan, health)
            return result
        journal = self.read_journal(request) or {
            "schema": 1,
            "project": self.spec["project"],
            "operation_id": request["operation_id"],
            "configuration": plan["configuration"],
            "source_sha256": plan["source_sha256"],
            "stage": "reviewed",
            "log": str(self.journal(request).with_suffix(".log")),
            "done": [],
        }
        if journal["source_sha256"] != plan["source_sha256"]:
            raise ProtocolError(
                "Recovery source differs from the reviewed installation"
            )
        self.save_journal(request, journal)
        try:
            if self.spec["kind"] == "native":
                self.native(request, plan, journal)
            elif self.spec["kind"] == "static":
                self.static(request, plan, journal)
            else:
                self.application(request, plan, journal)
            health = self.health(request)
            journal["stage"] = "completed" if health["ok"] else "needs_recovery"
            self.save_journal(request, journal)
            result = self.result(
                request,
                journal["stage"],
                True,
                health,
                "Keep this private operation journal. Review a recovery plan before resuming incomplete setup.",
            )
            result["binding"] = self.binding(request, plan, health)
            return result
        except (
            ProtocolError,
            OSError,
            subprocess.SubprocessError,
            KeyboardInterrupt,
        ) as error:
            journal["stage"] = "needs_recovery"
            self.save_journal(request, journal)
            health = {"ok": False, "checks": []}
            result = self.result(request, "needs_recovery", True, health, str(error))
            result["binding"] = self.binding(request, plan, health)
            return result

    def native(self, request, plan, journal):
        c = plan["configuration"]
        project = self.spec["project"]
        if project == "mail":
            argv = ["/bin/bash", str(self.source / "install.sh")]
            for key in ("domain", "hostname", "ipv4", "ipv6", "admin", "password_file"):
                if c.get(key):
                    argv += ["--" + key.replace("_", "-"), c[key]]
            if request.get("intent") == "recover":
                argv.append("--resume")
        elif project == "vpn":
            argv = ["/bin/bash", str(self.source / "setup.sh"), "--install"]
        else:
            argv = [self.spec["installed_entrypoint"], "setup"]
        self.command(
            argv,
            journal,
            cwd=self.source,
            interactive=plan["interactive"],
            timeout=7200,
        )

    def static(self, request, plan, journal):
        target = self.safe_path(plan["paths"]["code"])
        target.mkdir(parents=True, exist_ok=True, mode=0o755)
        for name, expected in self.source_inventory().items():
            dest = self.safe_path(target / name)
            dest.parent.mkdir(parents=True, exist_ok=True)
            if dest.exists():
                if (
                    not dest.is_file()
                    or hashlib.sha256(dest.read_bytes()).hexdigest() != expected
                ):
                    raise ProtocolError(
                        "Static artwork differs; no existing file was replaced"
                    )
            else:
                with dest.open("xb") as output:
                    output.write((self.source / name).read_bytes())
                dest.chmod(0o644)

    def application(self, request, plan, journal):
        paths = plan["paths"]
        config = plan["configuration"]
        project = self.spec["project"]
        user = self.spec["user"]

        def step(name, action):
            if name in journal["done"]:
                return
            journal["stage"] = name
            self.save_journal(request, journal)
            action()
            journal["done"].append(name)
            self.save_journal(request, journal)

        def account():
            try:
                pwd.getpwnam(user)
            except KeyError:
                self.command(
                    [
                        "useradd",
                        "--system",
                        "--home-dir",
                        paths["data"],
                        "--shell",
                        "/usr/sbin/nologin",
                        user,
                    ],
                    journal,
                )

        step("service-account", account)
        identity = pwd.getpwnam(user)
        if (
            identity.pw_uid == 0
            or identity.pw_gid == 0
            or identity.pw_shell
            not in ("/usr/sbin/nologin", "/sbin/nologin", "/bin/false")
        ):
            raise ProtocolError(
                "The service identity must be a dedicated non-root account with login disabled"
            )

        def as_user(argv, **kwargs):
            self.command(["runuser", "-u", user, "--", *argv], journal, **kwargs)

        def data():
            directory = self.safe_path(paths["data"])
            directory.mkdir(parents=True, exist_ok=True, mode=0o700)
            if any(directory.iterdir()):
                raise ProtocolError(
                    "Private data already exists; no contents or ownership were changed"
                )
            directory.chmod(0o700)
            os.chown(directory, identity.pw_uid, identity.pw_gid)

        step("private-data-directory", data)

        def source():
            target = Path(paths["code"])
            target.mkdir(parents=True, exist_ok=True, mode=0o755)
            for name, expected in self.source_inventory().items():
                dest = target / name
                dest.parent.mkdir(parents=True, exist_ok=True)
                self.safe_path(dest)
                if dest.exists():
                    if (
                        not dest.is_file()
                        or hashlib.sha256(dest.read_bytes()).hexdigest() != expected
                    ):
                        raise ProtocolError(
                            "Recovery source differs from this operation; no file was replaced"
                        )
                else:
                    with dest.open("xb") as output:
                        output.write((self.source / name).read_bytes())
                dest.chmod(
                    0o755 if (self.source / name).stat().st_mode & 0o111 else 0o644
                )
            for entry in [target, *target.rglob("*")]:
                self.safe_path(entry)
                os.chown(entry, identity.pw_uid, identity.pw_gid)

        step("source-copy", source)
        env = {
            "PATH": ":".join(
                dict.fromkeys(
                    [
                        str(Path(shutil.which(name)).parent)
                        for name in self.spec["dependencies"]
                        if shutil.which(name)
                    ]
                    + [
                        "/usr/local/sbin",
                        "/usr/local/bin",
                        "/usr/sbin",
                        "/usr/bin",
                        "/sbin",
                        "/bin",
                    ]
                )
            ),
            "LANG": "C.UTF-8",
            "HOME": paths["data"],
            "DATA_DIR": paths["data"],
            "PUBLIC_URL": config.get("public_url", ""),
            "NODE_ENV": "production",
        }
        code = Path(paths["code"])
        if project == "passwords":
            step(
                "python-environment",
                lambda: as_user(
                    [shutil.which("python3"), "-m", "venv", str(code / "venv")], env=env
                ),
            )
            step(
                "python-dependencies",
                lambda: as_user(
                    [
                        str(code / "venv/bin/python"),
                        "-m",
                        "pip",
                        "install",
                        "--only-binary=:all:",
                        "-r",
                        str(code / "requirements.txt"),
                    ],
                    cwd=code,
                    env=env,
                ),
            )
            command = [
                str(code / "venv/bin/python"),
                "-m",
                "authentactor",
                "--host",
                "127.0.0.1",
                "--port",
                str(config["port"]),
                "--data-dir",
                paths["data"],
            ]
            if config.get("trusted_proxy"):
                command += ["--trusted-proxy", config["trusted_proxy"]]
        elif project == "media":
            step(
                "node-dependencies",
                lambda: as_user(
                    [shutil.which("npm"), "ci", "--include=dev", "--ignore-scripts"],
                    cwd=code,
                    env=env,
                ),
            )
            step(
                "node-build",
                lambda: as_user(
                    [shutil.which("npm"), "run", "build"], cwd=code, env=env
                ),
            )
            command = [shutil.which("node"), str(code / "dist/server.js")]

            def administrator():
                secret_path = self.safe_path(config["admin_password_file"])
                descriptor = os.open(
                    secret_path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK
                )
                try:
                    meta = os.fstat(descriptor)
                    if (
                        not stat.S_ISREG(meta.st_mode)
                        or meta.st_uid != os.geteuid()
                        or meta.st_mode & 0o077
                        or meta.st_nlink != 1
                        or not 12 <= meta.st_size <= 1024
                    ):
                        raise ProtocolError(
                            "Administrator password file changed or is not private"
                        )
                    secret = os.read(descriptor, 1025)
                    if len(secret) != meta.st_size:
                        raise ProtocolError(
                            "Administrator password file changed while reading"
                        )
                finally:
                    os.close(descriptor)
                self.command(
                    [
                        "runuser",
                        "-u",
                        user,
                        "--",
                        shutil.which("node"),
                        str(code / "dist/cli.js"),
                        "admin-create",
                        config["admin_email"],
                        config["admin_name"],
                        "--password-stdin",
                    ],
                    journal,
                    cwd=code,
                    env=env,
                    stdin=secret,
                )

            step("initial-administrator", administrator)
        else:
            step(
                "rust-build",
                lambda: as_user(
                    [
                        shutil.which("cargo"),
                        "build",
                        "--release",
                        "--locked",
                        "--jobs",
                        "2",
                    ],
                    cwd=code,
                    env=env,
                    timeout=3600,
                ),
            )
            command = [
                str(code / "target/release/kagnode-entropy"),
                "serve",
                "--credentials-file",
                str(Path(paths["data"]) / "identity/credentials.json"),
                "--port",
                str(config["port"]),
            ]
            step(
                "private-credentials",
                lambda: self.command(
                    [
                        "runuser",
                        "-u",
                        user,
                        "--",
                        command[0],
                        "init",
                        "--directory",
                        str(Path(paths["data"]) / "identity"),
                    ],
                    journal,
                    cwd=code,
                ),
            )

        def promote():
            # Dependency/build processes ran without root; service code becomes read-only.
            for entry in [code, *code.rglob("*")]:
                if entry.is_symlink():
                    # Python venv links only to the reviewed system interpreter; other links stay inside source.
                    allowed_python = (
                        project == "passwords"
                        and entry.parent == code / "venv/bin"
                        and re.fullmatch(r"python(?:[0-9]+(?:\.[0-9]+)?)?", entry.name)
                        and entry.resolve() == Path(shutil.which("python3")).resolve()
                    )
                    if not entry.resolve().is_relative_to(code) and not allowed_python:
                        raise ProtocolError(
                            "A build output symlink escapes the application directory"
                        )
                    os.lchown(entry, 0, 0)
                else:
                    os.chown(entry, 0, 0)
                    entry.chmod(entry.stat().st_mode & 0o755)

        step("read-only-code", promote)

        def write_exact(target, content, mode):
            self.safe_path(target)
            if target.exists():
                if not target.is_file() or target.read_text() != content:
                    raise ProtocolError(
                        "Recovery cannot replace differing configuration or service files"
                    )
                return
            fd = os.open(
                target, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, mode
            )
            with os.fdopen(fd, "w") as stream:
                stream.write(content)

        def configure():
            target = Path(paths["configuration"])
            target.parent.mkdir(parents=True, exist_ok=True)
            content = (
                "HOST=127.0.0.1\nPORT="
                + str(config["port"])
                + "\nDATA_DIR="
                + paths["data"]
                + "\nNODE_ENV=production\n"
            )
            if config.get("public_url"):
                content += "PUBLIC_URL=" + config["public_url"] + "\n"
            write_exact(target, content, 0o640)
            os.chown(target, 0, identity.pw_gid)

        step("configuration", configure)

        def service():
            unit = (
                "[Unit]\nDescription=KagNode "
                + project.title()
                + "\nAfter=network.target\n[Service]\nUser="
                + user
                + "\nGroup="
                + user
                + "\nWorkingDirectory="
                + paths["code"]
                + "\nEnvironmentFile="
                + paths["configuration"]
                + "\nExecStart="
                + " ".join(command)
                + "\nRestart=on-failure\nUMask=0077\nNoNewPrivileges=yes\nPrivateTmp=yes\nProtectSystem=strict\nProtectHome=yes\nReadWritePaths="
                + paths["data"]
                + "\n[Install]\nWantedBy=multi-user.target\n"
            )
            write_exact(Path(paths["unit"]), unit, 0o644)
            self.command(["systemctl", "daemon-reload"], journal)
            self.command(
                ["systemctl", "enable", "--now", self.spec["units"][0]], journal
            )

        step("web-service", service)
        for _ in range(20):
            if self.health(request)["ok"]:
                break
            time.sleep(0.5)

    def result(self, request, state, changed, health, recovery):
        return {
            "schema": 1,
            "project": self.spec["project"],
            "operation_id": request["operation_id"],
            "state": state,
            "changed": changed,
            "health": health,
            "recovery": recovery,
        }

    def main(self):
        parser = argparse.ArgumentParser(
            description="Reviewed project installer protocol"
        )
        parser.add_argument(
            "operation", choices=("plan", "apply", "status", "health", "recover")
        )
        parser.add_argument("--request", required=True)
        parser.add_argument("--output", required=True)
        args = parser.parse_args()
        request = read_request(args.request, self.spec["project"])
        signal.signal(signal.SIGTERM, interrupted)
        try:
            if args.operation == "plan":
                result = self.plan(request)
            elif args.operation in ("apply", "recover"):
                if args.operation == "recover" and request.get("intent") != "recover":
                    raise ProtocolError("Review a recovery plan first")
                result = self.apply(request)
            else:
                health = self.health(request)
                result = self.result(
                    request,
                    "ready" if health["ok"] else "incomplete",
                    False,
                    health,
                    "No changes made.",
                )
            write_result(args.output, result)
            return 0 if result.get("state") not in ("failed", "needs_recovery") else 1
        except (
            ProtocolError,
            OSError,
            ValueError,
            subprocess.SubprocessError,
        ) as error:
            write_result(
                args.output,
                self.result(
                    request, "failed", False, {"ok": False, "checks": []}, str(error)
                ),
            )
            return 1
