"""One helper at a time, and smooth updates.

Only one helper may run (they'd fight over the same files). When a newer helper starts while an
older one is running, it asks the old one to quit (by creating quit.request in the app folder),
waits for it to let go of the lock, and takes over. An older helper never replaces a newer one.
"""
from __future__ import annotations

import json
import os
import re
import sys
import time

from .paths import app_dir


def vtuple(v: str | None) -> tuple[int, ...]:
    return tuple(int(x) for x in re.findall(r"\d+", v or "")[:3])


def _running_file():
    return app_dir() / "running.json"


def _quit_file():
    return app_dir() / "quit.request"


class SingleInstance:
    """Holds an exclusive lock for as long as the process lives (or until release())."""

    def __init__(self):
        self.path = app_dir() / "helper.lock"
        self.fh = None

    def acquire(self) -> bool:
        if self.fh is None:
            self.fh = open(self.path, "a+")
        try:
            if sys.platform == "win32":
                import msvcrt
                self.fh.seek(0)
                msvcrt.locking(self.fh.fileno(), msvcrt.LK_NBLCK, 1)
            else:
                import fcntl
                fcntl.flock(self.fh.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
            return True
        except OSError:
            return False

    def release(self) -> None:
        if self.fh is None:
            return
        try:
            if sys.platform == "win32":
                import msvcrt
                self.fh.seek(0)
                msvcrt.locking(self.fh.fileno(), msvcrt.LK_UNLCK, 1)
            else:
                import fcntl
                fcntl.flock(self.fh.fileno(), fcntl.LOCK_UN)
        except OSError:
            pass
        self.fh.close()
        self.fh = None


def write_running(version: str) -> None:
    try:
        _running_file().write_text(json.dumps({"version": version, "pid": os.getpid(), "t": time.time()}))
    except OSError:
        pass


def running_version() -> str | None:
    try:
        return json.loads(_running_file().read_text()).get("version")
    except (OSError, ValueError):
        return None


def quit_requested() -> bool:
    return _quit_file().exists()


def clear_quit_request() -> None:
    try:
        _quit_file().unlink()
    except OSError:
        pass


def take_over(lock: SingleInstance, my_version: str, timeout: float = 15.0) -> tuple[bool, str | None]:
    """Get the lock, asking an older running helper to quit if needed.

    Returns (got_lock, version_of_the_other_helper_or_None)."""
    if lock.acquire():
        clear_quit_request()
        return True, None
    other = running_version()
    if other and vtuple(other) > vtuple(my_version):
        return False, other  # never replace a newer helper
    try:
        _quit_file().write_text(my_version)
    except OSError:
        return False, other
    deadline = time.time() + timeout
    while time.time() < deadline:
        time.sleep(0.5)
        if lock.acquire():
            clear_quit_request()
            return True, other
    clear_quit_request()
    return False, other
