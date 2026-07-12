"""Thin Python wrapper over the ordo binary.

Binary resolution: ORDO_BIN env → vendored (shipped in the wheel) → `ordo` on PATH.
"""
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

__all__ = ["order", "review", "binary_path"]


def binary_path() -> str:
    env = os.environ.get("ORDO_BIN")
    if env:
        return env
    exe = "ordo.exe" if sys.platform == "win32" else "ordo"
    vendored = Path(__file__).parent / "vendor" / exe
    if vendored.exists():
        return str(vendored)
    found = shutil.which("ordo")
    if found:
        return found
    raise FileNotFoundError("ordo binary not found; set ORDO_BIN or install a prebuilt")


def _run(args, stdin: str) -> dict:
    r = subprocess.run(
        [binary_path(), *args], input=stdin, capture_output=True, text=True
    )
    if r.returncode != 0:
        raise RuntimeError(f"ordo exited {r.returncode}: {r.stderr.strip()}")
    return json.loads(r.stdout)


def order(input: dict) -> dict:
    """Order a changeset (schema/v1 input) → v1 output."""
    return _run(["order", "--json"], json.dumps(input))


def review(patch: str) -> dict:
    """Order a git/unified diff string → v1 output."""
    return _run(["review"], patch)
