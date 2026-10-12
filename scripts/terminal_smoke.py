#!/usr/bin/env python3
"""Native PTY/ConPTY evidence, not a GUI terminal or native-graphics certification.

Requires pyte==0.8.2, wcwidth==0.2.13, and (Windows) pywinpty==2.0.15.
The supplied binary must already be built. One isolated cargo example build
exercises the real terminal module's error and panic paths; repository files
and the user's terminal are never modified. All transcripts are local artifacts.
"""

from __future__ import annotations

import argparse
import codecs
import ctypes
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path
import platform
import queue
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
import unicodedata

ESC = "\x1b"
RIGHT = ESC + "[C"
FORMS = ("Eevee", "Vaporeon", "Jolteon", "Flareon", "Espeon", "Umbreon",
         "Leafeon", "Glaceon", "Sylveon")
WINDOWS = os.name == "nt"
CONSOLE_MARKER = "EEVEE_CONSOLE_RESTORATION="


def save_json(path, value):
    Path(path).write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def run_owned(argv, *, timeout, cwd=None, env=None, text=False):
    """Bound command duration and own descendants, including cargo/rustc."""
    command = ([sys.executable, str(Path(__file__).resolve()), "--owned-command", "--", *argv]
               if WINDOWS else argv)
    process = subprocess.Popen(command, cwd=cwd, env=env, stdin=subprocess.PIPE,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               start_new_session=not WINDOWS)
    job = None
    try:
        if WINDOWS:
            job = WindowsJob(process.pid)
        stdout, stderr = process.communicate(b"EEVEE_SMOKE_START\n" if WINDOWS else b"", timeout=timeout)
        if text:
            stdout, stderr = stdout.decode("utf-8", "replace"), stderr.decode("utf-8", "replace")
        return subprocess.CompletedProcess(argv, process.returncode, stdout, stderr)
    finally:
        if WINDOWS:
            if job:
                job.close()
            elif process.poll() is None:
                process.kill()
        else:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        process.wait(timeout=5)


def command_version(argv):
    try:
        result = run_owned(argv, text=True, timeout=15)
        return {"argv": argv, "status": result.returncode,
                "stdout": result.stdout.strip(), "stderr": result.stderr.strip()}
    except (OSError, subprocess.TimeoutExpired) as error:
        return {"argv": argv, "unavailable": str(error)}


def host_evidence():
    machine = platform.machine().lower()
    arch = {"aarch64": "arm64", "amd64": "x86_64"}.get(machine, machine)
    versions = {}
    for package in ("pyte", "wcwidth", "pywinpty"):
        try:
            versions[package] = importlib.metadata.version(package)
        except importlib.metadata.PackageNotFoundError:
            versions[package] = "not installed"
    return {
        "os": platform.system(), "arch": arch, "platform": platform.platform(),
        "uname": list(platform.uname()), "python": sys.version,
        "macos_version": platform.mac_ver()[0],
        "windows_version": list(platform.win32_ver()),
        "linux_distribution": platform.freedesktop_os_release() if platform.system() == "Linux" else None,
        "rustc": command_version(["rustc", "-vV"]),
        "cargo": command_version(["cargo", "--version"]), "packages": versions,
        "runner": {key: os.environ.get(key) for key in
                   ("RUNNER_OS", "RUNNER_ARCH", "NATIVE_RUNNER_LABEL", "ImageOS", "ImageVersion")},
        "terminal_backend": "Windows ConPTY (pywinpty backend=0)" if WINDOWS else "native POSIX PTY",
        "screen_model": "pyte", "gui_visual_acceptance": "not exercised; separate manual gate",
        "native_images": "not exercised or claimed",
        "minimum_os": "only the observed host version is exercised; older OS versions are unqualified",
    }


def console_modes():
    from ctypes import wintypes
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.GetStdHandle.argtypes = [wintypes.DWORD]
    kernel.GetStdHandle.restype = wintypes.HANDLE
    kernel.GetConsoleMode.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)]
    modes = []
    for kind in (-10, -11):
        mode = wintypes.DWORD()
        if not kernel.GetConsoleMode(kernel.GetStdHandle(kind & 0xFFFFFFFF), ctypes.byref(mode)):
            raise ctypes.WinError(ctypes.get_last_error())
        modes.append(mode.value)
    return modes

