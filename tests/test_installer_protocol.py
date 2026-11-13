"""Disposable plan/apply orchestration checks; no native provisioning is performed."""

from __future__ import annotations
import copy
import json
from pathlib import Path
import subprocess
import os
import time
import sys
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from installer_process import OwnedRunner
from installer_runtime import Installer
from installer_protocol import (
    ProtocolError,
    canonical,
    plan_digest,
    read_request,
    reviewed_plan,
)


class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.folder = Path(self.temp.name)
        source = self.folder / "source"
        (source / "packaging").mkdir(parents=True)
        (source / "fixture.txt").write_text("Reviewed source\n")
        (source / "packaging/source-files.json").write_text('["fixture.txt"]')
        self.spec = json.loads((ROOT / "scripts/installer.json").read_text())
        self.spec = copy.deepcopy(self.spec)
        self.spec["paths"] = {
            key: str(self.folder / "legacy" / key) for key in self.spec["paths"]
        }
        if "installed_entrypoint" in self.spec:
            self.spec["installed_entrypoint"] = str(self.folder / "native-console")
            Path(self.spec["installed_entrypoint"]).write_text("fixture")
        self.calls = []

        def execute(argv, **options):
            self.calls.append((argv, options))
            text = {
                "python3": "Python 3.12.9",
                "node": "v24.12.0",
                "rustc": "rustc 1.90.0",
            }.get(Path(argv[0]).name, "")
            return subprocess.CompletedProcess(argv, 0, stdout=text, stderr="")

        self.installer = Installer(source, self.spec, execute)
        self.installer.require_root = lambda: None
        self.installer.system_tool = lambda _: True
        self.installer.platform = lambda: {
            "id": "ubuntu",
            "version": "24.04",
            "arch": "x86_64",
        }
        self.request = {
            "schema": 1,
            "project": self.spec["project"],
            "operation_id": "1" * 32,
            "root": str(self.folder / "workspace"),
            "mode": "native" if self.spec["kind"] == "native" else "application",
            "configuration": {},
        }
        self.which = patch(
            "installer_runtime.shutil.which",
            side_effect=lambda name: "/usr/bin/" + name,
        )
        self.which.start()
        self.addCleanup(self.which.stop)

    def configured(self):
        result = copy.deepcopy(self.request)
        defaults = {
            "domain": "example.test",
            "hostname": "mail.example.test",
            "ipv4": "192.0.2.10",
            "admin": "admin@example.test",
            "public_url": "https://media.example.test",
            "admin_email": "admin@example.test",
            "admin_name": "Installer fixture",
        }
        for field in self.spec["fields"]:
            if field["key"].endswith("password_file"):
                secret = self.folder / "password"
                secret.write_text("Disposable-only password 123!")
                secret.chmod(0o600)
                result["configuration"][field["key"]] = str(secret)
            elif field["required"]:
                result["configuration"][field["key"]] = defaults[field["key"]]
        return result

    def reviewed(self, request):
        plan = self.installer.plan(request)
        request["review"] = {"plan_sha256": plan["plan_sha256"]}
        return plan

    def test_fields_are_discoverable_without_secrets(self):
        plan = self.installer.plan(self.request)
        self.assertEqual(plan["configuration_fields"], self.spec["fields"])
        required = any(
            x["required"] and x.get("default", "") == "" for x in self.spec["fields"]
        )
        self.assertEqual(plan["ready_for_apply"], not required)
        if required:
            self.request["review"] = {"plan_sha256": plan["plan_sha256"]}
            with self.assertRaises(ProtocolError):
                self.installer.apply(self.request)

    def test_source_and_configuration_drift_require_new_review(self):
        request = self.configured()
        self.reviewed(request)
        (self.installer.source / "fixture.txt").write_text("Changed source")
        with self.assertRaises(ProtocolError):
            self.installer.apply(request)
        self.assertFalse(
            any(
                argv[0] in ("useradd", "systemctl", "/bin/bash")
                for argv, _ in self.calls
            )
        )

    def test_private_runtime_is_rejected_without_executing_its_version_probe(self):
        if self.spec["kind"] != "application":
            return
        self.installer.system_tool = lambda _: False
        plan = self.installer.plan(self.configured())
        self.assertFalse(plan["supported"])
        self.assertIn("system-wide", plan["reason"])
        self.assertEqual(self.calls, [])

    def test_adopt_does_not_need_setup_password_or_admin_fields(self):
        request = copy.deepcopy(self.request)
        request["mode"] = "adopt"
        plan = self.installer.plan(request)
        self.assertTrue(plan["ready_for_apply"])
        self.installer.health = lambda _: {
            "ok": False,
            "checks": [{"name": "service", "ok": False}],
        }
        request["review"] = {"plan_sha256": plan["plan_sha256"]}
        result = self.installer.apply(request)
        self.assertEqual(result["state"], "needs_recovery")
        self.assertFalse(result["changed"])
        self.assertFalse(self.installer.journal(request).exists())

    def test_healthy_adopt_records_binding_without_mutation(self):
        request = copy.deepcopy(self.request)
        request["mode"] = "adopt"
        self.reviewed(request)
        self.installer.health = lambda _: {
            "ok": True,
            "checks": [{"name": "service", "ok": True}],
        }
        result = self.installer.apply(request)
        self.assertEqual(result["state"], "completed")
        self.assertFalse(result["changed"])
        self.assertIn("data", result["binding"])
        self.assertEqual(result["binding"]["services"], self.spec["units"])
        self.assertIsNone(result["binding"]["reviewed_source_sha256"])
        self.assertIsNone(result["binding"]["reviewed_source_version"])
        self.assertFalse(result["binding"]["updates"]["supported"])
        self.assertEqual(self.installer.read_journal(request)["stage"], "adopted")

    def test_adoption_reads_recognized_service_paths_and_actual_port(self):
        if self.spec["kind"] != "application":
            self.assertIn(self.spec["kind"], ("native", "static"))
            return
        configuration = self.folder / "existing.env"
        data = self.folder / "existing-data"
        configuration.write_text(
            "AUTHENTACTOR_PORT=12123\nAUTHENTACTOR_DATA="
            + str(data)
            + "\nPRIVATE_SECRET=not-returned\n"
            if self.spec["project"] == "passwords"
            else "PORT=12123\nDATA_DIR=" + str(data) + "\nPRIVATE_SECRET=not-returned\n"
        )
        properties = {
            "WorkingDirectory": str(self.folder / "existing-code"),
            "EnvironmentFiles": str(configuration) + " (ignore_errors=no)",
            "FragmentPath": str(self.folder / "existing.service"),
            "ExecStart": "{ path=/usr/bin/fixture ; argv[]=/usr/bin/fixture ; }",
        }

        def service(argv, **options):
            key = next(
                (
                    value.removeprefix("--property=")
                    for value in argv
                    if value.startswith("--property=")
                ),
                "",
            )
            return subprocess.CompletedProcess(
                argv, 0, stdout=properties.get(key, ""), stderr=""
            )

        self.installer.run = service
        request = dict(self.request, mode="adopt")
        paths, port = self.installer.installed_binding(request)
        self.assertEqual(port, 12123)
        self.assertEqual(paths["data"], str(data))
        self.assertEqual(paths["code"], str(self.folder / "existing-code"))
        self.assertNotIn("PRIVATE_SECRET", json.dumps(paths))
        self.assertFalse(self.installer.journal(request).exists())

    def test_unknown_fields_and_unreviewed_apply_fail(self):
        request = self.configured()
        request["configuration"]["run_shell"] = "anything"
        with self.assertRaises(ProtocolError):
            self.installer.plan(request)
        with self.assertRaises(ProtocolError):
            self.installer.apply(self.configured())

    def test_existing_data_is_preserved(self):
        if self.spec["kind"] != "application":
            self.assertIn(self.spec["kind"], ("native", "static"))
            self.assertFalse(self.installer.plan(self.request)["destructive"])
            return
        request = self.configured()
        paths = self.installer.paths(request)
        data = Path(paths["data"])
        data.mkdir(parents=True)
        sentinel = data / "keep"
        sentinel.write_bytes(b"Existing user data")
        plan = self.installer.plan(request)
        self.assertFalse(plan["supported"])
        request["review"] = {"plan_sha256": plan["plan_sha256"]}
        with self.assertRaises(ProtocolError):
            self.installer.apply(request)
        self.assertEqual(sentinel.read_bytes(), b"Existing user data")

    def test_empty_cli_private_data_directory_is_supported(self):
        if self.spec["kind"] != "application":
            self.assertIn(self.spec["kind"], ("native", "static"))
            return
        request = self.configured()
        data = Path(self.installer.paths(request)["data"])
        data.mkdir(parents=True, mode=0o700)
        self.assertTrue(self.installer.plan(request)["ready_for_apply"])
        self.assertTrue(
            str(data).startswith(request["root"] + "/" + self.spec["project"] + "/")
        )

    def test_untrusted_workspace_parent_is_rejected_before_commands(self):
        parent = self.folder / "shared"
        parent.mkdir(mode=0o777)
        parent.chmod(0o777)
        request = self.configured()
        request["root"] = str(parent / "workspace")
        with self.assertRaises(ProtocolError):
            self.installer.plan(request)
        self.assertEqual(self.calls, [])

    def test_symlink_source_is_rejected(self):
        (self.installer.source / "fixture.txt").unlink()
        (self.installer.source / "fixture.txt").symlink_to("/etc/hostname")
        with self.assertRaises(ProtocolError):
            self.installer.plan(self.configured())

    def test_interactive_setup_requires_real_terminal_before_child(self):
        if not self.spec.get("interactive"):
            self.assertFalse(self.installer.plan(self.configured())["interactive"])
            return
        request = self.configured()
        plan = self.reviewed(request)
        with (
            patch("installer_runtime.sys.stdin.isatty", return_value=False),
            patch("installer_runtime.sys.stdout.isatty", return_value=False),
        ):
            result = self.installer.apply(request)
        self.assertEqual(result["state"], "needs_recovery")
        self.assertFalse(
            any(
                argv[0] in ("/bin/bash", self.spec.get("installed_entrypoint"))
                for argv, _ in self.calls
            )
        )

    def test_zero_exit_without_health_never_completes(self):
        request = self.configured()
        self.reviewed(request)
        self.installer.native = lambda *_: None
        self.installer.application = lambda *_: None
        self.installer.health = lambda _: {
            "ok": False,
            "checks": [{"name": "readiness", "ok": False}],
        }
        result = self.installer.apply(request)
        self.assertEqual(result["state"], "needs_recovery")
        self.assertEqual(
            self.installer.read_journal(request)["stage"], "needs_recovery"
        )

    def test_completed_setup_reports_runtime_binding_not_a_source_symlink_upgrade(self):
        request = self.configured()
        plan = self.reviewed(request)
        self.installer.native = lambda *_: None
        self.installer.application = lambda *_: None
        self.installer.health = lambda _: {
            "ok": True,
            "checks": [{"name": "fixture", "ok": True}],
            "observed_version": "5.0.2",
        }
        result = self.installer.apply(request)
        self.assertEqual(result["state"], "completed")
        binding = result["binding"]
        self.assertEqual(binding["code"], plan["paths"]["code"])
        self.assertEqual(binding["reviewed_source_sha256"], plan["source_sha256"])
        self.assertEqual(binding["reviewed_source_version"], self.spec["version"])
        self.assertEqual(binding["observed_runtime_version"], "5.0.2")
        self.assertFalse(binding["updates"]["supported"])

    def test_recovery_requires_owned_journal_and_same_configuration(self):
        request = self.configured()
        request["intent"] = "recover"
        self.assertFalse(self.installer.plan(request)["ready_for_apply"])
        request.pop("intent")
        self.reviewed(request)
        self.installer.native = lambda *_: None
        self.installer.application = lambda *_: None
        self.installer.health = lambda _: {"ok": False, "checks": []}
        self.installer.apply(request)
        request["intent"] = "recover"
        self.assertTrue(self.installer.plan(request)["ready_for_apply"])
        request["operation_id"] = "2" * 32
        self.assertFalse(self.installer.plan(request)["ready_for_apply"])

    def test_application_builds_are_unprivileged_and_failed_build_is_recoverable(self):
        # Exercise the real orchestration with a disposable path set and injected
        # process boundary. No dependency installer, service or account is run.
        self.installer.spec.update(
            project="passwords",
            kind="application",
            user="fixture-service",
            units=["fixture-web"],
            fields=[
                {"key": "port", "type": "integer", "default": 8092, "required": False}
            ],
            dependencies=["systemctl", "python3", "runuser", "useradd"],
            paths={
                "code": str(self.folder / "legacy/code"),
                "data": str(self.folder / "legacy/data"),
                "configuration": str(self.folder / "legacy/service.env"),
                "unit": str(self.folder / "fixture.service"),
            },
        )
        self.installer.spec.pop("installed_entrypoint", None)
        request = dict(
            self.request, project="passwords", mode="application", configuration={}
        )
        self.reviewed(request)
        real_run = self.installer.run

        def fail_build(argv, **options):
            if "pip" in argv:
                self.calls.append((argv, options))
                return subprocess.CompletedProcess(argv, 1, stdout="", stderr="")
            return real_run(argv, **options)

        self.installer.run = fail_build
        identity = SimpleNamespace(
            pw_uid=65534, pw_gid=65534, pw_shell="/usr/sbin/nologin"
        )
        with (
            patch("installer_runtime.pwd.getpwnam", return_value=identity),
            patch("installer_runtime.os.chown"),
        ):
            result = self.installer.apply(request)
        self.assertEqual(result["state"], "needs_recovery")
        build = [argv for argv, _ in self.calls if "pip" in argv or "venv" in argv]
        self.assertEqual(result["binding"]["managed_by"], "project-installer")
        self.assertFalse(result["binding"]["updates"]["supported"])
        self.assertIsNone(result["binding"]["observed_runtime_version"])
        self.assertEqual(len(build), 2)
        self.assertTrue(
            all(
                argv[:4] == ["runuser", "-u", "fixture-service", "--"] for argv in build
            )
        )
        self.assertIn("--only-binary=:all:", build[-1])
        self.assertFalse(
            any(argv[:2] == ["systemctl", "enable"] for argv, _ in self.calls)
        )
        journal = self.installer.read_journal(request)
        self.assertEqual(journal["stage"], "needs_recovery")
        self.assertIn("python-environment", journal["done"])
        self.assertNotIn("python-dependencies", journal["done"])
        self.assertFalse(Path(self.installer.spec["paths"]["unit"]).exists())

    def test_request_parser_rejects_duplicate_keys_and_public_permissions(self):
        path = self.folder / "request.json"
        path.write_text(json.dumps(self.request))
        path.chmod(0o644)
        with self.assertRaises(ProtocolError):
            read_request(path, self.spec["project"])
        path.chmod(0o600)
        path.write_text('{"schema":1,"schema":1}')
        with self.assertRaises((ProtocolError, ValueError)):
            read_request(path, self.spec["project"])


