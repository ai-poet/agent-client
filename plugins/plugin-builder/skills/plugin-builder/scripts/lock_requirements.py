"""Generate the standard hash-locked dependency file for a plugin."""
import argparse
from pathlib import Path
import subprocess
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path, help="Dependency input file")
    parser.add_argument("package", type=Path, help="Plugin package directory")
    args = parser.parse_args()
    package = args.package.resolve(strict=True)
    if not (package / "plugin.json").is_file():
        parser.error("package directory must contain plugin.json")
    source = args.input.resolve(strict=True)
    subprocess.run(
        [sys.executable, "-m", "piptools", "compile", "--generate-hashes",
         "--no-header", "--no-annotate", "--no-emit-options",
         "--pip-args=--only-binary=:all:",
         "--output-file=" + str(package / "requirements.txt"), str(source)],
        check=True,
    )


if __name__ == "__main__":
    main()
