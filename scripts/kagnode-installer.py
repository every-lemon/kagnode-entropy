#!/usr/bin/env python3
"""Explicit local setup entrypoint; never run by source package hooks."""

import json
from pathlib import Path
from installer_runtime import Installer

if __name__ == "__main__":
    source = Path(__file__).resolve().parents[1]
    specification = json.loads((source / "scripts/installer.json").read_text())
    raise SystemExit(Installer(source, specification).main())