class ProcessTests(unittest.TestCase):
    def test_successful_parent_cannot_leave_a_background_build_child(self):
        with tempfile.TemporaryDirectory() as folder:
            pidfile = Path(folder) / "child"
            source = "import subprocess,sys;from pathlib import Path;p=subprocess.Popen([sys.executable,'-c','import signal,time;signal.signal(signal.SIGTERM,signal.SIG_IGN);time.sleep(60)'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL);Path(sys.argv[1]).write_text(str(p.pid))"
            result = OwnedRunner()(
                [sys.executable, "-c", source, str(pidfile)],
                capture_output=True,
                timeout=3,
            )
            self.assertEqual(result.returncode, 0)
            proc = Path("/proc") / pidfile.read_text() / "stat"
            for _ in range(40):
                try:
                    if proc.read_text().split()[2] == "Z":
                        break
                except (FileNotFoundError, ProcessLookupError):
                    break
                time.sleep(0.05)
            else:
                self.fail("Successful build left its background child running")

    def test_capture_and_timeout_reap_only_owned_process_group(self):
        runner = OwnedRunner()
        response = runner(
            [sys.executable, "-c", "print('fixture output')"],
            capture_output=True,
            text=True,
            timeout=3,
        )
        self.assertEqual(response.returncode, 0)
        self.assertEqual(response.stdout.strip(), "fixture output")
        with tempfile.TemporaryDirectory() as folder:
            pidfile = Path(folder) / "child"
            source = "import subprocess,sys,time;from pathlib import Path;p=subprocess.Popen([sys.executable,'-c','import time;time.sleep(60)']);Path(sys.argv[1]).write_text(str(p.pid));time.sleep(60)"
            with self.assertRaises(subprocess.TimeoutExpired):
                runner(
                    [sys.executable, "-c", source, str(pidfile)],
                    capture_output=True,
                    timeout=0.3,
                )
            pid = int(pidfile.read_text())
            for _ in range(40):
                proc = Path("/proc") / str(pid) / "stat"
                try:
                    if proc.read_text().split()[2] == "Z":
                        break
                except (FileNotFoundError, ProcessLookupError):
                    break
                time.sleep(0.05)
            else:
                self.fail("Owned child remained running after timeout")


if __name__ == "__main__":
    unittest.main()
