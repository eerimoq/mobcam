"""The few things the workflow scripts need from the runner itself.

GitHub Actions passes values from one step to the next through the files
GITHUB_OUTPUT and GITHUB_ENV point at, and reads annotations off stdout. None
of that is worth repeating in every script, and none of it has to be there for
a script to be run by hand.
"""

import contextlib
import json
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


class Error(Exception):
    """Anything the workflow author can fix, reported without a traceback."""


def _append(variable, name, value):
    path = os.environ.get(variable)

    if path is None:
        # Run outside a workflow, where printing is all that can be done.
        print(f"{name}={value}", flush=True)
        return

    with open(path, "a", encoding="utf-8") as fout:
        fout.write(f"{name}={value}\n")


def output(name, value):
    _append("GITHUB_OUTPUT", name, value)


def env(name, value):
    _append("GITHUB_ENV", name, value)


def flag(name):
    """A boolean the workflow passed in. Anything the workflow left unset is
    false, which is what turns signing and packaging off on a fork."""

    return os.environ.get(name, "") == "true"


def boolean(value):
    return str(bool(value)).lower()


def notice(message):
    print(f"::notice::{message}", flush=True)


@contextlib.contextmanager
def group(title):
    print(f"::group::{title}", flush=True)

    try:
        yield
    finally:
        print("::endgroup::", flush=True)


def run(command, quiet=False, **kwargs):
    """Run a command, and fail the step if it does. Secrets end up on the
    command line here, so quiet leaves the arguments out of the log."""

    command = [str(argument) for argument in command]

    if quiet:
        print(f"    {' '.join(command[:2])} ...", flush=True)
    else:
        print(f"    {' '.join(command)}", flush=True)

    try:
        return subprocess.run(command, check=True, **kwargs)
    except FileNotFoundError:
        raise Error(f"{command[0]} not found")
    except subprocess.CalledProcessError as error:
        raise Error(f"{command[0]} failed with exit code {error.returncode}")


def python(*arguments):
    """Call build.py with the interpreter that is running this script."""

    return run([sys.executable, ROOT / "build.py", *arguments])


def host_platform():
    if sys.platform == "darwin":
        return "macos"
    elif sys.platform == "win32":
        return "windows"
    elif sys.platform.startswith("linux"):
        return "linux"
    else:
        raise Error(f"unsupported platform {sys.platform}")


def buildspec():
    with open(ROOT / "buildspec.json", encoding="utf-8") as fin:
        return json.load(fin)


def main(function):
    """Every script is run for its side effects and reports what the user can
    fix as an annotation rather than as a traceback."""

    try:
        function()
    except Error as error:
        print(f"::error::{error}", flush=True)
        sys.exit(2)