def primary_screen_marker(write=False):
    """Read the active native buffer; ConPTY need not relay CSI 1049."""
    from ctypes import wintypes
    class Coordinate(ctypes.Structure):
        _fields_ = [("x", wintypes.SHORT), ("y", wintypes.SHORT)]
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD,
                                  wintypes.LPVOID, wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE]
    kernel.CreateFileW.restype = wintypes.HANDLE
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel.ReadConsoleOutputCharacterW.argtypes = [wintypes.HANDLE, wintypes.LPWSTR,
                                                  wintypes.DWORD, Coordinate,
                                                  ctypes.POINTER(wintypes.DWORD)]
    kernel.WriteConsoleOutputCharacterW.argtypes = [wintypes.HANDLE, wintypes.LPCWSTR,
                                                   wintypes.DWORD, Coordinate,
                                                   ctypes.POINTER(wintypes.DWORD)]
    marker = "EEVEE_PRIMARY_SENTINEL"
    handle = kernel.CreateFileW("CONOUT$", 0xC0000000, 3, None, 3, 0, None)
    if handle == ctypes.c_void_p(-1).value:
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        count = wintypes.DWORD()
        if write:
            if not kernel.WriteConsoleOutputCharacterW(handle, marker, len(marker),
                                                       Coordinate(0, 0), ctypes.byref(count)):
                raise ctypes.WinError(ctypes.get_last_error())
            assert count.value == len(marker), "primary console sentinel was not written completely"
        text = ctypes.create_unicode_buffer(len(marker) + 1)
        if not kernel.ReadConsoleOutputCharacterW(handle, text, len(marker),
                                                  Coordinate(0, 0), ctypes.byref(count)):
            raise ctypes.WinError(ctypes.get_last_error())
        return text.value
    finally:
        kernel.CloseHandle(handle)


def console_child(argv):
    """Observe the child's *same native console*, not the CI runner's handles."""
    # The parent assigns this wrapper to its kill-on-close Job before releasing
    # the gate. Every subsequently spawned binary inherits process ownership.
    if sys.stdin.readline().strip() != "EEVEE_SMOKE_START":
        return 2
    before = console_modes()
    primary_before = primary_screen_marker(write=True)
    primary_hidden = False
    observed = set()
    child = subprocess.Popen(argv)
    try:
        while child.poll() is None:
            observed.add(tuple(console_modes()))
            if primary_screen_marker() != primary_before:
                primary_hidden = True
            time.sleep(0.01)
        status = child.wait()
    finally:
        if child.poll() is None:
            child.kill()
            child.wait()
    after = console_modes()
    evidence = {"before": before, "observed": sorted(observed),
                "after": after, "child_status": status,
                "primary_before": primary_before,
                "primary_after": primary_screen_marker(),
                "primary_hidden_during_child": primary_hidden}
    save_json(os.environ["EEVEE_CONSOLE_EVIDENCE"], evidence)
    # A short marker survives ConPTY's screen wrapping; structured evidence is
    # written separately instead of scraping a possibly reflowed JSON line.
    print(CONSOLE_MARKER + "recorded", flush=True)
    return status


