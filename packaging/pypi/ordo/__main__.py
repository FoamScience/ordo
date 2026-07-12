"""`ordo` console entry point — exec the resolved binary with the same args."""
import subprocess
import sys

from . import binary_path


def main() -> None:
    sys.exit(subprocess.run([binary_path(), *sys.argv[1:]]).returncode)


if __name__ == "__main__":
    main()
