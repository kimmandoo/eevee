#!/usr/bin/env python3
"""Actual PTY/ConPTY local-command acceptance; requires an already-built native ev.

Runs only disposable Python fixtures outside the checkout. Exercises real owned
process trees, not a mocked runner; unsupported native behavior fails, never skips.
Artifacts include terminal transcripts, committed journal records, and results.json.
"""
import argparse
from contextlib import ExitStack, closing
import json
import os
from pathlib import Path
import platform
import re
import sqlite3
import subprocess
import sys
import tempfile
import time

from terminal_smoke import Session, confirm_exit, terminal_environment
from workbench_smoke import wait_for


def lead_drafts(task):
    return task["partner_drafts"].get(str(task["lead_partner"]), {}).get("drafts", [])


ESC = "\x1b"
HOME = ESC + "[H"
END = ESC + "[F"
DOWN = ESC + "[B"
RIGHT = ESC + "[C"
DELETE = ESC + "[3~"
TITLE = "local command evidence"
REVIEW = "Review one-shot execution permission"
GUARD = "Stay with owned work, or cancel and wait"
IDLE = "Local command evidence; no command running"

# This file is written into the temporary fixture directory, never the checkout.
FIXTURE = r'''import json
import os
from pathlib import Path
import subprocess
import sys
import time

mode = sys.argv[1]
location = Path(sys.argv[2])
if mode in ("once", "stream"):
    # Exclusive creation detects an accidental replay even after application restart.
    with location.open("x", encoding="utf-8") as handle:
        json.dump({"argv": sys.argv[3:], "cwd": os.getcwd()}, handle, ensure_ascii=False)
    print("OUTPUT-BEGIN 東京", flush=True)
    if mode == "stream":
        for number in range(80):
            print(f"line-{number:03d}" + (" needle" if number in (5, 45) else ""), flush=True)
            time.sleep(0.01)
        print("STREAM-READY", flush=True)
        deadline = time.monotonic() + 90
        while not location.with_suffix(".release").exists():
            if location.with_suffix(".stop").exists() or time.monotonic() > deadline:
                sys.exit(91)
            time.sleep(0.03)
        for number in range(80, 100):
            print(f"line-{number:03d}", flush=True)
    print("OUTPUT-END 東京", flush=True)
elif mode == "tree":
    role = int(sys.argv[3])
    pid = location / f"{role}.pid"
    temporary = pid.with_suffix(".tmp")
    temporary.write_text(str(os.getpid()), encoding="ascii")
    temporary.replace(pid)
    child = None
    if role < 2:
        child = subprocess.Popen([sys.executable, "-X", "utf8", "-u", str(Path(__file__).resolve()),
                                  "tree", str(location), str(role + 1)], stdin=subprocess.DEVNULL)
    print(f"TREE-READY-{role}", flush=True)
    deadline = time.monotonic() + 90
    tick = 0
    while not (location / "stop").exists() and time.monotonic() < deadline:
        (location / f"{role}.heartbeat").write_text(str(tick), encoding="ascii")
        tick += 1
        time.sleep(0.05)
    if child is not None:
        child.wait(timeout=5)
else:
    raise RuntimeError("unknown fixture mode")
'''