class WindowsJob:
    """Own wrapper and descendants; closing the job kills any surviving child."""

    def __init__(self, pid):
        from ctypes import wintypes
        class Basic(ctypes.Structure):
            _fields_ = [("ProcessTime", ctypes.c_int64), ("JobTime", ctypes.c_int64),
                        ("Flags", wintypes.DWORD), ("MinWorkingSet", ctypes.c_size_t),
                        ("MaxWorkingSet", ctypes.c_size_t), ("ActiveProcesses", wintypes.DWORD),
                        ("Affinity", ctypes.c_size_t), ("Priority", wintypes.DWORD),
                        ("Scheduling", wintypes.DWORD)]
        class Counters(ctypes.Structure):
            _fields_ = [(name, ctypes.c_uint64) for name in
                        ("ReadOps", "WriteOps", "OtherOps", "ReadBytes", "WriteBytes", "OtherBytes")]
        class Extended(ctypes.Structure):
            _fields_ = [("Basic", Basic), ("IO", Counters), ("ProcessMemory", ctypes.c_size_t),
                        ("JobMemory", ctypes.c_size_t), ("PeakProcess", ctypes.c_size_t),
                        ("PeakJob", ctypes.c_size_t)]
        self.kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        self.kernel.CreateJobObjectW.argtypes = [ctypes.c_void_p, wintypes.LPCWSTR]
        self.kernel.CreateJobObjectW.restype = wintypes.HANDLE
        self.kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
        self.kernel.OpenProcess.restype = wintypes.HANDLE
        self.kernel.CloseHandle.argtypes = [wintypes.HANDLE]
        self.kernel.SetInformationJobObject.argtypes = [wintypes.HANDLE, ctypes.c_int,
                                                       ctypes.c_void_p, wintypes.DWORD]
        self.kernel.AssignProcessToJobObject.argtypes = [wintypes.HANDLE, wintypes.HANDLE]
        self.handle = self.kernel.CreateJobObjectW(None, None)
        if not self.handle:
            raise ctypes.WinError(ctypes.get_last_error())
        limits = Extended()
        limits.Basic.Flags = 0x2000  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        process = self.kernel.OpenProcess(0x0100 | 0x0001, False, pid)
        try:
            if not process or not self.kernel.SetInformationJobObject(
                    self.handle, 9, ctypes.byref(limits), ctypes.sizeof(limits)):
                raise ctypes.WinError(ctypes.get_last_error())
            if not self.kernel.AssignProcessToJobObject(self.handle, process):
                raise ctypes.WinError(ctypes.get_last_error())
        except BaseException:
            self.close()
            raise
        finally:
            if process:
                self.kernel.CloseHandle(process)

    def close(self):
        if self.handle:
            self.kernel.CloseHandle(self.handle)
            self.handle = None


