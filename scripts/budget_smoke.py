#!/usr/bin/env python3
"""Real-TUI token-economy acceptance (T01 local receipt, T05 budgets); offline.

Drives an already-built native ev in a disposable git project with provider
credentials removed. Checks a repeated unchanged command pause, continue-once,
the per-task command budget after a source edit, and the provider preview
receipt/budget pause. No provider turn is sent and nothing contacts a network.
"""
import argparse
from contextlib import closing
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import time

from terminal_smoke import Session, terminal_environment
from workbench_smoke import wait_for

ESC = "\x1b"
HOME = ESC + "[H"
RIGHT = ESC + "[C"
DELETE = ESC + "[3~"
REVIEW = "Review one-shot execution permission"
PAUSE = "Budget pause before another command"
IDLE = "Local command evidence; no command running"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/release/ev")
    parser.add_argument("--output", default="target/budget-smoke")
    args = parser.parse_args()
    binary, output = Path(args.binary).resolve(), Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    results = {"status": "failed", "cases": []}

    def passed(case):
        results["cases"].append(case)
        print("passed:", case, flush=True)

    with tempfile.TemporaryDirectory(prefix="ev-budget-") as temp:
        root = Path(temp)
        project, data = root / "project", root / "data"
        project.mkdir()
        source = project / "source.txt"
        source.write_text("stable source\n", encoding="utf-8")
        for arguments in (("init", "--quiet"), ("add", "--", "source.txt"),
                          ("-c", "user.name=Eevee smoke", "-c", "user.email=smoke@example.invalid",
                           "commit", "-qm", "budget fixture")):
            subprocess.run(["git", "-c", "core.hooksPath=", *arguments], cwd=project, check=True,
                           capture_output=True, timeout=10)
        environment = terminal_environment("mono")
        for key in list(environment):
            if key.startswith(("OPENAI_", "CODEX_", "ANTHROPIC_", "CLAUDE_", "GEMINI_", "GOOGLE_", "AWS_")):
                environment.pop(key)
        def launch(name):
            return Session([str(binary), "--data-dir", str(data), "--images", "off", "--color", "mono",
                            "--max-commands", "2", "--max-input-tokens", "5"],
                           project, environment, (120, 40), output / name)

        session = launch("journey")

        def runs():
            with closing(sqlite3.connect((data / "workbench.sqlite3").as_uri() + "?mode=ro", uri=True, timeout=1)) as db:
                return [json.loads(row[0]) for row in db.execute("SELECT record FROM executions ORDER BY rowid")]

        def until(predicate, explanation, timeout=35):
            deadline = time.monotonic() + timeout
            while not predicate():
                assert session.alive(), "application exited: " + explanation
                assert time.monotonic() < deadline, explanation + "\n" + session.display()
                session.pump(0.05)

        def finished(count):
            until(lambda: len(runs()) == count and runs()[-1]["state"] == "Succeeded", "run %d not journaled" % count)
            wait_for(session, IDLE, timeout=35)

        def prepare(value=None):
            session.send("e")
            wait_for(session, "Command / explicit JSON argv or shell")
            if value is not None:
                session.paste(value)
            session.send("\r")
            wait_for(session, "Approval pending", timeout=35)

        try:
            wait_for(session, "Storage: saved")
            session.send(ESC)
            session.send("g")
            session.paste("Budget smoke task")
            session.send("\r")
            wait_for(session, "Storage: saved")
            session.send("j")
            wait_for(session, "local command evidence")
            prepare(json.dumps([sys.executable, "-c", "print('budget-ok')"]))
            session.send("v")
            wait_for(session, REVIEW)
            assert PAUSE not in session.display()
            session.send(RIGHT + "\r")
            finished(1)
            passed("first command needs only the normal one-shot approval")

            prepare()
            session.send("v")
            wait_for(session, PAUSE)
            wait_for(session, "Repeated unchanged command")
            session.send("\r")  # Stay is the default.
            session.pump(1)
            assert len(runs()) == 1, "Stay executed the paused command"
            session.send("v")
            wait_for(session, PAUSE)
            session.send(RIGHT + "\r")
            wait_for(session, REVIEW)
            session.send(RIGHT + "\r")
            finished(2)
            passed("unchanged repeat pauses; Stay runs nothing; continue once then approval runs it")

            source.write_text("changed source\n", encoding="utf-8")
            prepare()
            session.send("v")
            wait_for(session, PAUSE)
            wait_for(session, "Command budget reached: 2 recorded runs, limit 2")
            assert "Repeated unchanged command" not in session.display(), "edited source treated as a loop"
            session.send("x")
            wait_for(session, "Request denied")
            assert len(runs()) == 2, "denied paused request executed"
            passed("changed source is not a loop, but the committed per-task command budget still pauses; x denies")

            session.send(ESC)
            session.send(ESC)
            session.send("q")
            wait_for(session, "Confirm exit")
            session.send(RIGHT + "\r")
            assert session.wait_exit() == 0
            session.close()
            session = launch("restart")
            wait_for(session, "Storage: saved")
            session.send("j")
            wait_for(session, "local command evidence")
            prepare()
            session.send("v")
            wait_for(session, "Command budget reached: 2 recorded runs, limit 2")
            session.send("x")
            wait_for(session, "Request denied")
            assert len(runs()) == 2
            passed("restart reloads committed runs; the command budget cannot be reset by quitting")

            session.send(ESC)
            session.send(ESC)
            session.send("e")
            wait_for(session, "explicit New or same-task Resume")
            session.send("n")
            wait_for(session, "Local preview only", timeout=40)
            for _ in range(60):
                if "Total sent by Eevee" in session.display() and "PAUSED" in session.display():
                    break
                session.send(ESC + "[6~")  # PgDn through the exact review.
                session.pump(0.1)
            display = session.display()
            for text in ("Total sent by Eevee", "provider turns journaled", "PAUSED"):
                assert text in display, text + "\n" + display
            session.capture("provider-receipt")
            session.send(ESC + "[F")
            wait_for(session, "Run")
            session.send(RIGHT + "\r")
            wait_for(session, "Request estimate")
            session.pump(1)
            with closing(sqlite3.connect((data / "workbench.sqlite3").as_uri() + "?mode=ro", uri=True)) as db:
                assert db.execute("SELECT count(*) FROM provider_journal").fetchone()[0] == 0
            passed("provider preview shows a local receipt and journaled turns; Run is held by the estimate-limit pause")

            with closing(sqlite3.connect((data / "workbench.sqlite3").as_uri() + "?mode=ro", uri=True)) as db:
                assert db.execute("SELECT count(*) FROM provider_journal").fetchone()[0] == 0
            results["status"] = "passed-offline"
        finally:
            session.close()
            (output / "results.json").write_text(json.dumps(results, indent=2) + "\n")


if __name__ == "__main__":
    main()