class NativeProcess:
    """Observe exit without signalling or mistaking POSIX zombies for live work."""
    def __init__(self, pid):
        self.pid = pid
        self.handle = None
        if os.name == "nt":
            import ctypes
            from ctypes import wintypes
            self.kernel = ctypes.WinDLL("kernel32", use_last_error=True)
            self.kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
            self.kernel.OpenProcess.restype = wintypes.HANDLE
            self.kernel.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
            self.kernel.WaitForSingleObject.restype = wintypes.DWORD
            self.kernel.CloseHandle.argtypes = [wintypes.HANDLE]
            self.kernel.CloseHandle.restype = wintypes.BOOL
            self.handle = self.kernel.OpenProcess(0x00100000, False, pid)  # SYNCHRONIZE
            if not self.handle:
                raise ctypes.WinError(ctypes.get_last_error())

    def alive(self):
        if os.name == "nt":
            state = self.kernel.WaitForSingleObject(self.handle, 0)
            assert state in (0, 258), "native process wait failed: " + str(state)
            return state == 258
        process = subprocess.run(["ps", "-p", str(self.pid), "-o", "stat="],
                                 capture_output=True, text=True, timeout=5)
        assert process.returncode in (0, 1), process.stderr
        status = process.stdout.strip()
        return bool(status) and not status.startswith("Z")

    def close(self):
        if self.handle:
            self.kernel.CloseHandle(self.handle)
            self.handle = None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/release/ev")
    parser.add_argument("--output", default="target/execution-smoke")
    args = parser.parse_args()
    binary = Path(args.binary).resolve()
    if os.name == "nt" and not binary.exists():
        binary = binary.with_suffix(".exe")
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    python = Path(sys.executable).resolve()
    result = {"status": "failed", "cases": [], "host": platform.platform(),
              "python": str(python), "terminal": "ConPTY" if os.name == "nt" else "PTY",
              "scope": "Native owned local commands only; no provider, network, or sandbox claim"}
    sessions, processes, stops = [], [], []

    def passed(name):
        result["cases"].append(name)
        print("passed:", name, flush=True)

    def cleanup():
        # Failure cleanup is not evidence of cancellation: assertions run first.
        # Cooperative stop also reaches fixture children outside the terminal group.
        for stop in stops:
            stop.touch()
        for session in sessions:
            if not session.closed:
                session.capture("final")
                session.close()
        deadline = time.monotonic() + 6
        while any(process.alive() for process in processes) and time.monotonic() < deadline:
            time.sleep(0.05)
        for process in processes:
            process.close()

    try:
        assert python.is_absolute() and python.is_file(), "native Python executable unavailable"
        with ExitStack() as stack:
            # Do not put fixtures under --output: target/ discovers the enclosing Git root.
            root = Path(stack.enter_context(tempfile.TemporaryDirectory(prefix="ev-execution-空 格-"))).resolve()
            stack.callback(cleanup)
            project = root / "project 漢字"
            project.mkdir()
            data = root / "data 空 格"
            database = data / "workbench.sqlite3"
            fixture = root / "command fixture 漢字.py"
            fixture.write_text(FIXTURE, encoding="utf-8")
            source = project / "source.txt"
            source.write_text("stable source identity\n", encoding="utf-8")
            for arguments in (("init", "--quiet"), ("add", "--", "source.txt"),
                              ("-c", "user.name=Eevee smoke", "-c", "user.email=smoke@example.invalid",
                               "commit", "-qm", "execution fixture")):
                subprocess.run(["git", "-c", "core.autocrlf=false", "-c", "core.hooksPath=", *arguments],
                               cwd=project, check=True, capture_output=True, timeout=10)

            def launch(name):
                session = Session([str(binary), "--data-dir", str(data), "--images", "off", "--color", "mono"],
                                  project, terminal_environment("mono"), (120, 40), output / name)
                sessions.append(session)
                session.pump(0.5)
                wait_for(session, "Storage: saved")
                return session

            def rows():
                with closing(sqlite3.connect(database.as_uri() + "?mode=ro", uri=True, timeout=1)) as db:
                    db.row_factory = sqlite3.Row
                    tasks = [dict(row) for row in db.execute("SELECT id,goal,command,lead_partner,partner_drafts,complete,form FROM tasks ORDER BY id")]
                    runs = [dict(row) for row in db.execute("SELECT record,output_name,reserved FROM executions ORDER BY rowid")]
                    active = db.execute("SELECT active_id FROM selections").fetchone()[0]
                for task in tasks:
                    task["partner_drafts"] = json.loads(task["partner_drafts"])
                for run in runs:
                    run["record"] = json.loads(run["record"])
                return {"tasks": tasks, "runs": runs, "active": active}

            def until(session, predicate, explanation, timeout=35):
                deadline = time.monotonic() + timeout
                while not predicate():
                    assert session.alive(), "application exited: " + explanation
                    if time.monotonic() >= deadline:
                        states = [{
                            "state": run["record"]["state"], "tree": run["record"]["tree"],
                            "cancelled": run["record"]["cancelled"], "reserved": run["reserved"],
                        } for run in rows()["runs"]]
                        raise AssertionError(explanation + "\n" + json.dumps(states) + "\n" + session.display())
                    session.pump(0.05)

            def task(task_id):
                return next(row for row in rows()["tasks"] if row["id"] == task_id)

            def task_menu(session, index):
                # The palette works from both the workspace and execution overlay.
                session.command("tasks")
                wait_for(session, "Task cards / committed snapshots")
                wait_for(session, "Storage: saved")
                session.send(HOME + DOWN * index)
                wait_for(session, "Storage: saved")
                session.send("\r")

            def switch(session, index, task_id):
                task_menu(session, index)
                until(session, lambda: rows()["active"] == task_id, "task switch did not commit")
                until(session, lambda: "Task cards / committed snapshots" not in session.display(),
                      "task switch committed before its UI acknowledgement")
                session.command("goal")
                wait_for(session, task(task_id)["goal"])
                session.send("\x13")
                wait_for(session, "Storage: saved")

            def open_execution(session):
                session.command("jolteon")  # Jolteon owns command preparation.
                session.command("run")
                wait_for(session, TITLE)

            def command(mode, location, *arguments):
                # ASCII JSON makes editor replacement exact; decoded CJK/quotes remain literal argv.
                return json.dumps([str(python), "-X", "utf8", "-u", str(fixture), mode, str(location), *arguments])

            def edit_command(session, task_id, value, draft=False):
                previous = next((entry["text"] for entry in lead_drafts(task(task_id)) if entry["field"] == "Command"),
                                task(task_id)["command"])
                session.command("command-edit")
                wait_for(session, "Command / explicit JSON argv or shell")
                session.send(HOME)
                # Drive acknowledged editing, rather than overflowing the native
                # terminal's finite typeahead queue with hundreds of CSI events.
                for offset in range(0, len(previous), 32):
                    removed = min(32, len(previous) - offset)
                    session.send(DELETE * removed)
                    wait_for(session, str(len(previous) - offset - removed) + " / 4096 bytes")
                session.paste(value)
                session.send("\x13" if draft else "\r")
                if draft:
                    until(session, lambda: any(entry["field"] == "Command" and entry["text"] == value
                                              for entry in lead_drafts(task(task_id))), "command draft was not saved")
                else:
                    until(session, lambda: task(task_id)["command"] == value, "command was not saved before approval")
                    wait_for(session, "Approval pending", timeout=35)

            def approve(session):
                session.command("approvals")
                wait_for(session, REVIEW)
                session.send(RIGHT + "\r")

            def final_run(session, count, state="Succeeded", idle=True):
                until(session, lambda: len(rows()["runs"]) == count
                      and rows()["runs"][-1]["record"]["state"] == state
                      and not rows()["runs"][-1]["reserved"], "final execution journal was not committed")
                if idle:
                    wait_for(session, IDLE, timeout=35)
                run = rows()["runs"][-1]
                assert run["record"]["tree"] == "OwnedTreeExited", run
                assert run["record"]["ended_at_ms"] is not None, run
                return run

            def quit_idle(session):
                session.command("quit")
                confirm_exit(session)
                assert session.wait_exit() == 0
                session.close()

            first = launch("approval-output")
            first.send(ESC)  # Enter navigation from the clean first-run composer.
            first.command("goal")
            first.paste("execution task A")
            first.send("\r")
            until(first, lambda: rows()["tasks"][0]["goal"] == "execution task A", "first task save")
            task_a = rows()["active"]
            first.command("task-new")
            first.command("goal")
            first.paste("execution task B")
            first.send("\r")
            until(first, lambda: len(rows()["tasks"]) == 2 and rows()["tasks"][1]["goal"] == "execution task B", "second task save")
            task_b = rows()["active"]
            switch(first, 0, task_a)
            first.command("goal")
            first.send(END)
            first.paste(" retained draft 漢字")
            first.send("\x13")
            retained_goal = "execution task A retained draft 漢字"
            until(first, lambda: any(entry["text"] == retained_goal for entry in lead_drafts(task(task_a))), "goal draft save")
            open_execution(first)
            denied = root / "denied.json"
            denied_command = command("once", denied)
            edit_command(first, task_a, denied_command, draft=True)
            assert task(task_a)["command"] == "" and not rows()["runs"] and not denied.exists()
            first.command("command-edit")
            first.send("\r")
            wait_for(first, "Approval pending", timeout=35)
            first.command("run")
            wait_for(first, REVIEW)
            first.capture("neutral-approval")
            first.send("\r")
            first.pump(0.4)
            assert not denied.exists() and not rows()["runs"], "neutral Enter executed a command"
            first.command("approvals")
            first.send(ESC)
            assert not denied.exists() and not rows()["runs"], "Esc executed a command"
            first.command("approvals")
            first.command("deny")
            wait_for(first, "Request denied")
            assert not denied.exists() and not rows()["runs"]
            assert task(task_a)["command"] == denied_command
            passed("command-draft-save-apply-neutral-approval-and-explicit-denial-no-execution")

            literal_marker = root / "literal.json"
            stops.append(literal_marker.with_suffix(".stop"))
            shell_marker = root / "must not be shell executed"
            literal_args = ["東京 空 格", "", "a;b|c&d", "$(touch '" + str(shell_marker) + "')",
                            "`echo unexpected`", "*?[]", 'quote"single\'end', "backslash\\tail\\", "%PATH%", "$HOME"]
            literal_command = command("stream", literal_marker, *literal_args)
            edit_command(first, task_a, literal_command)
            approve(first)
            wait_for(first, "STREAM-READY", timeout=35)
            marker_before = literal_marker.read_bytes()
            observed = json.loads(marker_before)
            assert observed["argv"] == literal_args, observed
            assert Path(observed["cwd"]).samefile(project), observed
            assert not shell_marker.exists() and not denied.exists()
            first.send(HOME)
            wait_for(first, "Follow paused")
            first.command("search")
            first.send("needle")
            search_surface = first.display()
            first.send("\x0b")
            assert first.display() == search_surface, "global palette escaped the output search field"
            first.send("\r")
            wait_for(first, "line-005 needle")
            assert "line-045 needle" not in first.display()
            first.command("search-next")
            wait_for(first, "line-045 needle")
            first.command("search-previous")
            wait_for(first, "line-005 needle")
            literal_marker.with_suffix(".release").touch()
            until(first, lambda: re.search(r"Follow paused\s*·\s*[1-9][0-9]* new lines", first.display()),
                  "paused output did not report newly arriving lines")
            assert "line-005 needle" in first.display(), "new output moved the paused search anchor"
            first.capture("paused-search-new-output")
            first.command("follow")
            wait_for(first, "Follow ON")
            wait_for(first, "OUTPUT-END 東京")
            literal_run = final_run(first, 1)
            record = literal_run["record"]
            assert record["prepared"]["spec"]["Direct"]["args"] == json.loads(literal_command)[1:]
            assert not record["truncated"] and not record["output_unavailable"] and record["exit_code"] == 0
            assert record["prepared"]["before"]["complete"] and record["after"]["complete"]
            assert record["prepared"]["before"]["fingerprint"] == record["after"]["fingerprint"]
            assert not task(task_a)["complete"], "successful execution marked user criteria complete"
            passed("direct-literal-cjk-metachar-argv-search-next-previous-pause-follow-and-real-output")

            switch(first, 1, task_b)
            open_execution(first)
            other_marker = root / "other.json"
            edit_command(first, task_b, command("once", other_marker, "independent task evidence"))
            approve(first)
            other_run = final_run(first, 2)
            other_spool = data / other_run["output_name"]
            other_bytes = other_spool.read_bytes()
            switch(first, 0, task_a)
            quit_idle(first)
            resumed = launch("restart-purge-missing")
            assert rows()["active"] == task_a
            assert literal_marker.read_bytes() == marker_before and len(rows()["runs"]) == 2
            open_execution(resumed)
            wait_for(resumed, IDLE)
            resumed.command("history")
            wait_for(resumed, "Succeeded")
            resumed.send("\r")
            wait_for(resumed, "Retained bytes")
            wait_for(resumed, "OUTPUT-BEGIN 東京")
            resumed.send(END)
            wait_for(resumed, "OUTPUT-END 東京")
            resumed.capture("retained-output-after-restart")
            assert literal_marker.read_bytes() == marker_before and len(rows()["runs"]) == 2
            passed("restart-loads-durable-output-and-source-evidence-without-reexecution")
            own_spool = data / literal_run["output_name"]
            retained_bytes = own_spool.read_bytes()
            resumed.command("purge")
            wait_for(resumed, "Purge retained local output")
            resumed.send("\r")
            assert own_spool.read_bytes() == retained_bytes, "neutral purge removed evidence"
            resumed.command("purge")
            resumed.send(RIGHT + "\r")
            wait_for(resumed, "Output unavailable")
            until(resumed, lambda: rows()["runs"][0]["record"]["output_unavailable"], "purge metadata did not commit")
            assert not own_spool.exists() and other_spool.read_bytes() == other_bytes
            resumed.capture("purged-unavailable")
            passed("neutral-purge-keeps-output-explicit-purge-keeps-metadata-and-other-task-evidence")
            switch(resumed, 1, task_b)
            other_spool.unlink()
            open_execution(resumed)
            resumed.command("history")
            wait_for(resumed, "Output unavailable")
            resumed.send("\r")
            wait_for(resumed, "Output unavailable (purged, missing, or replaced)")
            assert len(rows()["runs"]) == 2 and other_marker.exists()
            resumed.capture("missing-output-unavailable")
            passed("missing-spool-is-unavailable-not-empty-success-or-automatic-retry")
            switch(resumed, 0, task_a)
            open_execution(resumed)

            def start_tree(session, name, count):
                location = root / name
                location.mkdir()
                stops.append(location / "stop")
                value = command("tree", location, "0")
                edit_command(session, task_a, value)
                approve(session)
                until(session, lambda: all((location / (str(role) + ".pid")).exists() for role in range(3)),
                      "owned root/child/grandchild did not all start")
                wait_for(session, "TREE-READY-2")
                owned = [NativeProcess(int((location / (str(role) + ".pid")).read_text(encoding="ascii"))) for role in range(3)]
                processes.extend(owned)
                assert all(process.alive() for process in owned)
                until(session, lambda: len(rows()["runs"]) == count and rows()["runs"][-1]["record"]["state"] == "Running",
                      "running ownership was not journaled")
                return owned, value

            def exited(session, owned):
                until(session, lambda: not any(process.alive() for process in owned), "cancel leaked owned root/child/grandchild", timeout=10)

            owned, _ = start_tree(resumed, "cancel tree", 3)
            form_before = task(task_a)["form"]
            resumed.command("forms")
            resumed.command("role-vaporeon")
            wait_for(resumed, GUARD)
            resumed.send("\r")
            assert task(task_a)["form"] == form_before and all(process.alive() for process in owned)
            resumed.capture("active-form-change-neutral-stay")
            resumed.command("cancel")
            cancelled = final_run(resumed, 3, "Cancelled")
            exited(resumed, owned)
            assert cancelled["record"]["cancelled"] and cancelled["record"]["captured_bytes"] > 0
            resumed.capture("cancelled-owned-tree")
            passed("explicit-lead-cancel-waits-for-owned-child-and-grandchild")

            owned, switched_command = start_tree(resumed, "task switch tree", 4)
            task_menu(resumed, 1)
            wait_for(resumed, GUARD)
            resumed.send("\r")
            assert rows()["active"] == task_a and all(process.alive() for process in owned)
            task_menu(resumed, 1)
            wait_for(resumed, GUARD)
            resumed.send(RIGHT + "\r")
            final_run(resumed, 4, "Cancelled", idle=False)
            exited(resumed, owned)
            until(resumed, lambda: rows()["active"] == task_b, "cancel-and-switch did not navigate after final journal")
            # The cancellation guard already hid task cards. Wait for the new
            # task's actual home content, not merely an absent old overlay.
            wait_for(resumed, task(task_b)["goal"])
            assert task(task_a)["command"] == switched_command
            assert any(entry["text"] == retained_goal for entry in lead_drafts(task(task_a)))
            passed("task-switch-neutral-stay-then-cancel-wait-retains-owning-task-drafts")
            switch(resumed, 0, task_a)
            open_execution(resumed)
            owned, quit_command = start_tree(resumed, "quit tree", 5)
            resumed.command("quit")
            wait_for(resumed, "ALL owned work")
            resumed.send("\r")
            assert resumed.alive() and all(process.alive() for process in owned)
            resumed.command("quit")
            wait_for(resumed, "ALL owned work")
            resumed.capture("neutral-cancel-and-exit")
            resumed.send(RIGHT + "\r")
            final_run(resumed, 5, "Cancelled", idle=False)
            exited(resumed, owned)
            confirm_exit(resumed)
            assert resumed.wait_exit() == 0
            resumed.close()
            last = launch("restart-retained-drafts")
            assert rows()["active"] == task_a and task(task_a)["command"] == quit_command
            assert len(rows()["runs"]) == 5
            last.command("goal")
            wait_for(last, "retained draft 漢字")
            last.capture("retained-goal-draft-after-cancel-exit")
            last.send("\x13")
            wait_for(last, "Storage: saved")
            assert task(task_a)["goal"] == "execution task A" and not task(task_a)["complete"]
            assert any(entry["text"] == retained_goal for entry in lead_drafts(task(task_a)))
            quit_idle(last)
            passed("quit-neutral-stay-cancel-and-wait-clean-exit-and-draft-restart")
            assert source.read_text(encoding="utf-8") == "stable source identity\n"
            assert not denied.exists() and not shell_marker.exists()
            assert literal_marker.read_bytes() == marker_before
            (output / "committed-evidence.json").write_text(json.dumps(rows(), indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
            result["run_count"] = len(rows()["runs"])
        result["status"] = "passed"
    except Exception as error:
        result["error"] = str(error)
        for session in sessions:
            session.capture("failure")
        raise
    finally:
        (output / "results.json").write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
