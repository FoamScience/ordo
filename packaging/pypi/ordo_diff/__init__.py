"""Thin Python wrapper over the ordo binaries.

Binary resolution: ORDO_BIN_DIR env → vendored (shipped in the wheel). There is
no PATH fallback: the first `ordo` on PATH is this package's own console script.
"""
import json
import os
import subprocess
import sys
from pathlib import Path

__all__ = ["order", "review", "binary_path"]


def binary_path(name: str = "ordo-engine") -> str:
    """Where binary `name` (`ordo` or `ordo-engine`) lives."""
    exe = f"{name}.exe" if sys.platform == "win32" else name
    env = os.environ.get("ORDO_BIN_DIR")
    if env:
        return str(Path(env) / exe)
    vendored = Path(__file__).parent / "vendor" / exe
    if vendored.exists():
        return str(vendored)
    raise FileNotFoundError(
        f"{exe} is not in this install (no wheel for this platform?); "
        "set ORDO_BIN_DIR to a directory holding it"
    )


def _run(args, stdin: str) -> dict:
    r = subprocess.run(
        [binary_path(), *args], input=stdin, capture_output=True, text=True
    )
    if r.returncode != 0:
        raise RuntimeError(f"ordo-engine exited {r.returncode}: {r.stderr.strip()}")
    return json.loads(r.stdout)


def order(input: dict) -> dict:
    """Order a changeset (schema/v2.json input) → schema v2 output."""
    return _run(["order", "--json"], json.dumps(input))


def review(patch: str) -> dict:
    """Order a git/unified diff string → schema v2 output."""
    return _run(["review"], patch)
