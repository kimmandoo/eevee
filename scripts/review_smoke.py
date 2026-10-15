#!/usr/bin/env python3
"""Opt-in real Codex independent-review acceptance; no simulated provider."""
import argparse
from contextlib import ExitStack, closing
import hashlib
import json
import os
from pathlib import Path
import re
import sqlite3
import subprocess
import tempfile

from terminal_smoke import Session, confirm_exit, terminal_environment
from workbench_smoke import wait_for

ESC, DOWN, RIGHT, PAGE = "\x1b", "\x1b[B", "\x1b[C", "\x1b[6~"
IDLE = "Ready ·"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--live", action="store_true")
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if not args.live:
        parser.error("--live is required: real provider model requests and provider-owned account effects")
    binary, output = args.binary.resolve(), args.output.resolve()
    if os.name == "nt" and not binary.exists():
        binary = binary.with_suffix(".exe")
    output.mkdir(parents=True, exist_ok=False)
    result = {"status": "failed", "cases": [], "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest()}
    sessions = []

    def passed(name):
        result["cases"].append(name)
        print("passed:", name, flush=True)

    def cleanup():
        for session in sessions:
            if not session.closed:
                session.capture("final")
                session.close()

    try:
        with ExitStack() as stack:
            root = Path(stack.enter_context(tempfile.TemporaryDirectory(prefix="ev-review-"))).resolve()
            stack.callback(cleanup)
            project, data = root / "project", root / "data"
            project.mkdir()
            source = project / "calculator.py"
            source.write_text("def add(a, b):\n    return a + b\n", encoding="utf-8")
            for arguments in (("init", "--quiet"), ("add", "--", "calculator.py"),
                              ("-c", "user.name=Smoke", "-c", "user.email=smoke@example.invalid", "commit", "-qm", "review fixture")):
                subprocess.run(["git", "-c", "core.hooksPath=", *arguments], cwd=project, check=True, capture_output=True)
            source.write_text("def add(a, b):\n    return a - b\n\n# Preserve this unrelated user comment.\n", encoding="utf-8")
            (project / "notes.md").write_text("Unrelated user-owned documentation.\n", encoding="utf-8")
            before = {str(path.relative_to(project)): hashlib.sha256(path.read_bytes()).hexdigest()
                      for path in project.rglob("*") if path.is_file()}
            database = data / "workbench.sqlite3"
            session = Session([str(binary), "--data-dir", str(data), "--images", "off", "--color", "mono"],
                              project, terminal_environment("mono"), (120, 36), output / "review")
            sessions.append(session)
            wait_for(session, "Storage: saved")
            session.command("verbose")  # Inspect actual provider identities and complete diagnostics.
            session.send(ESC)
            session.command("goal")
            session.paste("Independently review the working calculator.py patch against integer addition. Identify the concrete regression, its source location, impact and actionable recommendation. Do not edit files or run commands. Missing checks must be inconclusive, not a pass.")
            session.send("\r")
            session.command("criteria")
            session.command("criteria-edit")
            session.paste("add(7, 2) returns 9; unrelated user files and comments remain unchanged")
            session.send("\r")
            session.send(ESC)
            session.command("next")
            canary = "IMPLEMENTATION_TRANSCRIPT_MUST_NOT_ENTER_REVIEW"
            session.paste(canary)
            session.send("\r")
            wait_for(session, "Storage: saved")
            session.command("workflows")
            session.send(RIGHT * 2 + DOWN * 9 + "\r")
            wait_for(session, IDLE)

            def journal():
                with closing(sqlite3.connect(database.as_uri() + "?mode=ro", uri=True)) as db:
                    records = [json.loads(row[0]) for row in db.execute("SELECT record FROM provider_journal ORDER BY id")]
                    local_runs = db.execute("SELECT count(*) FROM executions").fetchone()[0]
                    task = db.execute("SELECT complete,next_action FROM tasks ORDER BY id LIMIT 1").fetchone()
                return records, local_runs, task

            def turn(number):
                session.command("new")
                wait_for(session, "LOCAL PREVIEW", timeout=35)
                wait_for(session, "y Run")
                session.send("d")
                pages = []
                for _ in range(64):
                    pages.append(session.display())
                    assert "Run blocked" not in pages[-1], pages[-1]
                    if "EXACT OUTGOING PAYLOAD END" in pages[-1]:
                        break
                    session.send(PAGE)
                else:
                    raise AssertionError("bounded review payload preview did not reach its end")
                preview = "\n".join(pages)
                assert canary not in preview, "implementation-only next action leaked into independent review"
                assert "calculator.py" in preview and "return a - b" in preview
                (output / ("preview-%d.txt" % number)).write_text(preview, encoding="utf-8")
                session.send("d")
                wait_for(session, "y Run")
                assert journal()[1] == 0
                session.send("y")
                wait_for(session, IDLE, timeout=180)
                session.send("\x1b[1;3H")
                wait_for(session, "Thread:")
                assert re.search(r"Ready · (Response|Plan) \d+/\d+ complete", session.display()), session.display()
                thread = re.search(r"Thread: *([0-9a-f-]{36})", session.display())
                assert thread, "completed review did not expose its actual session identity"
                observation = []
                for _ in range(32):
                    view = session.display()
                    if observation and view == observation[-1]:
                        break
                    observation.append(view)
                    session.send(PAGE)
                session.capture("completed-review-%d" % number)
                assert journal()[1] == 0 and not journal()[2][0]
                after = {str(path.relative_to(project)): hashlib.sha256(path.read_bytes()).hexdigest()
                         for path in project.rglob("*") if path.is_file()}
                assert after == before, "managed review changed source or repository metadata"
                return {"thread_id": thread.group(1), "visible_response": "\n".join(observation)}

            first = turn(1)
            passed("Actual independent Codex review omitted implementation-only context and preserved all project bytes")
            before_resume = journal()[0]
            session.command("resume")
            wait_for(session, "Review requires a fresh session")
            assert journal()[0] == before_resume, "Resume created or replayed review observations"
            second = turn(2)
            assert first["thread_id"] != second["thread_id"]
            passed("Resume was refused and separate approved reviews used distinct real provider sessions")
            result["observations"] = [first, second]
            result["project_before_sha256"] = before
            result["project_after_sha256"] = {str(path.relative_to(project)): hashlib.sha256(path.read_bytes()).hexdigest()
                                               for path in project.rglob("*") if path.is_file()}
            session.send("\x03")
            confirm_exit(session)
            assert session.wait_exit() == 0
            session.close()
        result["status"] = "passed"
    except BaseException as error:
        result["error"] = str(error)
        raise
    finally:
        (output / "results.json").write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