class Session:
    def __init__(self, argv, cwd, environment, size, directory):
        import pyte
        self.directory = directory
        directory.mkdir(parents=True, exist_ok=True)
        self.events = [{"argv": argv, "size": list(size), "environment": {
            key: environment.get(key) for key in
            ("TERM", "COLORTERM", "LC_ALL", "LC_CTYPE", "LANG", "NO_COLOR", "PYWINPTY_BACKEND")}}]
        self.text = ""
        class TerminalScreen(pyte.Screen):
            def window_manipulation(self, operation, lines=0, columns=0):
                # ConPTY announces its resize in the output stream. Apply it
                # at that boundary, not before draining old-size output.
                if operation == 8 and lines > 0 and columns > 0:
                    self.resize(lines=lines, columns=columns)

        class TerminalStream(pyte.Stream):
            csi = {**pyte.Stream.csi, "t": "window_manipulation"}
            events = pyte.Stream.events | {"window_manipulation"}

        self.screen = TerminalScreen(*size)
        self.stream = TerminalStream(self.screen)
        self.decoder = codecs.getincrementaldecoder("utf-8")("replace")
        self.mode_before = None
        self.mode_raw = None
        self.closed = False
        self.eof = False
        self.job = None
        self.master = self.slave = None
        self.data = queue.Queue()
        if WINDOWS:
            from winpty import PtyProcess
            environment = dict(environment)
            environment["EEVEE_CONSOLE_EVIDENCE"] = str(directory / "native-console.json")
            wrapper = [sys.executable, str(Path(__file__).resolve()), "--console-child", "--", *argv]
            self.process = PtyProcess.spawn(wrapper, cwd=str(cwd), env=environment,
                                            dimensions=(size[1], size[0]), backend=0)
            try:
                self.job = WindowsJob(self.process.pid)
                self.process.write("EEVEE_SMOKE_START\r")
            except BaseException:
                self.process.terminate(force=True)
                raise
        else:
            import fcntl
            import pty
            import struct
            import termios
            self.master, self.slave = pty.openpty()
            fcntl.ioctl(self.slave, termios.TIOCSWINSZ, struct.pack("HHHH", size[1], size[0], 0, 0))
            self.mode_before = termios.tcgetattr(self.slave)
            self.process = subprocess.Popen(argv, cwd=cwd, env=environment,
                                            stdin=self.slave, stdout=self.slave, stderr=self.slave,
                                            start_new_session=True)
        self.reader = threading.Thread(target=self._reader, daemon=True)
        self.reader.start()

    def _reader(self):
        try:
            while True:
                data = self.process.read(65536) if WINDOWS else os.read(self.master, 65536)
                if not data:
                    break
                self.data.put(data)
        except (OSError, EOFError) as error:
            self.data.put({"reader_end": str(error)})
        finally:
            self.data.put(None)

    def pump(self, duration=0.25, settle=False):
        deadline = time.monotonic() + duration
        maximum = deadline + 2 if settle else deadline
        last_output = time.monotonic()
        while time.monotonic() < maximum:
            now = time.monotonic()
            horizon = max(deadline, last_output + 0.12) if settle else deadline
            if now >= horizon and (not settle or self.data.empty()):
                break
            try:
                timeout = 0.001 if not self.data.empty() else max(0.001, min(maximum, horizon) - now)
                data = self.data.get(timeout=timeout)
            except queue.Empty:
                break
            if data is None:
                self.eof = True
                break
            if isinstance(data, dict):
                self.events.append(data)
                continue
            text = data if isinstance(data, str) else self.decoder.decode(data)
            self.text += text
            self.stream.feed(text)
            last_output = time.monotonic()
            if len(self.text) > 16 * 1024 * 1024:
                raise AssertionError("terminal transcript exceeded 16 MiB")
        if settle:
            assert time.monotonic() < maximum or time.monotonic() - last_output >= 0.12, \
                "native console output did not settle within the bounded capture window"
        if not WINDOWS and self.alive():
            import termios
            modes = termios.tcgetattr(self.slave)
            if not modes[3] & (termios.ICANON | termios.ECHO):
                self.mode_raw = modes

    def alive(self):
        return self.process.isalive() if WINDOWS else self.process.poll() is None

    def send(self, keys):
        self.events.append({"input": repr(keys)})
        if WINDOWS:
            self.process.write(keys)
        else:
            pending = keys.encode("utf-8")
            while pending:
                pending = pending[os.write(self.master, pending):]
        self.pump(settle=WINDOWS)

    def paste(self, text):
        self.send(ESC + "[200~" + text + ESC + "[201~")

    def resize(self, size):
        if WINDOWS:
            self.process.setwinsize(size[1], size[0])
        else:
            import fcntl
            import struct
            import termios
            fcntl.ioctl(self.slave, termios.TIOCSWINSZ, struct.pack("HHHH", size[1], size[0], 0, 0))
            os.kill(self.process.pid, signal.SIGWINCH)
        if not WINDOWS:
            self.screen.resize(lines=size[1], columns=size[0])
        self.events.append({"resize": list(size)})
        self.pump(settle=WINDOWS)

    def display(self):
        # Pyte's display accessor assumes wide-glyph continuation cells were
        # never overwritten by a resize/diff. Export actual cells robustly.
        from wcwidth import wcswidth
        rows = []
        for y in range(self.screen.lines):
            row, x = [], 0
            while x < self.screen.columns:
                data = self.screen.buffer[y][x].data
                row.append(data or " ")
                x += max(1, wcswidth(data)) if data else 1
            rows.append("".join(row))
        return unicodedata.normalize("NFC", "\n".join(rows))

    def capture(self, name):
        (self.directory / (name + ".txt")).write_text(self.display() + "\n", encoding="utf-8")

    def idle(self):
        self.pump(0.3)
        before = len(self.text)
        self.pump(0.5)
        assert len(self.text) == before, "app emitted output while idle"
        self.events.append({"idle_seconds": 0.5, "output_bytes": 0})

    def wait_exit(self):
        deadline = time.monotonic() + 8
        while self.alive() and time.monotonic() < deadline:
            self.pump(0.1)
        assert not self.alive(), "owned terminal process did not exit"
        self.pump(0.3)
        if WINDOWS:
            evidence_file = self.directory / "native-console.json"
            assert evidence_file.is_file(), "native console restoration evidence missing"
            native = json.loads(evidence_file.read_text(encoding="utf-8"))
            self.status = native["child_status"]
            self.events.append({"console_restoration": native})
            assert native["before"] == native["after"], "Windows console modes not restored"
            assert any(not mode[0] & 0x0007 for mode in native["observed"]), "Windows raw console mode not observed"
            assert native["primary_hidden_during_child"], "native alternate buffer was not observed"
            assert native["primary_before"] == native["primary_after"] == "EEVEE_PRIMARY_SENTINEL", \
                "native primary screen contents were not restored"
        else:
            import termios
            self.status = self.process.wait(timeout=2)
            after = termios.tcgetattr(self.slave)
            native = {"before": repr(self.mode_before), "raw": repr(self.mode_raw), "after": repr(after)}
            self.events.append({"termios_restoration": native})
            assert self.mode_raw is not None, "POSIX raw input mode not observed"
            before_flags, after_flags = list(self.mode_before), list(after)
            # macOS kernel-managed pending input is not a terminal mode owned
            # by the app; all raw/canonical/echo and other attributes must match.
            pending = getattr(termios, "PENDIN", 0)
            before_flags[3] &= ~pending
            after_flags[3] &= ~pending
            assert after_flags == before_flags, "POSIX termios not restored"
        modes = {}
        for sequence in re.finditer(r"\x1b\[\?([0-9;]+)([hl])", self.text):
            for number in sequence.group(1).split(";"):
                modes[int(number)] = sequence.group(2) == "h"
        self.events.append({"status": self.status, "final_vt_modes": modes})
        assert modes.get(25) is True, "cursor was not shown on exit"
        if not WINDOWS:
            assert modes.get(1049) is False, "alternate screen was not left"
        assert modes.get(2004) is False, "bracketed paste was not disabled"
        self.events.append({"status": self.status, "restoration": native, "final_vt_modes": modes})
        return self.status

    def close(self):
        if self.closed:
            return
        self.closed = True
        try:
            if WINDOWS:
                if self.job:
                    self.job.close()
                self.process.close(force=True)
                # PtyProcess.isalive() marks itself closed before close(), so
                # explicitly close its reader sockets even after normal exit.
                self.process.fileobj.close()
                self.process._server.close()
            else:
                if self.alive():
                    os.killpg(self.process.pid, signal.SIGKILL)
                self.process.wait(timeout=3)
        finally:
            if not WINDOWS:
                os.close(self.slave)
                os.close(self.master)
            self.reader.join(timeout=2)
            with (self.directory / "terminal.ansi").open("w", encoding="utf-8", newline="") as transcript:
                transcript.write(self.text)
            save_json(self.directory / "events.json", self.events)


