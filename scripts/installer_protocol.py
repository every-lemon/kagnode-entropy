"""Shared, vendorable KagNode installer protocol. No application business rules."""

from __future__ import annotations
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import tempfile


class ProtocolError(ValueError):
    pass


def canonical(value):
    return json.dumps(
        value,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
        allow_nan=False,
    ).encode()


def load_json(raw):
    def unique(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ProtocolError("Duplicate JSON field")
            result[key] = value
        return result

    def invalid(_):
        raise ProtocolError("Non-finite JSON number")

    return json.loads(raw, object_pairs_hook=unique, parse_constant=invalid)


def plan_digest(plan):
    return hashlib.sha256(
        canonical({k: v for k, v in plan.items() if k != "plan_sha256"})
    ).hexdigest()


def read_request(path, project):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        info = os.fstat(fd)
        if (
            not stat.S_ISREG(info.st_mode)
            or info.st_mode & 0o077
            or info.st_nlink != 1
            or info.st_uid != os.geteuid()
            or info.st_size > 65536
        ):
            raise ProtocolError(
                "Request must be a private regular file owned by the current user"
            )
        raw = os.read(fd, 65537)
    finally:
        os.close(fd)
    value = load_json(raw)
    if (
        not isinstance(value, dict)
        or type(value.get("schema")) is not int
        or value.get("schema") != 1
        or value.get("project") != project
        or not re.fullmatch(r"[a-f0-9]{32}", str(value.get("operation_id", "")))
    ):
        raise ProtocolError("Invalid project or operation")
    if value.get("mode") not in ("native", "application", "adopt") or not isinstance(
        value.get("configuration", {}), dict
    ):
        raise ProtocolError("Invalid installation mode or configuration")
    raw_root = value.get("root", "")
    if not isinstance(raw_root, str) or not re.fullmatch(
        r"/(?:[A-Za-z0-9._-]+/)*[A-Za-z0-9._-]+", raw_root
    ):
        raise ProtocolError(
            "Workspace root must use absolute ASCII path components without whitespace or controls"
        )
    root = Path(raw_root)
    if str(root) != raw_root:
        raise ProtocolError("Workspace root must be canonical")
    if (
        not root.is_absolute()
        or ".." in root.parts
        or str(root) in ("/", "/etc", "/usr", "/var", "/opt", "/srv")
        or any(p.is_symlink() for p in [root, *root.parents])
    ):
        raise ProtocolError("Unsafe workspace root")
    return value


def reviewed_plan(request, plan):
    expected = plan_digest(plan)
    if (
        request.get("review", {}).get("plan_sha256") != expected
        or plan.get("plan_sha256") != expected
    ):
        raise ProtocolError("The installation plan changed; review it again")
    if (
        not plan.get("supported")
        or plan.get("ready_for_apply") is False
        or (plan.get("manual_steps") and not plan.get("interactive"))
    ):
        raise ProtocolError(
            "Required manual steps or unsupported capability prevent automatic apply"
        )
    return plan


def write_result(path, value):
    path = Path(path)
    if path.exists() or path.is_symlink():
        raise ProtocolError("Output must not already exist")
    parent = path.parent
    info = parent.stat()
    if parent.is_symlink() or info.st_mode & 0o077 or info.st_uid != os.geteuid():
        raise ProtocolError(
            "Output directory must be private and owned by current user"
        )
    fd, temp = tempfile.mkstemp(prefix=".result-", dir=parent)
    try:
        with os.fdopen(fd, "wb") as stream:
            stream.write(canonical(value) + b"\n")
            stream.flush()
            os.fsync(stream.fileno())
        # Exclusive link prevents replacement of a concurrent output.
        os.link(temp, path)
    finally:
        os.unlink(temp)
