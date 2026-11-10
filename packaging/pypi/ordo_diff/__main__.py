"""`ordo` and `ordo-engine` console entry points: run the binary with the same args."""
import subprocess
import sys

from . import binary_path


def _exec(name: str) -> None:
    try:
        binary = binary_path(name)
    except FileNotFoundError as e:
        sys.exit(f"{name}: {e}")
    sys.exit(subprocess.run([binary, *sys.argv[1:]]).returncode)


def ordo() -> None:
    _exec("ordo")


def engine() -> None:
    _exec("ordo-engine")


if __name__ == "__main__":
    engine()