def terminal_environment(mode):
    environment = os.environ.copy()
    for key in ("NO_COLOR", "TERM_PROGRAM", "TERM_PROGRAM_VERSION", "KITTY_WINDOW_ID", "TMUX", "STY"):
        environment.pop(key, None)
    environment.update(TERM="xterm-256color", COLORTERM="truecolor", LC_ALL="en_US.UTF-8",
                       LC_CTYPE="en_US.UTF-8", LANG="en_US.UTF-8", PYTHONUTF8="1")
    if WINDOWS:
        # PtyProcess's `backend or env` expression treats integer zero as unset.
        # Supplying the string in env forces its documented ConPTY=0 backend.
        environment["PYWINPTY_BACKEND"] = "0"
    if mode == "no-color":
        environment["NO_COLOR"] = "1"
    return environment


def exercise_ui(session, size, cwd):
    session.pump(0.8)
    assert session.alive(), "app failed at startup"
    assert ESC + "]0;SMOKE_CWD" not in session.text, "cwd controls escaped sanitization"
    session.capture("initial")
    session.idle()
    session.send("g")
    session.paste("🐾 smoke_goal 漢字 cafe\u0301")
    assert "smoke_goal 漢字 café" in session.display(), "CJK/combining paste not visible"
    assert "🐾" in session.display(), "astral paste lost its UTF-16 surrogate pair"
    session.send("\x08")
    assert "café" not in session.display(), "backspace split a combining grapheme"
    session.paste("e\u0301 qf?\necho unsafe > SMOKE_SIDE_EFFECT\n" + ESC + "]0;SMOKE_TITLE\x07")
    session.capture("editor-paste")
    assert session.alive(), "paste executed a shortcut or quit"
    assert not (cwd / "SMOKE_SIDE_EFFECT").exists(), "pasted command was executed"
    assert ESC + "]0;SMOKE_TITLE" not in session.text, "pasted OSC escaped sanitization"
    session.resize((39, 10))
    session.capture("below-minimum-draft")
    session.send(ESC)
    session.capture("below-minimum-discard-neutral")
    session.send(ESC)  # Keep the draft; cancel remains available below minimum.
    session.send("\x03")
    session.capture("below-minimum-quit-neutral")
    session.send("\r")  # Neutral Stay must preserve the dirty editor.
    assert session.alive(), "below-minimum neutral confirmation exited"
    session.resize(size)
    assert "smoke_goal 漢字 café" in session.display(), "resize lost the editor draft"
    session.send("\r")
    assert "smoke_goal 漢字 café" in session.display(), "local apply lost the goal"
    session.capture("applied-goal")
    for key, name in (("?", "help"), ("i", "inspector"), ("b", "companion"),
                      ("p", "preview"), ("\x0b", "palette")):
        before = session.display()
        session.send(key)
        assert session.display() != before, name + " did not open"
        session.capture(name)
        session.send(ESC)
        session.capture(name + "-back")
        assert session.display() == before, name + " back did not restore the invoking surface"
    session.send("f")
    for index, form in enumerate(FORMS):
        session.send(str(index + 1))
        assert form in session.display(), form + " is unreachable in picker"
        session.capture("form-" + str(index + 1))
    session.send(ESC)
    session.send("f")
    session.send("9")
    session.send("\r")
    assert "smoke_goal" in session.display(), "form selection lost the goal"
    session.send("g")
    session.paste(" changed")
    session.send(ESC)
    session.capture("dirty-discard-neutral")
    session.send("\r")  # Neutral Keep must not destroy the draft.
    assert "changed" in session.display(), "neutral confirmation discarded the draft"
    session.send(ESC)
    session.send(RIGHT)
    session.send("\r")
    assert "changed" not in session.display(), "explicit discard did not restore the goal"
    session.send("g")
    session.send(ESC)  # Unchanged drafts close without a destructive confirmation.
    session.idle()
    session.send("\x03")
    session.capture("quit-neutral")
    session.send("\r")
    assert session.alive(), "neutral quit confirmation exited"
    session.send("\x03")
    session.send(RIGHT)
    session.send("\r")
    assert session.wait_exit() == 0, "normal terminal exit failed"


