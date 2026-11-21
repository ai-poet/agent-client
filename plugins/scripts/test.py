"""Run all deterministic plugin and example tests."""

from pathlib import Path
import argparse
import os
import subprocess


ROOT = Path(__file__).resolve().parents[1]


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runtime-host", type=Path,
                        help="Use a prebuilt production script test host; never builds it")
    args = parser.parse_args()
    env = os.environ.copy()
    # Runtime execution must be explicitly selected for this invocation.
    env.pop("SAILRY_PLUGIN_TEST_HOST", None)
    if args.runtime_host:
        host = args.runtime_host.resolve()
        if not host.is_file() or not os.access(host, os.X_OK):
            parser.error("--runtime-host must name an executable file")
        env["SAILRY_PLUGIN_TEST_HOST"] = str(host)
    tests = sorted(path for path in ROOT.rglob("*.test.mjs")
                   if "node_modules" not in path.parts)
    if not tests:
        raise SystemExit("No plugin tests found")
    raise SystemExit(subprocess.run(
        ["node", "--experimental-vm-modules", "--test", "--test-reporter=spec",
         *(str(path.relative_to(ROOT)) for path in tests)], cwd=ROOT, env=env,
    ).returncode)
