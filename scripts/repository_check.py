#!/usr/bin/env python3
"""Validate documentation, tracked-source inventory and publication boundaries."""

import json
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from release import identity, source_files

HEADINGS = [
    "Overview",
    "Release status",
    "Features",
    "Requirements",
    "Installation and downloads",
    "Usage",
    "Configuration and storage",
    "Security",
    "Development and testing",
    "Documentation",
    "Licensing",
]


def check():
    cfg, product, preview = identity()
    readme = (ROOT / "README.md").read_text()
    expected = (
        f"**{product}** is the coordinated stable"
        if cfg.get("channel") == "stable"
        else f"Product source: **{product}**. Packaging preview: **{preview}**."
    )
    if expected not in readme:
        raise ValueError("README release status differs from the package identity")
    if re.findall(r"^## (.+)$", readme, re.M) != HEADINGS:
        raise ValueError("README sections differ from the shared template")
    files = source_files()
    indexed = sorted(
        subprocess.check_output(["git", "ls-files"], cwd=ROOT, text=True).splitlines()
    )
    if indexed != [name for name, _ in files]:
        missing = sorted(set(indexed) ^ {name for name, _ in files})
        raise ValueError("Refresh the reviewed source allowlist: " + str(missing))
    for name, path in files:
        if not name.endswith(".md"):
            continue
        # Retained historical operations text may refer to past release paths.
        if name == "docs/OPERATIONS.md":
            continue
        for target in re.findall(r"\]\(([^)\s]+)", path.read_text()):
            if target.startswith(
                ("https:", "http:", "mailto:", "#")
            ) or target.startswith("<"):
                continue
            target = target.split("#")[0]
            if target and not (path.parent / target).exists():
                raise ValueError(f"{name}: missing link {target}")
    print(
        f"{cfg['repository']}: README, links and {len(files)} allowlisted source files verified"
    )


if __name__ == "__main__":
    check()