def non_tty(binary, output, environment, cwd):
    cases = (("help", ["--help"], 0), ("version", ["--version"], 0),
             ("invalid-option", ["--not-a-real-option"], 2),
             ("invalid-mode", ["--images", "unsupported"], 2),
             ("not-interactive", ["--images", "off"], 1))
    results = []
    for name, arguments, expected in cases:
        process = run_owned([str(binary), *arguments], cwd=cwd, env=environment, timeout=10)
        (output / (name + ".txt")).write_bytes(process.stdout + process.stderr)
        assert process.returncode == expected, name + " returned an unexpected status"
        assert b"\x1b" not in process.stdout + process.stderr, name + " emitted terminal controls"
        if name == "help":
            assert b"Usage:" in process.stdout and b"--images" in process.stdout
        if name == "not-interactive":
            assert b"interactive" in process.stderr
        results.append({"case": name, "status": process.returncode})
    return results


def lifecycle_binary(root, output):
    """Build once in a disposable package; do not add examples to the real repo."""
    with tempfile.TemporaryDirectory(prefix="lifecycle-", dir=output) as temporary:
        package = Path(temporary)
        (package / "examples").mkdir()
        (package / "src").mkdir()
        for name in ("Cargo.toml", "Cargo.lock"):
            shutil.copyfile(root / name, package / name)
        shutil.copyfile(root / "src/terminal.rs", package / "src/terminal.rs")
        shutil.copyfile(root / "src/input.rs", package / "src/input.rs")
        (package / "examples/terminal_probe.rs").write_text('''#[path = "../src/terminal.rs"]
mod terminal;
#[path = "../src/input.rs"]
mod input;
fn main() -> std::io::Result<()> {
    let mode = std::env::args().nth(1).unwrap();
    terminal::run(|| Ok(()), |_| {
        println!("EEVEE_LIFECYCLE_ENTERED");
        // Make raw input observable before either failure path unwinds.
        std::thread::sleep(std::time::Duration::from_millis(600));
        match mode.as_str() {
            "error" => Err(std::io::Error::other("intentional lifecycle error")),
            "panic" => panic!("intentional lifecycle panic"),
            _ => Ok(()),
        }
    })
}
''', encoding="utf-8")
        # Keep source/package isolation, but reuse already-built dependency
        # artifacts. A separate target needlessly recompiles the same graph.
        target = root / "target"
        result = run_owned(["cargo", "build", "--locked", "--example", "terminal_probe",
                            "--manifest-path", str(package / "Cargo.toml"),
                            "--target-dir", str(target)], timeout=900)
        (output / "lifecycle-build.log").write_bytes(result.stdout + result.stderr)
        assert result.returncode == 0, "isolated lifecycle example build failed; see lifecycle-build.log"
        return target / "debug/examples" / ("terminal_probe.exe" if WINDOWS else "terminal_probe")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/release/eevee", help="already-built native binary")
    parser.add_argument("--output", default="target/native-smoke", help="local evidence directory")
    parser.add_argument("--record-host", action="store_true", help="record versions/host only; do not run checks")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    host = host_evidence()
    save_json(output / "host.json", host)
    assert not os.environ.get("NATIVE_EXPECT_OS") or host["os"] == os.environ["NATIVE_EXPECT_OS"], "wrong native OS"
    assert not os.environ.get("NATIVE_EXPECT_ARCH") or host["arch"] == os.environ["NATIVE_EXPECT_ARCH"], "wrong native architecture"
    if args.record_host:
        return 0
    binary = Path(args.binary).resolve()
    if WINDOWS and not binary.exists():
        binary = binary.with_suffix(".exe")
    results = {"host": host, "binary": str(binary), "cases": [], "status": "failed"}
    save_json(output / "results.json", results)
    try:
        assert binary.is_file(), "build the native release binary before running terminal smoke"
        results["binary_sha256"] = hashlib.sha256(binary.read_bytes()).hexdigest()
        with tempfile.TemporaryDirectory(prefix="session-", dir=output) as temporary:
            name = "isolated-漢字" if WINDOWS else "isolated-" + ESC + "]0;SMOKE_CWD\x07-漢字"
            cwd = Path(temporary) / name
            cwd.mkdir()
            assets = Path(temporary) / "absent-artwork"
            results["non_tty"] = non_tty(binary, output, terminal_environment("dark"), cwd)
            modes = {
                "dark": ["--theme", "dark"], "light": ["--theme", "light"],
                "indexed": ["--color", "256"], "mono-ascii": ["--color", "mono", "--ascii"],
                "linear": ["--layout", "linear", "--color", "mono"],
                "missing-cells": ["--images", "cells"], "no-color": [],
            }
            sizes = ((120, 36), (100, 30), (70, 24), (60, 18))
            for mode, flags in modes.items():
                for size in sizes:
                    name = mode + "-" + str(size[0]) + "x" + str(size[1])
                    case = {"name": name, "size": list(size), "status": "failed"}
                    results["cases"].append(case)
                    session = Session([str(binary), "--images", "off", "--assets-dir", str(assets), *flags],
                                      cwd, terminal_environment(mode), size, output / name)
                    try:
                        exercise_ui(session, size, cwd)
                        case["status"] = "passed"
                    finally:
                        session.close()
                        save_json(output / "results.json", results)
            for name, size, key in (("below-minimum-quit", (39, 10), "q"),
                                    ("clean-quit", (60, 18), "q"),
                                    ("clean-ctrl-c", (60, 18), "\x03")):
                session = Session([str(binary), "--images", "off", "--color", "mono", "--ascii"],
                                  cwd, terminal_environment("mono-ascii"), size, output / name)
                try:
                    session.pump(0.8)
                    session.capture("initial")
                    session.send(key)
                    assert session.wait_exit() == 0
                    results["cases"].append({"name": name, "status": "passed"})
                finally:
                    session.close()
            probe = lifecycle_binary(root, output)
            for mode in ("normal", "error", "panic"):
                name = "lifecycle-" + mode
                session = Session([str(probe), mode], cwd, terminal_environment("mono-ascii"),
                                  (70, 24), output / name)
                try:
                    session.pump(0.25)
                    status = session.wait_exit()
                    assert (status == 0) == (mode == "normal"), "incorrect lifecycle exit status"
                    assert "EEVEE_LIFECYCLE_ENTERED" in session.text
                    if mode != "normal":
                        assert "intentional lifecycle " + mode in session.text
                    results["cases"].append({"name": name, "status": "passed", "exit_status": status})
                finally:
                    session.close()
        results["status"] = "passed"
        return 0
    except Exception as error:
        results["error"] = str(error)
        print("native terminal smoke failed: " + str(error), file=sys.stderr)
        return 1
    finally:
        save_json(output / "results.json", results)


if __name__ == "__main__":
    if "--owned-command" in sys.argv:
        if sys.stdin.readline().strip() != "EEVEE_SMOKE_START":
            raise SystemExit(2)
        command = sys.argv[sys.argv.index("--") + 1:]
        raise SystemExit(subprocess.run(command, stdin=subprocess.DEVNULL).returncode)
    if "--console-child" in sys.argv:
        raise SystemExit(console_child(sys.argv[sys.argv.index("--") + 1:]))
    raise SystemExit(main())
